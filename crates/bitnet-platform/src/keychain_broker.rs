//! launchd owns the socket and root helper; only the dedicated worker identity may obtain a key.
use super::keychain;
use bitnet_core::{AppError, ErrorCode, Scope};
use std::{
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::net::{UnixListener, UnixStream},
    },
    path::Path,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

const SOCKET: &str = "/var/run/bitnet-keychain.sock";
const REQUEST: &[u8; 16] = b"BITNET-KEY-V1\0\0\0";
extern "C" {
    fn launch_activate_socket(
        name: *const libc::c_char,
        fds: *mut *mut libc::c_int,
        count: *mut usize,
    ) -> libc::c_int;
}
fn unavailable() -> AppError {
    crate::error(
        ErrorCode::CredentialsUnavailable,
        "The privileged Keychain broker is unavailable or denied access",
    )
}
fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t, AppError> {
    let (mut uid, mut gid) = (0, 0);
    if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
        return Err(unavailable());
    }
    Ok(uid)
}
pub(super) fn request_key() -> Result<[u8; 32], AppError> {
    let mut stream = UnixStream::connect(SOCKET).map_err(|_| unavailable())?;
    if peer_uid(&stream)? != 0 {
        return Err(unavailable());
    }
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .map_err(|_| unavailable())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|_| unavailable())?;
    stream.write_all(REQUEST).map_err(|_| unavailable())?;
    let mut key = Zeroizing::new([0; 32]);
    stream.read_exact(key.as_mut()).map_err(|_| unavailable())?;
    Ok(*key)
}
pub(super) fn run() -> Result<(), AppError> {
    if unsafe { libc::geteuid() } != 0 {
        return Err(unavailable());
    }
    let worker = unsafe { libc::getpwnam(c"_bitnet".as_ptr()) };
    if worker.is_null() {
        return Err(unavailable());
    }
    let worker_uid = unsafe { (*worker).pw_uid };
    if worker_uid == 0 {
        return Err(unavailable());
    }
    // /var/run is recreated at boot. Provision the public, non-writable-by-other-users
    // control socket parent before accessing Keychain, so credential repair IPC remains available.
    let runtime = Path::new("/var/run/bitnet");
    crate::reject_link(runtime)?;
    std::fs::create_dir_all(runtime).map_err(|_| unavailable())?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(runtime, std::fs::Permissions::from_mode(0o755))
        .map_err(|_| unavailable())?;
    if unsafe { libc::chown(c"/var/run/bitnet".as_ptr(), worker_uid, (*worker).pw_gid) } != 0 {
        return Err(unavailable());
    }
    let (mut fds, mut count) = (std::ptr::null_mut(), 0usize);
    let result = unsafe { launch_activate_socket(c"Keychain".as_ptr(), &mut fds, &mut count) };
    if result != 0 || count != 1 || fds.is_null() {
        return Err(unavailable());
    }
    let listener = unsafe { UnixListener::from_raw_fd(*fds) };
    unsafe { libc::free(fds.cast()) };
    listener.set_nonblocking(true).map_err(|_| unavailable())?;
    let mut idle = Instant::now();
    while idle.elapsed() < Duration::from_secs(10) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let uid = peer_uid(&stream)?;
                if uid != worker_uid && uid != 0 {
                    continue;
                }
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .map_err(|_| unavailable())?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .map_err(|_| unavailable())?;
                let mut request = [0; 16];
                if stream.read_exact(&mut request).is_err() || &request != REQUEST {
                    continue;
                }
                let key = Zeroizing::new(keychain::key(
                    Scope::System,
                    Path::new("/Library/Application Support/BitNet"),
                )?);
                stream.write_all(key.as_ref()).map_err(|_| unavailable())?;
                idle = Instant::now();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err(unavailable()),
        }
    }
    Ok(())
}
