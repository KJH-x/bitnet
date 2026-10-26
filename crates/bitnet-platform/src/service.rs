//! Queries and runs service hosts. This module never installs, starts, or removes a service.
use bitnet_core::{AppError, ServiceStatus};
use std::sync::atomic::{AtomicBool, Ordering};
static STOPPING: AtomicBool = AtomicBool::new(false);
pub fn is_stopping() -> bool {
    STOPPING.load(Ordering::Acquire)
}

#[cfg(windows)]
#[path = "service_windows.rs"]
mod native;

pub fn status() -> ServiceStatus {
    #[cfg(windows)]
    {
        native::status()
    }
    #[cfg(unix)]
    {
        unix_status()
    }
}
pub fn is_admin() -> bool {
    #[cfg(windows)]
    {
        native::is_admin()
    }
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
}
pub fn run(
    callback: impl FnOnce() -> Result<(), AppError> + Send + 'static,
) -> Result<(), AppError> {
    STOPPING.store(false, Ordering::Release);
    #[cfg(windows)]
    {
        native::run(callback)
    }
    #[cfg(unix)]
    {
        callback()
    }
}

/// Notify systemd only when called by a live, functioning engine iteration.
pub fn notify_ready() -> Result<(), AppError> {
    notify("READY=1")
}
pub fn notify_watchdog() -> Result<(), AppError> {
    notify("WATCHDOG=1")
}
fn notify(message: &str) -> Result<(), AppError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::{
            linux::net::SocketAddrExt,
            unix::net::{SocketAddr, UnixDatagram},
        };
        let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
            return Ok(());
        };
        use std::os::unix::ffi::OsStrExt;
        let path = path.as_os_str().as_bytes();
        let addr = if path.first() == Some(&b'@') {
            SocketAddr::from_abstract_name(&path[1..])
        } else {
            SocketAddr::from_pathname(std::path::Path::new(std::ffi::OsStr::from_bytes(path)))
        }
        .map_err(crate::io_error)?;
        UnixDatagram::unbound()
            .and_then(|s| s.send_to_addr(message.as_bytes(), &addr))
            .map_err(crate::io_error)?;
    }
    let _ = message;
    Ok(())
}

#[cfg(unix)]
fn unix_status() -> ServiceStatus {
    #[cfg(target_os = "linux")]
    {
        let result = bounded_output(
            "/usr/bin/systemctl",
            &[
                "show",
                crate::UNIX_SERVICE_NAME,
                "--property=LoadState",
                "--property=ActiveState",
                "--property=UnitFileState",
                "--no-pager",
            ],
        );
        match result {
            Ok((true, s)) => ServiceStatus {
                installed: !s.lines().any(|l| l == "LoadState=not-found"),
                running: s.lines().any(|l| l == "ActiveState=active"),
                enabled: s.lines().any(|l| {
                    matches!(l, "UnitFileState=enabled" | "UnitFileState=enabled-runtime")
                }),
                detail: None,
            },
            _ => ServiceStatus {
                installed: std::path::Path::new("/etc/systemd/system/bitnet.service").exists()
                    || std::path::Path::new("/usr/lib/systemd/system/bitnet.service").exists()
                    || std::path::Path::new("/lib/systemd/system/bitnet.service").exists(),
                running: false,
                enabled: false,
                detail: Some("System service state could not be queried".into()),
            },
        }
    }
    #[cfg(target_os = "macos")]
    {
        match bounded_output(
            "/bin/launchctl",
            &["print", &format!("system/{}", crate::MACOS_SERVICE_LABEL)],
        ) {
            Ok((true, s)) => ServiceStatus {
                installed: true,
                running: s.lines().any(|l| l.trim() == "state = running"),
                enabled: true,
                detail: None,
            },
            _ => ServiceStatus {
                installed: std::path::Path::new("/Library/LaunchDaemons/top.nsapi.bitnet.plist")
                    .exists(),
                running: false,
                enabled: false,
                detail: Some("launchd job is not running or could not be queried".into()),
            },
        }
    }
}
#[cfg(unix)]
fn bounded_output(program: &str, args: &[&str]) -> Result<(bool, String), AppError> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(crate::io_error)?;
    use std::io::Read;
    let output = child.stdout.take().ok_or_else(|| {
        crate::error(
            bitnet_core::ErrorCode::Unavailable,
            "Service status output is unavailable",
        )
    })?;
    let reader = match std::thread::Builder::new()
        .name("bitnet-service-query".into())
        .spawn(move || {
            let mut bytes = Vec::new();
            output.take(32 * 1024 + 1).read_to_end(&mut bytes)?;
            if bytes.len() > 32 * 1024 {
                return Err(std::io::Error::other("Service status output exceeds limit"));
            }
            Ok(bytes)
        }) {
        Ok(reader) => reader,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(crate::io_error(error));
        }
    };
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().map_err(crate::io_error)? {
            let bytes = reader
                .join()
                .map_err(|_| {
                    crate::error(
                        bitnet_core::ErrorCode::Unavailable,
                        "Service status reader failed",
                    )
                })?
                .map_err(crate::io_error)?;
            return Ok((
                status.success(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        if start.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(crate::error(
                bitnet_core::ErrorCode::Unavailable,
                "Service status query timed out",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
