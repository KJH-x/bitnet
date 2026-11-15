//! Length-delimited local IPC. Passwords are never sent before checking the server.
use bitnet_core::*;
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{mpsc, oneshot, watch},
};
use zeroize::Zeroizing;

pub const MAX_FRAME: usize = 128 * 1024;
#[derive(Debug, Clone)]
pub struct Caller {
    pub administrator: bool,
    pub owner: bool,
}
pub struct Envelope {
    pub request: Request,
    pub caller: Caller,
    pub answer: oneshot::Sender<Reply>,
    pub queued_at: std::time::Instant,
    pub response_written: oneshot::Receiver<()>,
}

async fn read_frame<R: AsyncRead + Unpin>(stream: &mut R) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let len = stream.read_u32().await.map_err(|_| unavailable())? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "IPC 消息长度超出限制",
        ));
    }
    let mut bytes = Zeroizing::new(vec![0; len]);
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| unavailable())?;
    Ok(bytes)
}
async fn write_frame<W: AsyncWrite + Unpin>(stream: &mut W, bytes: &[u8]) -> Result<(), AppError> {
    if bytes.len() > MAX_FRAME {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "IPC 返回数据过大，请缩小查询范围",
        ));
    }
    stream
        .write_u32(bytes.len() as u32)
        .await
        .map_err(|_| unavailable())?;
    stream.write_all(bytes).await.map_err(|_| unavailable())?;
    stream.flush().await.map_err(|_| unavailable())
}
fn unavailable() -> AppError {
    AppError::new(ErrorCode::Unavailable, "无法连接本地认证引擎")
}

async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    request: &Request,
) -> Result<ReplyData, AppError> {
    let bytes = Zeroizing::new(
        serde_json::to_vec(request)
            .map_err(|_| AppError::new(ErrorCode::Internal, "命令编码失败"))?,
    );
    write_frame(stream, &bytes).await?;
    let response = read_frame(stream).await?;
    let reply: Reply = serde_json::from_slice(&response)
        .map_err(|_| AppError::new(ErrorCode::Incompatible, "本地引擎返回了不兼容的数据"))?;
    if reply.api_version != API_VERSION || reply.request_id != request.request_id {
        return Err(AppError::new(
            ErrorCode::Incompatible,
            "本地引擎协议或请求标识不匹配",
        ));
    }
    reply.result
}

