//! Native integration. No authentication policy or frontend dependencies.
use bitnet_core::{AppError, ErrorCode, Scope};
use std::path::{Path, PathBuf};

pub mod credentials;
pub mod network;
mod secure_fs;
pub mod service;
#[cfg(windows)]
mod windows;

pub const WINDOWS_SERVICE_NAME: &str = "BitNet";
pub const UNIX_SERVICE_NAME: &str = "bitnet.service";
pub const MACOS_SERVICE_LABEL: &str = "top.nsapi.bitnet";

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub data_dir: PathBuf,
    pub config_file: PathBuf,
    pub credentials_file: PathBuf,
    pub history_file: PathBuf,
    pub log_dir: PathBuf,
}

/// Computes locations only; calling this never creates files or directories.
pub fn paths(scope: Scope) -> Result<AppPaths, AppError> {
    #[cfg(windows)]
    let data = match scope {
        Scope::User => required_env_path("LOCALAPPDATA")?.join("BitNet"),
        Scope::System => required_env_path("ProgramData")?.join("BitNet"),
    };
    #[cfg(target_os = "linux")]
    let data = match scope {
        Scope::User => match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(v) => absolute(PathBuf::from(v))?.join("bitnet"),
            None => required_env_path("HOME")?.join(".local/share/bitnet"),
        },
        Scope::System => match std::env::var_os("STATE_DIRECTORY") {
            Some(v) => {
                let p = PathBuf::from(v);
                if p.to_string_lossy().contains(':') {
                    return Err(error(
                        ErrorCode::InvalidInput,
                        "Only one system state directory is supported",
                    ));
                }
                absolute(p)?
            }
            None => PathBuf::from("/var/lib/bitnet"),
        },
    };
    #[cfg(target_os = "macos")]
    let data = match scope {
        Scope::User => required_env_path("HOME")?.join("Library/Application Support/BitNet"),
        Scope::System => PathBuf::from("/Library/Application Support/BitNet"),
    };
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    let data: PathBuf = return Err(error(
        ErrorCode::Unavailable,
        "Unsupported operating system",
    ));
    let data = if scope == Scope::User {
        match std::env::var_os("BITNET_DATA_DIR").filter(|s| !s.is_empty()) {
            Some(p) => absolute(PathBuf::from(p))?,
            None => data,
        }
    } else {
        data
    };
    Ok(AppPaths {
        config_file: data.join("settings.json"),
        credentials_file: data.join("accounts.enc"),
        history_file: data.join("history.sqlite3"),
        log_dir: data.join("logs"),
        data_dir: data,
    })
}

fn required_env_path(name: &str) -> Result<PathBuf, AppError> {
    std::env::var_os(name)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            error(
                ErrorCode::Unavailable,
                "The operating system did not provide an application data directory",
            )
        })
        .and_then(absolute)
}
fn absolute(path: PathBuf) -> Result<PathBuf, AppError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(error(
            ErrorCode::InvalidInput,
            "Application paths must be absolute and cannot contain parent traversal",
        ));
    }
    Ok(path)
}
pub(crate) fn error(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message)
}
pub(crate) fn io_error(_: std::io::Error) -> AppError {
    error(
        ErrorCode::Unavailable,
        "Operating-system storage operation failed",
    )
}
pub(crate) fn random<const N: usize>() -> Result<[u8; N], AppError> {
    let mut b = [0; N];
    getrandom::getrandom(&mut b).map_err(|_| {
        error(
            ErrorCode::CredentialsUnavailable,
            "Secure random generator unavailable",
        )
    })?;
    Ok(b)
}
pub(crate) fn reject_link(path: &Path) -> Result<(), AppError> {
    match std::fs::symlink_metadata(path) {
        Ok(m) => {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if m.file_attributes() & 0x400 != 0 {
                    return Err(error(
                        ErrorCode::PermissionDenied,
                        "Reparse points are not allowed in protected storage",
                    ));
                }
            }
            if m.file_type().is_symlink() {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Symbolic links are not allowed in protected storage",
                ));
            }
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io_error(e)),
    }
}
