use crate::{App, Message, Page, SettingsTab};
use bitnet_core::{AppError, ErrorCode, ReplyData, Scope};
use bitnet_runtime::{
    client::Client,
    maintenance::{self, CheckedUpdate, MaintenanceAction},
    update_policy::{self, UpdateOutcome},
};
use iced::Task;
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

#[derive(Default)]
pub(crate) struct SettingsActions {
    pub update_release: Option<String>,
    pub update_install: Option<UpdateInstall>,
    pub ipv6_editor: Option<Vec<String>>,
    pub export_pending: bool,
    log_refresh_requested: bool,
    update_entry_requested: bool,
    request_serial: u64,
    update_request: Option<u64>,
}

pub(crate) struct UpdateInstall {
    request: u64,
    pub result: Option<Result<Option<CheckedUpdate>, AppError>>,
}

#[derive(Clone, Copy)]
pub(crate) enum CheckTrigger {
    Scheduled,
    OnEntry,
    User,
}

#[derive(Clone, Copy)]
pub(crate) enum ExportTo {
    Clipboard,
    File,
}

#[derive(Clone)]
pub(crate) enum Action {
    OpenIpv6Editor,
    Ipv6TargetChanged(usize, String),
    AddIpv6Target,
    RemoveIpv6Target(usize),
    SaveIpv6Targets,
    CloseIpv6Editor,
    OpenInstaller,
    VerifiedUpdate(u64, Result<Option<CheckedUpdate>, AppError>),
    InstallVerifiedUpdate,
    CloseInstaller,
    Export(ExportTo),
    ExportPrepared(ExportTo, Scope, Result<String, AppError>),
    ExportCopied(bool),
    ExportSaved(Result<bool, ()>),
}

impl SettingsActions {
    fn next_request(&mut self) -> u64 {
        self.request_serial = self.request_serial.wrapping_add(1);
        self.request_serial
    }

    pub fn reset_scope(&mut self) {
        self.update_release = None;
        self.update_install = None;
        self.ipv6_editor = None;
        self.update_request = None;
        self.log_refresh_requested = true;
        self.update_entry_requested = true;
    }

    pub fn close_dialog(&mut self) -> bool {
        self.ipv6_editor.take().is_some() || self.update_install.take().is_some()
    }

    pub fn checking_update(&self) -> bool {
        self.update_request.is_some()
    }
}

impl App {
    fn update_scope(&self) -> Scope {
        self.snapshot
            .as_ref()
            .filter(|_| self.snapshot_current)
            .map_or_else(|| self.mode.into(), |snapshot| snapshot.scope)
    }

    pub(crate) fn settings_tab_entered(&mut self) -> Task<Message> {
        match self.settings_tab {
            SettingsTab::Diagnostics => {
                self.log_size = None;
                self.settings_actions.log_refresh_requested = true;
                self.refresh_settings_log_size()
            }
            SettingsTab::Updates if self.draft.settings.check_updates => {
                self.settings_actions.update_entry_requested = true;
                self.check_desktop_update(CheckTrigger::OnEntry)
            }
            _ => Task::none(),
        }
    }

    pub(crate) fn refresh_settings_log_size(&mut self) -> Task<Message> {
        if self.page != Page::Settings
            || self.settings_tab != SettingsTab::Diagnostics
            || !self.settings_actions.log_refresh_requested
            || !self.can_read_log_size()
        {
            return Task::none();
        }
        self.settings_actions.log_refresh_requested = false;
        self.execute(bitnet_core::Command::LogSize)
    }

