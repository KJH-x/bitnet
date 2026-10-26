//! One active controller per authentication endpoint and local source address.
//! These locks coordinate cooperating processes; they are not credential stores.
use bitnet_core::{AppError, ErrorCode, NetworkBinding};
use sha2::{Digest, Sha256};

pub fn domain_key(endpoint: &str, binding: &NetworkBinding) -> String {
    // Do not key by user or data directory: separate users share this domain.
    let origin = url::Url::parse(endpoint)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| endpoint.into());
    format!(
        "{:x}",
        Sha256::digest(format!("{origin}|{}", binding.source_ip))
    )
}

pub struct DomainLock {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
    #[cfg(target_os = "linux")]
    _socket: std::os::unix::net::UnixListener,
    #[cfg(target_os = "macos")]
    _file: std::fs::File,
}

fn busy() -> AppError {
    AppError::new(
        ErrorCode::Busy,
        "此认证连接已有其他控制器，请使用现有后台或停止另一个用户引擎",
    )
}

impl DomainLock {
    pub fn acquire(key: &str) -> Result<Self, AppError> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::{
                Foundation::*, Security::Authorization::*, Security::*, System::Threading::*,
            };
            let sddl = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;0x00100001;;;AU)\0"
                .encode_utf16()
                .collect::<Vec<_>>();
            let mut descriptor = std::ptr::null_mut();
            if unsafe {
                ConvertStringSecurityDescriptorToSecurityDescriptorW(
                    sddl.as_ptr(),
                    1,
                    &mut descriptor,
                    std::ptr::null_mut(),
                )
            } == 0
            {
                return Err(busy());
            }
            let attributes = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor,
                bInheritHandle: 0,
            };
            let name = format!("Global\\BitNetDomain-v1-{key}\0")
                .encode_utf16()
                .collect::<Vec<_>>();
            let handle = unsafe { CreateMutexExW(&attributes, name.as_ptr(), 0, 0x00100001) };
            unsafe {
                LocalFree(descriptor);
            }
            if handle.is_null() {
                return Err(busy());
            }
            let status = unsafe { WaitForSingleObject(handle, 0) };
            if status != WAIT_OBJECT_0 && status != WAIT_ABANDONED {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(busy());
            }
            Ok(Self { handle })
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::{
                linux::net::SocketAddrExt,
                unix::net::{SocketAddr, UnixListener},
            };
            // Abstract AF_UNIX names are atomically reserved across users and
            // systemd PrivateTmp mounts; no user can unlink a live reservation.
            let address = SocketAddr::from_abstract_name(format!("bitnet-domain-v1-{key}"))
                .map_err(|_| busy())?;
            let socket = UnixListener::bind_addr(&address).map_err(|_| busy())?;
            Ok(Self { _socket: socket })
        }
        #[cfg(target_os = "macos")]
        {
            use fs2::FileExt;
            use std::os::{
                fd::AsRawFd,
                unix::fs::{MetadataExt, OpenOptionsExt},
            };
            let path = std::path::PathBuf::from(format!("/tmp/bitnet-domain-v1-{key}.lock"));
            let mut options = std::fs::OpenOptions::new();
            options
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
            let file = match options.create_new(true).mode(0o666).open(&path) {
                Ok(file) => {
                    if unsafe { libc::fchmod(file.as_raw_fd(), 0o666) } != 0 {
                        return Err(busy());
                    }
                    file
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    options.create_new(false).open(&path).map_err(|_| busy())?
                }
                Err(_) => return Err(busy()),
            };
            let held = file.metadata().map_err(|_| busy())?;
            let named = std::fs::symlink_metadata(&path).map_err(|_| busy())?;
            if !held.is_file()
                || held.nlink() != 1
                || held.ino() != named.ino()
                || held.dev() != named.dev()
            {
                return Err(busy());
            }
            file.try_lock_exclusive().map_err(|_| busy())?;
            // Never unlink lock files: the kernel releases the flock on exit.
            Ok(Self { _file: file })
        }
    }
}

#[cfg(windows)]
impl Drop for DomainLock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.handle);
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_ignore_user_and_interface_alias_but_separate_auth_domains() {
        let mut binding = NetworkBinding {
            interface_id: "first".into(),
            interface_index: 1,
            source_ip: "10.1.2.3".parse().unwrap(),
        };
        let first = domain_key("http://10.0.0.55", &binding);
        binding.interface_id = "alias".into();
        binding.interface_index = 42;
        assert_eq!(first, domain_key("http://10.0.0.55", &binding));
        assert_eq!(first, domain_key("http://10.0.0.55:80/", &binding));
        binding.source_ip = "10.1.2.4".parse().unwrap();
        assert_ne!(first, domain_key("http://10.0.0.55", &binding));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn a_live_domain_cannot_be_reserved_twice_and_exit_releases_it() {
        let key = format!("test-{}-{}", std::process::id(), bitnet_core::now_ms());
        let held = DomainLock::acquire(&key).unwrap();
        assert!(DomainLock::acquire(&key).is_err());
        drop(held);
        assert!(DomainLock::acquire(&key).is_ok());
    }
    #[cfg(windows)]
    #[test]
    fn cooperating_threads_cannot_control_the_same_domain() {
        let key = format!("test-{}-{}", std::process::id(), bitnet_core::now_ms());
        let held = DomainLock::acquire(&key).unwrap();
        let other = key.clone();
        assert!(
            std::thread::spawn(move || DomainLock::acquire(&other).is_err())
                .join()
                .unwrap()
        );
        drop(held);
        assert!(
            std::thread::spawn(move || DomainLock::acquire(&key).is_ok())
                .join()
                .unwrap()
        );
    }
}
