//! Local Windows GUI activation. The pipe exposes no engine or settings commands.
use std::{
    io,
    os::windows::{
        ffi::OsStrExt,
        io::{AsRawHandle, FromRawHandle, OwnedHandle},
    },
    ptr, thread,
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, NamedPipeServer, ServerOptions},
    sync::{mpsc, oneshot},
    time::{sleep, timeout},
};
use windows_sys::Win32::{
    Foundation::{
        LocalFree, ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    Globalization::GetUserDefaultUILanguage,
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
        },
        GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER,
    },
    System::{
        Pipes::{
            GetNamedPipeClientSessionId, GetNamedPipeServerProcessId, GetNamedPipeServerSessionId,
        },
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            CreateMutexExW, GetCurrentProcess, OpenProcessToken, ReleaseMutex, WaitForSingleObject,
            MUTEX_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE,
        },
    },
    UI::WindowsAndMessaging::{AllowSetForegroundWindow, MessageBoxW, MB_ICONWARNING, MB_OK},
};

pub(crate) type Show = mpsc::Sender<()>;
const DEADLINE: Duration = Duration::from_millis(1500);
const RETRY: Duration = Duration::from_millis(35);
const MAX_LINE: usize = 64;
const SHOW: &[u8] = b"SHOW\n";
const PING: &[u8] = b"PING\n";
const PONG: &[u8] = b"PONG\n";

pub(crate) enum Launch {
    Primary(Primary, mpsc::Receiver<Show>),
    Existing,
}

pub(crate) struct Primary {
    stop: Option<oneshot::Sender<()>>,
    worker: Option<thread::JoinHandle<()>>,
    _mutex: MutexOwner,
}

impl Drop for Primary {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

// Mutex ownership belongs to the main thread for the lifetime of iced::run.
struct MutexOwner(OwnedHandle);

impl Drop for MutexOwner {
    fn drop(&mut self) {
        unsafe { ReleaseMutex(self.0.as_raw_handle()) };
    }
}

struct Names {
    mutex: Vec<u16>,
    pipe: String,
    sid: String,
    session: u32,
}

impl Names {
    fn current(preview: bool) -> io::Result<Self> {
        use sha2::{Digest, Sha256};

        let sid = current_sid()?;
        let mut session = 0;
        if unsafe { ProcessIdToSessionId(std::process::id(), &mut session) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut tag = String::new();
        if let Some(directory) = std::env::var_os("BITNET_DATA_DIR") {
            let identity: Vec<u8> = directory.encode_wide().flat_map(u16::to_le_bytes).collect();
            let hash = format!("{:x}", Sha256::digest(identity));
            tag = format!("-{}", &hash[..16]);
        }
        if preview {
            tag.push_str("-preview");
        }
        Ok(Self {
            mutex: wide(&format!("Local\\BitNetGui-v1-{sid}{tag}")),
            pipe: format!(r"\\.\pipe\BitNetGui-v1-{session}-{sid}{tag}"),
            sid,
            session,
        })
    }

    fn create_pipe(&self) -> io::Result<NamedPipeServer> {
        let security = Descriptor::for_user(&self.sid)?;
        let mut attributes = security.attributes();
        // A single reusable instance plus first_pipe_instance prevents another
        // object from silently sharing the activation endpoint.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .max_instances(1)
                .in_buffer_size(MAX_LINE as u32)
                .out_buffer_size(MAX_LINE as u32)
                .create_with_security_attributes_raw(
                    &self.pipe,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                )
        }
    }
}

pub(crate) fn acquire(show_existing: bool, preview: bool) -> io::Result<Launch> {
    let names = Names::current(preview)?;
    let security = Descriptor::for_user(&names.sid)?;
    let raw = unsafe {
        CreateMutexExW(
            &security.attributes(),
            names.mutex.as_ptr(),
            0,
            SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
        )
    };
    if raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    match unsafe { WaitForSingleObject(handle.as_raw_handle(), 0) } {
        WAIT_OBJECT_0 | WAIT_ABANDONED => start_server(names, MutexOwner(handle)),
        WAIT_TIMEOUT => {
            runtime()?
                .block_on(async { timeout(DEADLINE, notify(&names, show_existing)).await? })?;
            Ok(Launch::Existing)
        }
        _ => Err(io::Error::last_os_error()),
    }
}

fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

fn start_server(names: Names, mutex: MutexOwner) -> io::Result<Launch> {
    let (ready, readiness) = std::sync::mpsc::sync_channel(1);
    let (stop, quit) = oneshot::channel();
    let (show, inbox) = mpsc::channel(1);
    let worker = thread::Builder::new()
        .name("bitnet-activation".into())
        .spawn(move || {
            let runtime = match runtime() {
                Ok(runtime) => runtime,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            runtime.block_on(async {
                let pipe = match names.create_pipe() {
                    Ok(pipe) => pipe,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                if ready.send(Ok(())).is_ok() {
                    tokio::select! {
                        _ = quit => {}
                        _ = serve(pipe, &show, names.session) => {}
                    }
                }
            });
        })?;
    if let Err(error) = readiness.recv().unwrap_or_else(|_| {
        Err(io::Error::other(
            "BitNet GUI activation listener did not start",
        ))
    }) {
        let _ = worker.join();
        return Err(error);
    }
    Ok(Launch::Primary(
        Primary {
            stop: Some(stop),
            worker: Some(worker),
            _mutex: mutex,
        },
        inbox,
    ))
}

async fn serve(
    mut pipe: NamedPipeServer,
    show: &mpsc::Sender<Show>,
    session: u32,
) -> io::Result<()> {
    loop {
        pipe.connect().await?;
        let _ = timeout(DEADLINE, answer(&mut pipe, show, session)).await;
        pipe.disconnect()?;
    }
}

async fn answer(
    pipe: &mut NamedPipeServer,
    show: &mpsc::Sender<Show>,
    session: u32,
) -> io::Result<()> {
    let mut peer_session = 0;
    if unsafe { GetNamedPipeClientSessionId(pipe.as_raw_handle(), &mut peer_session) } == 0
        || peer_session != session
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    let response = match read_line(pipe).await?.as_slice() {
        PING => PONG.to_vec(),
        SHOW => {
            let (ack, mut acknowledgement) = mpsc::channel(1);
            show.send(ack)
                .await
                .map_err(|_| io::ErrorKind::BrokenPipe)?;
            acknowledgement
                .recv()
                .await
                .ok_or(io::ErrorKind::BrokenPipe)?;
            ack_line(std::process::id()).into_bytes()
        }
        _ => return Err(io::ErrorKind::InvalidData.into()),
    };
    pipe.write_all(&response).await?;
    // DisconnectNamedPipe discards unread data, so let the requesting process
    // consume the ACK before reusing this instance. The outer deadline applies.
    let _ = pipe.read_u8().await;
    Ok(())
}

async fn notify(names: &Names, show: bool) -> io::Result<()> {
    let mut pipe = loop {
        match ClientOptions::new().open(&names.pipe) {
            Ok(pipe) => break pipe,
            Err(error)
                if matches!(
                    error.raw_os_error().map(|code| code as u32),
                    Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY)
                ) =>
            {
                sleep(RETRY).await;
            }
            Err(error) => return Err(error),
        }
    };
    let mut peer_session = 0;
    let mut pid = 0;
    if unsafe { GetNamedPipeServerSessionId(pipe.as_raw_handle(), &mut peer_session) } == 0
        || peer_session != names.session
        || unsafe { GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) } == 0
        || pid == 0
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if show {
        // A launcher with foreground permission can give it to the original
        // process. Windows may decline; the original then flashes its taskbar button.
        unsafe { AllowSetForegroundWindow(pid) };
    }
    pipe.write_all(if show { SHOW } else { PING }).await?;
    let reply = read_line(&mut pipe).await?;
    let valid = if show {
        reply == ack_line(pid).as_bytes()
    } else {
        reply == PONG
    };
    if valid {
        Ok(())
    } else {
        Err(io::ErrorKind::InvalidData.into())
    }
}

async fn read_line(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_LINE);
    for _ in 0..MAX_LINE {
        let byte = reader.read_u8().await?;
        bytes.push(byte);
        if byte == b'\n' {
            return Ok(bytes);
        }
    }
    Err(io::ErrorKind::InvalidData.into())
}