#[cfg(unix)]
async fn dispatch<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    caller: Caller,
    queue: mpsc::Sender<Envelope>,
    state: watch::Receiver<Snapshot>,
) -> Result<(), AppError> {
    let bytes = read_frame(stream).await?;
    let request: Request = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "无效的本地命令"))?;
    if request.request_id.len() > 128 {
        return Err(AppError::new(ErrorCode::InvalidInput, "请求标识过长"));
    }
    let (reply, receipt) = process_request(request, caller, queue, state).await;
    let output = serde_json::to_vec(&reply)
        .map_err(|_| AppError::new(ErrorCode::Internal, "返回数据编码失败"))?;
    write_frame(stream, &output).await?;
    if let Some(receipt) = receipt {
        let _ = receipt.send(());
    }
    Ok(())
}

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        fs::OpenOptions,
        os::windows::{
            fs::OpenOptionsExt,
            io::{AsRawHandle, IntoRawHandle},
        },
    };
    use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
    use windows_sys::Win32::{
        Foundation::*,
        Security::Authorization::*,
        Security::*,
        System::{Pipes::*, Services::*, Threading::*},
    };

    fn sid_for_token(token: HANDLE) -> Result<String, AppError> {
        let mut needed = 0;
        unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut needed) };
        let mut words = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                words.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(unavailable());
        }
        let info = unsafe { &*(words.as_ptr().cast::<TOKEN_USER>()) };
        let mut out = std::ptr::null_mut();
        if unsafe { ConvertSidToStringSidW(info.User.Sid, &mut out) } == 0 {
            return Err(unavailable());
        }
        let mut len = 0;
        unsafe {
            while *out.add(len) != 0 {
                len += 1;
            }
        }
        let result = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(out, len) });
        unsafe { LocalFree(out.cast()) };
        Ok(result)
    }
    fn process_sid(pid: u32) -> Result<String, AppError> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return Err(unavailable());
        }
        let mut token = std::ptr::null_mut();
        let ok = unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) };
        unsafe { CloseHandle(process) };
        if ok == 0 {
            return Err(unavailable());
        }
        let result = sid_for_token(token);
        unsafe { CloseHandle(token) };
        result
    }
    pub fn endpoint(scope: Scope) -> Result<String, AppError> {
        Ok(match scope {
            Scope::System => r"\\.\pipe\BitNet-v1-system".into(),
            Scope::User => format!(
                r"\\.\pipe\BitNet-v1-user-{}{}",
                process_sid(std::process::id())?,
                namespace_suffix()
            ),
        })
    }
    struct Descriptor(PSECURITY_DESCRIPTOR);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn descriptor(scope: Scope) -> Result<Descriptor, AppError> {
        // Deliberately omit FILE_CREATE_PIPE_INSTANCE (0x4) for ordinary clients.
        let sid = process_sid(std::process::id())?;
        let public = if scope == Scope::System {
            "(A;;0x00120183;;;AU)"
        } else {
            ""
        };
        let sddl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid}){public}");
        let wide = sddl.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut sd = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                1,
                &mut sd,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "无法设置本地管道访问权限",
            ));
        }
        Ok(Descriptor(sd))
    }
    fn create(scope: Scope, first: bool) -> Result<NamedPipeServer, AppError> {
        let sd = descriptor(scope)?;
        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: 0,
        };
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .max_instances(32);
        unsafe {
            options.create_with_security_attributes_raw(
                endpoint(scope)?,
                (&sa as *const SECURITY_ATTRIBUTES).cast_mut().cast(),
            )
        }
        .map_err(|_| AppError::new(ErrorCode::Busy, "已有引擎占用本地控制端点"))
    }
    fn caller(server: &NamedPipeServer) -> Result<Caller, AppError> {
        let own_sid = process_sid(std::process::id())?;
        if unsafe { ImpersonateNamedPipeClient(server.as_raw_handle()) } == 0 {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "无法验证调用者身份",
            ));
        }
        struct Revert;
        impl Drop for Revert {
            fn drop(&mut self) {
                unsafe { RevertToSelf() };
            }
        }
        let _guard = Revert;
        let mut token = std::ptr::null_mut();
        if unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) } == 0 {
            return Err(unavailable());
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0;
        unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            )
        };
        let peer = sid_for_token(token);
        unsafe { CloseHandle(token) };
        Ok(Caller {
            administrator: elevation.TokenIsElevated != 0,
            owner: peer? == own_sid,
        })
    }
    fn service_pid() -> Result<u32, AppError> {
        let manager =
            unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
        if manager.is_null() {
            return Err(unavailable());
        }
        let name = "BitNet\0".encode_utf16().collect::<Vec<_>>();
        let service = unsafe { OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_STATUS) };
        unsafe { CloseServiceHandle(manager) };
        if service.is_null() {
            return Err(unavailable());
        }
        let mut status: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
        let mut size = 0;
        let ok = unsafe {
            QueryServiceStatusEx(
                service,
                SC_STATUS_PROCESS_INFO,
                (&mut status as *mut SERVICE_STATUS_PROCESS).cast(),
                std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut size,
            )
        };
        unsafe { CloseServiceHandle(service) };
        if ok == 0 || status.dwCurrentState != SERVICE_RUNNING || status.dwProcessId == 0 {
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "系统后台未就绪；请检查服务状态",
            ));
        }
        Ok(status.dwProcessId)
    }
    pub async fn request(scope: Scope, request: &Request) -> Result<ReplyData, AppError> {
        let file = OpenOptions::new()
            .access_mode(0x00120183)
            .custom_flags(0x40000000 | 0x00100000 | 0x00010000)
            .open(endpoint(scope)?)
            .map_err(|_| unavailable())?;
        let mut stream = unsafe { NamedPipeClient::from_raw_handle(file.into_raw_handle()) }
            .map_err(|_| unavailable())?;
        let mut pid = 0;
        if unsafe { GetNamedPipeServerProcessId(stream.as_raw_handle(), &mut pid) } == 0 {
            return Err(unavailable());
        }
        let trusted = match scope {
            Scope::System => pid == service_pid()?,
            Scope::User => process_sid(pid)? == process_sid(std::process::id())?,
        };
        if !trusted {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "本地引擎身份验证失败，未发送命令",
            ));
        }
        exchange(&mut stream, request).await
    }
    pub async fn serve(
        scope: Scope,
        queue: mpsc::Sender<Envelope>,
        state: watch::Receiver<Snapshot>,
    ) -> Result<(), AppError> {
        let mut server = create(scope, true)?;
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(24));
        loop {
            server.connect().await.map_err(|_| unavailable())?;
            let mut connection = server;
            server = create(scope, false)?;
            let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                continue;
            };
            let queue = queue.clone();
            let state = state.clone();
            tokio::spawn(async move {
                let _permit = permit;
                // Impersonation is meaningful after a client message has been read.
                // Read the request before capturing its token; never await while impersonating.
                let result = tokio::time::timeout(Duration::from_secs(30), async {
                    let bytes = read_frame(&mut connection).await?;
                    let caller = caller(&connection)?;
                    let request: Request = serde_json::from_slice(&bytes)
                        .map_err(|_| AppError::new(ErrorCode::InvalidInput, "无效的本地命令"))?;
                    let (reply, receipt) = process_request(request, caller, queue, state).await;
                    write_frame(
                        &mut connection,
                        &serde_json::to_vec(&reply).map_err(|_| unavailable())?,
                    )
                    .await?;
                    if let Some(receipt) = receipt {
                        let _ = receipt.send(());
                    }
                    Ok::<(), AppError>(())
                })
                .await;
                let _ = result;
            });
        }
    }
}