    pub(crate) fn check_desktop_update(&mut self, trigger: CheckTrigger) -> Task<Message> {
        if self.busy
            || self.connecting
            || crate::preview_only()
            || self.settings_actions.update_install.is_some()
        {
            return Task::none();
        }
        let trigger = if matches!(trigger, CheckTrigger::Scheduled)
            && self.settings_actions.update_entry_requested
            && self.page == Page::Settings
            && self.settings_tab == SettingsTab::Updates
            && self.draft.settings.check_updates
        {
            CheckTrigger::OnEntry
        } else {
            trigger
        };
        let scheduled = matches!(trigger, CheckTrigger::Scheduled);
        let started = if scheduled {
            self.snapshot.as_ref().is_some_and(|snapshot| {
                self.update_policy
                    .due(&snapshot.settings, bitnet_core::now_secs())
            })
        } else {
            self.update_policy.begin_manual()
        };
        if !started {
            return Task::none();
        }
        self.settings_actions.update_entry_requested = false;
        let request = self.settings_actions.next_request();
        self.settings_actions.update_request = Some(request);
        let auto = scheduled
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.settings.auto_update);
        Task::perform(
            update_policy::check_desktop(self.update_scope(), auto),
            move |result| Message::UpdateChecked(request, result, trigger),
        )
    }

    pub(crate) fn desktop_update_checked(
        &mut self,
        request: u64,
        result: Result<UpdateOutcome, AppError>,
        trigger: CheckTrigger,
    ) -> Task<Message> {
        if self.settings_actions.update_request != Some(request) {
            return Task::none();
        }
        self.settings_actions.update_request = None;
        self.update_policy
            .complete(result.is_ok(), bitnet_core::now_secs());
        match result {
            Ok(UpdateOutcome::Available { version }) => {
                let new_release = self.settings_actions.update_release.as_ref() != Some(&version);
                self.settings_actions.update_release = Some(version.clone());
                if new_release || matches!(trigger, CheckTrigger::User) {
                    self.show_notice(
                        false,
                        format!("{} {version}", self.t("发现新版本", "Update available")),
                    );
                }
                if new_release && matches!(trigger, CheckTrigger::Scheduled) {
                    self.pending_notification
                        .get_or_insert(crate::notifications::Kind::Update);
                }
            }
            Ok(UpdateOutcome::PublicRelease { version }) => {
                self.settings_actions.update_release = Some(version);
            }
            Ok(UpdateOutcome::InstallerStarted { message }) => {
                self.settings_actions.update_release = None;
                self.show_notice(false, message);
            }
            Ok(UpdateOutcome::NoUpdate | UpdateOutcome::Unconfigured) => {
                self.settings_actions.update_release = None;
                if matches!(trigger, CheckTrigger::User) {
                    self.show_notice(
                        false,
                        self.t("当前没有新版", "No newer release is available")
                            .into(),
                    );
                }
            }
            Err(error) => {
                if !matches!(trigger, CheckTrigger::Scheduled) || self.notice.is_none() {
                    self.error(error);
                }
            }
        }
        Task::none()
    }

    pub(crate) fn settings_action(&mut self, action: Action) -> Task<Message> {
        match action {
            Action::OpenIpv6Editor => {
                self.settings_actions.ipv6_editor = Some(self.draft.settings.ipv6_targets.clone());
            }
            Action::Ipv6TargetChanged(index, value) => {
                if let Some(target) = self
                    .settings_actions
                    .ipv6_editor
                    .as_mut()
                    .and_then(|targets| targets.get_mut(index))
                {
                    *target = value;
                }
            }
            Action::AddIpv6Target => {
                if let Some(targets) = self
                    .settings_actions
                    .ipv6_editor
                    .as_mut()
                    .filter(|targets| targets.len() < bitnet_core::MAX_IPV6_TARGETS)
                {
                    targets.push(String::new());
                }
            }
            Action::RemoveIpv6Target(index) => {
                if let Some(targets) = self
                    .settings_actions
                    .ipv6_editor
                    .as_mut()
                    .filter(|targets| index < targets.len())
                {
                    targets.remove(index);
                }
            }
            Action::SaveIpv6Targets => {
                if let Some(targets) = self.settings_actions.ipv6_editor.clone() {
                    match self.draft.settings.set_ipv6_targets(targets) {
                        Ok(()) => {
                            self.settings_actions.ipv6_editor = None;
                            self.clear_notice();
                            self.mark_settings_dirty();
                        }
                        Err(error) => self.error(error),
                    }
                }
            }
            Action::CloseIpv6Editor => self.settings_actions.ipv6_editor = None,
            Action::OpenInstaller => {
                if self.busy
                    || self.connecting
                    || crate::preview_only()
                    || self.settings_actions.update_install.is_some()
                {
                    return Task::none();
                }
                let request = self.settings_actions.next_request();
                self.settings_actions.update_install = Some(UpdateInstall {
                    request,
                    result: None,
                });
                return Task::perform(
                    maintenance::check_installable_update(self.update_scope()),
                    move |result| Message::SettingsAction(Action::VerifiedUpdate(request, result)),
                );
            }
            Action::VerifiedUpdate(request, result) => {
                if let Some(install) = self
                    .settings_actions
                    .update_install
                    .as_mut()
                    .filter(|install| install.request == request)
                {
                    install.result = Some(result);
                }
            }
            Action::InstallVerifiedUpdate => {
                if self.busy || self.connecting || crate::preview_only() {
                    return Task::none();
                }
                let Some(update) = self
                    .settings_actions
                    .update_install
                    .as_ref()
                    .and_then(|install| install.result.as_ref())
                    .and_then(|result| result.as_ref().ok())
                    .and_then(Option::as_ref)
                    .filter(|update| update.scope == self.update_scope())
                else {
                    return Task::none();
                };
                let action = MaintenanceAction::InstallCheckedUpdate {
                    manifest_sha256: update.manifest_sha256.clone(),
                };
                self.settings_actions.update_install = None;
                return self.maintain(action);
            }
            Action::CloseInstaller => self.settings_actions.update_install = None,
            Action::Export(destination) => {
                if self.busy || self.settings_actions.export_pending || !self.snapshot_current {
                    return Task::none();
                }
                self.settings_actions.export_pending = true;
                let scope = self.update_scope();
                return Task::perform(
                    async move {
                        match Client::exact(scope)
                            .command(bitnet_core::Command::ExportSettings)
                            .await?
                        {
                            ReplyData::Settings(value) => serde_json::to_string_pretty(&value)
                                .map_err(|_| {
                                    AppError::new(
                                        ErrorCode::Internal,
                                        "Could not encode the exported settings",
                                    )
                                }),
                            _ => Err(AppError::new(
                                ErrorCode::Incompatible,
                                "The engine did not return settings",
                            )),
                        }
                    },
                    move |result| {
                        Message::SettingsAction(Action::ExportPrepared(destination, scope, result))
                    },
                );
            }
            Action::ExportPrepared(destination, scope, result) => {
                let encoded = match result {
                    Ok(encoded) if scope == self.update_scope() && self.snapshot_current => encoded,
                    Ok(_) => {
                        self.settings_actions.export_pending = false;
                        self.show_notice(
                            true,
                            self.t(
                                "连接范围已变化，请重新导出",
                                "Connection scope changed. Export again.",
                            )
                            .into(),
                        );
                        return Task::none();
                    }
                    Err(error) => {
                        self.settings_actions.export_pending = false;
                        self.error(error);
                        return Task::none();
                    }
                };
                return match destination {
                    ExportTo::Clipboard => iced::clipboard::write(encoded).map(|result| {
                        Message::SettingsAction(Action::ExportCopied(result.is_ok()))
                    }),
                    ExportTo::File => save_settings_export(encoded, self.english()),
                };
            }
            Action::ExportCopied(copied) => {
                self.settings_actions.export_pending = false;
                self.show_notice(
                    !copied,
                    if copied {
                        self.t("已复制", "Copied")
                    } else {
                        self.t("无法复制到剪贴板", "Could not copy to the clipboard")
                    }
                    .into(),
                );
            }
            Action::ExportSaved(saved) => {
                self.settings_actions.export_pending = false;
                match saved {
                    Ok(true) => self.show_notice(
                        false,
                        self.t("设置已导出到文件", "Settings exported to file")
                            .into(),
                    ),
                    Ok(false) => {}
                    Err(()) => self.show_notice(
                        true,
                        self.t(
                            "无法打开保存窗口或写入文件",
                            "Could not open the save dialog or write the file",
                        )
                        .into(),
                    ),
                }
            }
        }
        Task::none()
    }
}

