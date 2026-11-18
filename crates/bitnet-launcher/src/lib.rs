use bitnet_core::Scope;
use std::fmt;
use std::process::{Command, ExitStatus, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebAction {
    Open,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchError {
    Missing,
    Failed,
}
impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "未安装对应前端",
            Self::Failed => "前端未能完成操作",
        })
    }
}

pub fn web(scope: Scope, action: WebAction, english: bool) -> Result<(), LaunchError> {
    let mut process = companion("bitnet-web", scope)?;
    match action {
        WebAction::Open => process.args(["--lang", if english { "en" } else { "zh" }]),
        WebAction::Stop => process.arg("--stop"),
    };
    process
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW
        process.creation_flags(0x0800_0000);
    }
    if process.status().is_ok_and(|status| status.success()) {
        Ok(())
    } else {
        Err(LaunchError::Failed)
    }
}

pub fn tui(scope: Scope) -> Result<ExitStatus, LaunchError> {
    companion("bitnet-tui", scope)?
        .status()
        .map_err(|_| LaunchError::Failed)
}

fn companion(name: &str, scope: Scope) -> Result<Command, LaunchError> {
    let executable = std::env::current_exe().map_err(|_| LaunchError::Failed)?;
    let directory = executable.parent().ok_or(LaunchError::Failed)?;
    let path = directory.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !path.is_file() {
        return Err(LaunchError::Missing);
    }
    let mut process = Command::new(path);
    process.args([
        "--scope",
        match scope {
            Scope::User => "user",
            Scope::System => "system",
        },
    ]);
    Ok(process)
}