async fn process_request(
    request: Request,
    caller: Caller,
    queue: mpsc::Sender<Envelope>,
    state: watch::Receiver<Snapshot>,
) -> (Reply, Option<oneshot::Sender<()>>) {
    let request_id = request.request_id.clone();
    if request.api_version != API_VERSION || request.request_id.len() > 128 {
        return (
            Reply {
                api_version: API_VERSION,
                request_id,
                result: Err(AppError::new(ErrorCode::Incompatible, "本地协议不兼容")),
            },
            None,
        );
    }
    if matches!(request.command, Command::Status) {
        return (
            Reply {
                api_version: API_VERSION,
                request_id,
                result: Ok(ReplyData::Snapshot(Box::new(state.borrow().clone()))),
            },
            None,
        );
    }
    let (answer, wait) = oneshot::channel();
    let (receipt, response_written) = oneshot::channel();
    if queue
        .try_send(Envelope {
            request,
            caller,
            answer,
            queued_at: std::time::Instant::now(),
            response_written,
        })
        .is_err()
    {
        return (
            Reply {
                api_version: API_VERSION,
                request_id,
                result: Err(AppError::new(ErrorCode::Busy, "认证引擎正忙")),
            },
            None,
        );
    }
    (
        wait.await.unwrap_or(Reply {
            api_version: API_VERSION,
            request_id,
            result: Err(unavailable()),
        }),
        Some(receipt),
    )
}