fn save_settings_export(encoded: String, english: bool) -> Task<Message> {
    let (send, receive) = iced::futures::channel::oneshot::channel();
    let _ = std::thread::Builder::new().spawn(move || {
        let result = (|| {
            let Some(path) = save_dialog(english)? else {
                return Ok(false);
            };
            if !path.is_absolute() {
                return Err(());
            }
            bitnet_runtime::persistence::atomic_write(&path, encoded.as_bytes()).map_err(|_| ())?;
            Ok(true)
        })();
        let _ = send.send(result);
    });
    Task::perform(async move { receive.await.unwrap_or(Err(())) }, |result| {
        Message::SettingsAction(Action::ExportSaved(result))
    })
}

#[cfg(windows)]
fn save_dialog(english: bool) -> Result<Option<PathBuf>, ()> {
    use std::os::windows::process::CommandExt as _;
    let system = std::env::var_os("SystemRoot").ok_or(())?;
    let program = PathBuf::from(system).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    let mut command = Command::new(program);
    command.creation_flags(0x0800_0000);
    command.env("BITNET_SAVE_LANGUAGE", if english { "en" } else { "zh" });
    command.args([
        "-NoProfile",
        "-STA",
        "-Command",
        r#"
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = New-Object Text.UTF8Encoding($false)
try {
    Add-Type -AssemblyName System.Windows.Forms
    $dialog = New-Object Windows.Forms.SaveFileDialog
    $dialog.Title = if ($env:BITNET_SAVE_LANGUAGE -eq 'en') { 'Export settings' } else {
        [string]::Concat([char]0x5BFC, [char]0x51FA, [char]0x8BBE, [char]0x7F6E)
    }
    $dialog.FileName = 'bitnet-settings.json'
    $dialog.Filter = 'JSON (*.json)|*.json'
    $dialog.DefaultExt = 'json'
    $dialog.RestoreDirectory = $true
    $dialog.OverwritePrompt = $true
    if ($dialog.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { exit 1 }
    [Console]::Write($dialog.FileName)
} catch {
    exit 2
} finally { if ($dialog) { $dialog.Dispose() } }
"#,
    ]);
    dialog_output(command)
}

#[cfg(target_os = "linux")]
fn save_dialog(english: bool) -> Result<Option<PathBuf>, ()> {
    let title = if english {
        "Export settings"
    } else {
        "导出设置"
    };
    let kde = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .split(':')
        .any(|name| name.eq_ignore_ascii_case("kde"));
    let helpers = if kde {
        ["/usr/bin/kdialog", "/usr/bin/zenity"]
    } else {
        ["/usr/bin/zenity", "/usr/bin/kdialog"]
    };
    let helper = helpers
        .into_iter()
        .find(|helper| std::path::Path::new(helper).is_file())
        .ok_or(())?;
    let mut command = Command::new(helper);
    if helper.ends_with("kdialog") {
        command.args([
            "--title",
            title,
            "--getsavefilename",
            "bitnet-settings.json",
            "*.json|JSON (*.json)",
        ]);
    } else {
        command.args([
            "--file-selection",
            "--save",
            "--confirm-overwrite",
            "--title",
            title,
            "--filename",
            "bitnet-settings.json",
            "--file-filter",
            "JSON | *.json",
        ]);
    }
    dialog_output(command)
}

#[cfg(target_os = "macos")]
fn save_dialog(english: bool) -> Result<Option<PathBuf>, ()> {
    let prompt = if english {
        "Export settings"
    } else {
        "导出设置"
    };
    let mut command = Command::new("/usr/bin/osascript");
    command.args(["-e", "on run argv", "-e", "try", "-e", "return POSIX path of (choose file name with prompt (item 1 of argv) default name \"bitnet-settings.json\")", "-e", "on error number -128", "-e", "return \"\"", "-e", "end try", "-e", "end run", prompt]);
    dialog_output(command)
}

fn dialog_output(mut command: Command) -> Result<Option<PathBuf>, ()> {
    let output = command
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|_| ())?;
    if !output.status.success() {
        return if output.status.code() == Some(1) && output.stdout.is_empty() {
            Ok(None)
        } else {
            Err(())
        };
    }
    let value = String::from_utf8(output.stdout).map_err(|_| ())?;
    let path = value.trim_end_matches(['\r', '\n']);
    Ok((!path.is_empty()).then(|| PathBuf::from(path)))
}