fn ack_line(pid: u32) -> String {
    format!("ACK {pid}\n")
}

struct Descriptor(PSECURITY_DESCRIPTOR);

impl Descriptor {
    fn for_user(sid: &str) -> io::Result<Self> {
        let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
        let mut raw = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut raw,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Self(raw))
    }

    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}

impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe { LocalFree(self.0) };
    }
}

fn current_sid() -> io::Result<String> {
    let mut token: HANDLE = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut needed = 0;
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut needed,
        )
    };
    if needed == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut words = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            words.as_mut_ptr().cast(),
            needed,
            &mut needed,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    let user = unsafe { &*words.as_ptr().cast::<TOKEN_USER>() };
    let mut sid = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut sid) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut len = 0;
    while unsafe { *sid.add(len) } != 0 {
        len += 1;
    }
    let value = String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid, len) });
    unsafe { LocalFree(sid.cast()) };
    Ok(value)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

pub(crate) fn show_startup_error(error: &io::Error) {
    let chinese = unsafe { GetUserDefaultUILanguage() } & 0x03ff == 4;
    let label = if error.kind() == io::ErrorKind::TimedOut {
        if chinese {
            "BitNet 窗口没有响应。请从托盘退出 BitNet 后再打开。"
        } else {
            "The BitNet window did not respond. Quit BitNet from the tray, then open it again."
        }
    } else if chinese {
        "无法打开 BitNet 窗口。请先从托盘退出已运行的 BitNet，再试一次。"
    } else {
        "Could not open the BitNet window. Quit any running copy from the tray and try again."
    };
    unsafe {
        MessageBoxW(
            ptr::null_mut(),
            wide(label).as_ptr(),
            wide("BitNet").as_ptr(),
            MB_OK | MB_ICONWARNING,
        )
    };
}