#[cfg(unix)]
mod native {
    use super::*;
    #[cfg(target_os = "macos")]
    use std::os::unix::io::AsRawFd;
    use std::{
        os::unix::fs::{MetadataExt, PermissionsExt},
        path::PathBuf,
    };
    use tokio::net::{UnixListener, UnixStream};
    pub fn endpoint(scope: Scope) -> Result<String, AppError> {
        if scope == Scope::System {
            #[cfg(target_os = "macos")]
            return Ok("/var/run/bitnet/control.sock".into());
            #[cfg(not(target_os = "macos"))]
            return Ok("/run/bitnet/control.sock".into());
        }
        let uid = unsafe { libc::geteuid() };
        let base = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/bitnet-{uid}")));
        std::fs::create_dir_all(&base).map_err(|_| unavailable())?;
        let metadata = std::fs::symlink_metadata(&base).map_err(|_| unavailable())?;
        if metadata.uid() != uid || !metadata.is_dir() || metadata.permissions().mode() & 0o022 != 0
        {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "不安全的本地运行目录",
            ));
        }
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| unavailable())?;
        Ok(base
            .join(format!("bitnet-v1{}.sock", namespace_suffix()))
            .to_string_lossy()
            .into_owned())
    }
    fn peer(stream: &UnixStream) -> Result<(u32, u32), AppError> {
        #[cfg(target_os = "linux")]
        {
            let value = stream.peer_cred().map_err(|_| unavailable())?;
            Ok((value.uid(), value.pid().unwrap_or(0).max(0) as u32))
        }
        #[cfg(target_os = "macos")]
        {
            let mut uid = 0;
            let mut gid = 0;
            if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
                return Err(unavailable());
            }
            Ok((uid, 0))
        }
    }
    pub async fn request(scope: Scope, request: &Request) -> Result<ReplyData, AppError> {
        let path = endpoint(scope)?;
        let mut stream = UnixStream::connect(&path)
            .await
            .map_err(|_| unavailable())?;
        let (uid, pid) = peer(&stream)?;
        let trusted = match scope {
            Scope::User => uid == unsafe { libc::geteuid() },
            Scope::System => {
                #[cfg(target_os = "linux")]
                {
                    std::fs::read_to_string(format!("/proc/{pid}/cgroup"))
                        .map(|s| {
                            s.lines()
                                .any(|l| l.ends_with("/system.slice/bitnet.service"))
                        })
                        .unwrap_or(false)
                }
                #[cfg(target_os = "macos")]
                {
                    std::fs::metadata(&path)
                        .map(|m| m.uid() == uid)
                        .unwrap_or(false)
                        && std::fs::metadata("/var/run")
                            .map(|m| m.uid() == 0)
                            .unwrap_or(false)
                }
            }
        };
        if !trusted {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "本地引擎身份验证失败，未发送命令",
            ));
        }
        exchange(&mut stream, request).await
    }
    pub async fn serve(
        scope: Scope,
        queue: mpsc::Sender<Envelope>,
        state: watch::Receiver<Snapshot>,
    ) -> Result<(), AppError> {
        let path = endpoint(scope)?;
        if let Some(parent) = std::path::Path::new(&path).parent() {
            std::fs::create_dir_all(parent).map_err(|_| unavailable())?;
        }
        if std::path::Path::new(&path).exists() {
            if UnixStream::connect(&path).await.is_ok() {
                return Err(AppError::new(ErrorCode::Busy, "已有本地引擎"));
            }
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| unavailable())?;
            if metadata.uid() != unsafe { libc::geteuid() } {
                return Err(AppError::new(
                    ErrorCode::PermissionDenied,
                    "控制端点属于其他身份",
                ));
            }
            std::fs::remove_file(&path).map_err(|_| unavailable())?;
        }
        let listener = UnixListener::bind(&path).map_err(|_| unavailable())?;
        std::fs::set_permissions(
            &path,
            std::fs::Permissions::from_mode(if scope == Scope::System { 0o666 } else { 0o600 }),
        )
        .map_err(|_| unavailable())?;
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(24));
        loop {
            let (mut stream, _) = listener.accept().await.map_err(|_| unavailable())?;
            let Ok(permit) = semaphore.clone().try_acquire_owned() else {
                continue;
            };
            let (uid, _) = peer(&stream)?;
            let caller = Caller {
                administrator: uid == 0,
                owner: uid == unsafe { libc::geteuid() },
            };
            let queue = queue.clone();
            let state = state.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let _ = tokio::time::timeout(
                    Duration::from_secs(30),
                    dispatch(&mut stream, caller, queue, state),
                )
                .await;
            });
        }
    }
}

pub use native::endpoint;
fn namespace_suffix() -> String {
    use sha2::{Digest, Sha256};
    std::env::var_os("BITNET_DATA_DIR")
        .map(|p| {
            let hash = Sha256::digest(p.to_string_lossy().as_bytes());
            format!("-{:x}", hash)[..17].to_string()
        })
        .unwrap_or_default()
}
pub async fn request(scope: Scope, command: Command) -> Result<ReplyData, AppError> {
    tokio::time::timeout(
        Duration::from_secs(30),
        native::request(scope, &Request::new(command)),
    )
    .await
    .map_err(|_| AppError::new(ErrorCode::Busy, "本地操作超时；请先查询状态再决定是否重试"))?
}
pub async fn serve(
    scope: Scope,
    queue: mpsc::Sender<Envelope>,
    state: watch::Receiver<Snapshot>,
) -> Result<(), AppError> {
    native::serve(scope, queue, state).await
}
