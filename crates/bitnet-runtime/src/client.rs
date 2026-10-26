use bitnet_core::*;
use std::{
    path::PathBuf,
    process::{Command as Process, Stdio},
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct Client {
    pub scope: Scope,
}
impl Client {
    pub fn new(scope: Scope) -> Self {
        Self { scope }
    }
    pub fn effective_scope(&self) -> Scope {
        if simulation_only() {
            return self.scope;
        }
        if self.scope == Scope::User && bitnet_platform::service::status().installed {
            Scope::System
        } else {
            self.scope
        }
    }
    pub async fn command(&self, mut command: Command) -> Result<ReplyData, AppError> {
        if simulation_only() {
            if self.scope != Scope::User || std::env::var_os("BITNET_DATA_DIR").is_none() {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "模拟专用客户端必须使用独立用户数据目录",
                ));
            }
            let result = super::ipc::request(Scope::User, Command::Status).await?;
            let snapshot = result.snapshot()?;
            if !snapshot.capabilities.iter().any(|c| c == "simulation") {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "模拟专用客户端拒绝连接真实认证引擎",
                ));
            }
            if matches!(command, Command::Status) {
                return Ok(ReplyData::Snapshot(Box::new(snapshot)));
            }
            return super::ipc::request(Scope::User, command).await;
        }
        let effective = self.effective_scope();
        let personal = self.scope == Scope::User && effective == Scope::System;
        let mut requested_preferences = None;
        if personal {
            if let Command::SettingsPut { settings, .. } = &mut command {
                settings.validate()?;
                requested_preferences = Some(UserPreferences {
                    engine: settings.user_autostart,
                    gui: settings.gui_autostart,
                });
                // Shared service settings never represent a particular user's login items.
                let current = super::ipc::request(Scope::System, Command::Status)
                    .await?
                    .snapshot()?;
                settings.user_autostart = current.settings.user_autostart;
                settings.gui_autostart = current.settings.gui_autostart;
            }
        }
        let mut result = super::ipc::request(effective, command).await?;
        if let Some(preferences) = requested_preferences {
            let old = read_preferences();
            if preferences.engine != old.engine {
                super::maintenance::execute(
                    Scope::User,
                    super::maintenance::MaintenanceAction::SetAutostart {
                        enabled: preferences.engine,
                    },
                    false,
                )
                .await
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::Unavailable,
                        "共享设置已保存，但个人后台启动项更新失败；请刷新并修复启动项",
                    )
                })?;
            }
            if preferences.gui != old.gui {
                super::maintenance::execute(
                    Scope::User,
                    super::maintenance::MaintenanceAction::SetGuiAutostart {
                        enabled: preferences.gui,
                    },
                    false,
                )
                .await
                .map_err(|_| {
                    AppError::new(
                        ErrorCode::Unavailable,
                        "共享设置已保存，但个人 GUI 启动项更新失败；请刷新并修复启动项",
                    )
                })?;
            }
            let path = bitnet_platform::paths(Scope::User)?
                .data_dir
                .join("preferences.json");
            super::persistence::atomic_write(
                &path,
                &serde_json::to_vec(&preferences)
                    .map_err(|_| AppError::new(ErrorCode::Internal, "个人设置编码失败"))?,
            )?;
        }
        if personal {
            if let ReplyData::Snapshot(snapshot) = &mut result {
                let preferences = read_preferences();
                snapshot.settings.user_autostart = preferences.engine;
                snapshot.settings.gui_autostart = preferences.gui;
            }
        }
        Ok(result)
    }
    pub async fn status(&self) -> Result<Snapshot, AppError> {
        self.command(Command::Status).await?.snapshot()
    }
    pub async fn ensure_running(&self) -> Result<(), AppError> {
        if self.status().await.is_ok() {
            return Ok(());
        }
        if simulation_only() {
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "模拟引擎未就绪；不会启动真实认证引擎",
            ));
        }
        if super::maintenance::maintenance_in_progress() {
            return Err(AppError::new(
                ErrorCode::Busy,
                "安装维护正在交接认证控制权，请等待完成",
            ));
        }
        if self.effective_scope() == Scope::System {
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "系统后台未就绪；不会启动第二个用户认证引擎，请检查或修复服务",
            ));
        }
        let executable = engine_path()?;
        let mut process = Process::new(executable);
        process
            .arg("--user")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            process.creation_flags(0x08000000 | 0x00000008);
        }
        process
            .spawn()
            .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法启动认证引擎；请修复安装"))?;
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            if self.status().await.is_ok() {
                return Ok(());
            }
        }
        Err(AppError::new(
            ErrorCode::Unavailable,
            "认证引擎未能就绪；请查看诊断日志",
        ))
    }
}
fn simulation_only() -> bool {
    std::env::var("BITNET_SIMULATE_ONLY").as_deref() == Ok("1")
}
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct UserPreferences {
    engine: bool,
    gui: bool,
}
fn read_preferences() -> UserPreferences {
    use std::io::Read;
    let Ok(paths) = bitnet_platform::paths(Scope::User) else {
        return UserPreferences::default();
    };
    if let Ok(file) = std::fs::File::open(paths.data_dir.join("preferences.json")) {
        let mut bytes = Vec::new();
        if file.take(4096).read_to_end(&mut bytes).is_ok() && bytes.len() < 4096 {
            if let Ok(value) = serde_json::from_slice(&bytes) {
                return value;
            }
        }
    }
    super::persistence::read_settings(&paths.config_file)
        .map(|s| UserPreferences {
            engine: s.user_autostart,
            gui: s.gui_autostart,
        })
        .unwrap_or_default()
}
pub fn engine_path() -> Result<PathBuf, AppError> {
    let current = std::env::current_exe()
        .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法定位安装目录"))?;
    Ok(current.with_file_name(if cfg!(windows) {
        "bitengine.exe"
    } else {
        "bitengine"
    }))
}
