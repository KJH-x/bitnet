use bitnet_core::Scope;
use bitnet_platform::{paths, prepare_private_directory, read_private_file, write_private_file};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::process::{Child, Command, Stdio};
use std::{
    env,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::{Duration, Instant},
};

mod server;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

const READY_TIMEOUT: Duration = Duration::from_secs(4);
const CONTROL_TIMEOUT: Duration = Duration::from_millis(500);
const LOOPBACK: Ipv4Addr = Ipv4Addr::LOCALHOST;
const IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const SESSION_FILE: &str = "session.json";
const BROWSER_FILE: &str = "open.html";
const TOKEN_LENGTH: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Start,
    Stop,
    Serve,
}

struct Options {
    scope: Scope,
    mode: Mode,
    no_open: bool,
    language: &'static str,
}

fn scope_name(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "user",
        Scope::System => "system",
    }
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Option<Options>> {
    let mut options = Options {
        scope: Scope::User,
        mode: Mode::Start,
        no_open: false,
        language: "zh",
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "Usage: bitnet-web [--scope user|system] [--lang zh|en] [--no-open | --stop]\n\
                     Starts the local BitNet history viewer and opens the browser.\n\
                     --no-open   Print the private session URL instead of opening it.\n\
                     --stop      Stop this scope's history viewer."
                );
                return Ok(None);
            }
            "--version" => {
                println!("bitnet-web {}", bitnet_core::APP_VERSION);
                return Ok(None);
            }
            "--scope" => {
                options.scope = match args.next().as_deref() {
                    Some("user") => Scope::User,
                    Some("system") => Scope::System,
                    _ => return Err("Expected --scope user or --scope system".into()),
                };
            }
            "--lang" => {
                options.language = match args.next().as_deref() {
                    Some("zh") => "zh",
                    Some("en") => "en",
                    _ => return Err("Expected --lang zh or --lang en".into()),
                };
            }
            "--stop" if options.mode == Mode::Start => options.mode = Mode::Stop,
            "--serve" if options.mode == Mode::Start => options.mode = Mode::Serve,
            "--no-open" => options.no_open = true,
            _ => return Err("Unknown or conflicting option. See --help.".into()),
        }
    }
    if options.no_open && options.mode != Mode::Start {
        return Err("--no-open requires start mode".into());
    }
    Ok(Some(options))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Session {
    port: u16,
    control: String,
    view: String,
}

impl Session {
    fn new(port: u16) -> Result<Self> {
        Ok(Self {
            port,
            control: token()?,
            view: token()?,
        })
    }

    fn read(dir: &Path) -> Result<Option<Self>> {
        let Some(data) = read_private_file(&dir.join(SESSION_FILE))? else {
            return Ok(None);
        };
        Ok(serde_json::from_slice::<Self>(&data)
            .ok()
            .filter(|session| {
                session.port > 1023 && valid_token(&session.view) && valid_token(&session.control)
            }))
    }

    fn address(&self) -> SocketAddrV4 {
        SocketAddrV4::new(LOOPBACK, self.port)
    }

    fn user_url(&self, language: &str) -> String {
        format!("http://{}/?lang={language}#{}", self.address(), self.view)
    }

    fn control(&self, stop: bool) -> io::Result<bool> {
        let (method, endpoint) = if stop {
            ("POST", "/control/stop")
        } else {
            ("GET", "/control/ready")
        };
        let mut stream = TcpStream::connect_timeout(&self.address().into(), CONTROL_TIMEOUT)?;
        stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
        stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
        write!(
            stream,
            "{method} {endpoint} HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\nContent-Length: 0\r\n\r\n",
            self.address(),
            self.control
        )?;
        let mut line = String::new();
        BufReader::new(stream).take(100).read_line(&mut line)?;
        Ok(line == "HTTP/1.1 204 No Content\r\n")
    }
}

fn valid_token(value: &str) -> bool {
    value.len() == TOKEN_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn token() -> Result<String> {
    let mut bytes = [0; TOKEN_LENGTH / 2];
    getrandom::getrandom(&mut bytes).map_err(|_| "Secure random generator unavailable")?;
    let mut output = String::with_capacity(TOKEN_LENGTH);
    for byte in bytes {
        write!(output, "{byte:02x}")?;
    }
    Ok(output)
}

fn lock(dir: &Path, name: &str) -> io::Result<Option<File>> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join(name))?;
    match FileExt::try_lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == fs2::lock_contended_error().kind() => Ok(None),
        Err(error) => Err(error),
    }
}

fn cleanup(dir: &Path) {
    let _ = fs::remove_file(dir.join(SESSION_FILE));
    let _ = fs::remove_file(dir.join(BROWSER_FILE));
}

fn wait_for_session(dir: &Path, mut child: Option<&mut Worker>) -> Result<Session> {
    let deadline = Instant::now() + READY_TIMEOUT;
    while Instant::now() < deadline {
        if let Some(process) = &mut child {
            if process.try_wait()?.is_some() {
                return Err("The history viewer could not start".into());
            }
        }
        if let Some(session) = Session::read(dir)? {
            if session.control(false).unwrap_or(false) {
                return Ok(session);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("The history viewer did not respond".into())
}

#[cfg(unix)]
type Worker = Child;

#[cfg(unix)]
fn launch_worker(scope: Scope) -> Result<Worker> {
    use std::os::unix::process::CommandExt;
    let mut child = Command::new(env::current_exe()?);
    child
        .args(["--scope", scope_name(scope), "--serve"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    Ok(child.spawn()?)
}

#[cfg(windows)]
struct Worker(std::os::windows::io::OwnedHandle);

#[cfg(windows)]
impl Worker {
    fn try_wait(&mut self) -> io::Result<Option<()>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::WaitForSingleObject,
        };
        // The owned handle stays open for the duration of the nonblocking wait.
        match unsafe { WaitForSingleObject(self.0.as_raw_handle(), 0) } {
            WAIT_OBJECT_0 => Ok(Some(())),
            WAIT_TIMEOUT => Ok(None),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

#[cfg(windows)]
fn launch_worker(scope: Scope) -> Result<Worker> {
    Ok(spawn_without_inherited_handles(
        &env::current_exe()?,
        &["--scope", scope_name(scope), "--serve"],
    )?)
}

#[cfg(windows)]
fn spawn_without_inherited_handles(executable: &Path, arguments: &[&str]) -> io::Result<Worker> {
    use std::{
        mem::size_of,
        os::windows::ffi::OsStrExt,
        os::windows::io::{FromRawHandle, OwnedHandle},
        ptr,
    };
    use windows_sys::Win32::System::Threading::{
        CreateProcessW, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
    };
    let application: Vec<_> = executable.as_os_str().encode_wide().chain([0]).collect();
    let mut line = vec![u16::from(b'"')];
    line.extend(&application[..application.len() - 1]);
    line.push(u16::from(b'"'));
    for argument in arguments {
        if argument.contains(['\0', '"']) || argument.ends_with('\\') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid launcher argument",
            ));
        }
        line.extend([u16::from(b' '), u16::from(b'"')]);
        line.extend(argument.encode_utf16());
        line.push(u16::from(b'"'));
    }
    line.push(0);
    let startup = STARTUPINFOW {
        cb: size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // The executable and command line are NUL-terminated and live through
    // the call. Disabling inheritance also excludes redirected caller pipes.
    let launched = unsafe {
        CreateProcessW(
            application.as_ptr(),
            line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            CREATE_NO_WINDOW,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if launched == 0 {
        return Err(io::Error::last_os_error());
    }
    // On success CreateProcessW returns two independently owned handles.
    let thread = unsafe { OwnedHandle::from_raw_handle(process.hThread) };
    drop(thread);
    Ok(Worker(unsafe {
        OwnedHandle::from_raw_handle(process.hProcess)
    }))
}

fn open_browser(dir: &Path, url: &str) -> Result<()> {
    // Only this private file contains the browser token. The browser launcher
    // receives a file URI, so process listings don't disclose the session URL.
    let path = dir.join(BROWSER_FILE);
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=referrer content=no-referrer>\
         <meta http-equiv=refresh content=\"0;url={url}\"><title>BitNet</title>\
         <a href=\"{url}\" rel=noreferrer>Open BitNet history</a>"
    );
    write_private_file(&path, html.as_bytes())?;
    let uri = url::Url::from_file_path(&path).map_err(|_| "Invalid browser launch file")?;
    #[cfg(windows)]
    let program =
        PathBuf::from(env::var_os("SystemRoot").ok_or("Windows system path unavailable")?)
            .join("explorer.exe");
    #[cfg(target_os = "macos")]
    let program = PathBuf::from("/usr/bin/open");
    #[cfg(all(unix, not(target_os = "macos")))]
    let program = PathBuf::from("xdg-open");
    #[cfg(windows)]
    spawn_without_inherited_handles(&program, &[uri.as_str()])?;
    #[cfg(unix)]
    Command::new(program)
        .arg(uri.as_str())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

fn start(dir: &Path, options: &Options) -> Result<()> {
    let session = match lock(dir, "worker.lock")? {
        None => wait_for_session(dir, None)?,
        Some(inactive) => {
            cleanup(dir);
            drop(inactive);
            let mut worker = launch_worker(options.scope)?;
            wait_for_session(dir, Some(&mut worker))?
        }
    };
    let url = session.user_url(options.language);
    if options.no_open {
        println!("{url}");
    } else {
        open_browser(dir, &url)?;
    }
    Ok(())
}

fn stop(dir: &Path) -> Result<()> {
    let deadline = Instant::now() + READY_TIMEOUT;
    let mut requested = false;
    while Instant::now() < deadline {
        if let Some(_inactive) = lock(dir, "worker.lock")? {
            cleanup(dir);
            return Ok(());
        }
        if !requested {
            if let Some(session) = Session::read(dir)? {
                requested = session.control(true).unwrap_or(false);
            }
        }
        thread::sleep(Duration::from_millis(25));
    }
    Err("The history viewer did not stop in time".into())
}

fn run() -> Result<()> {
    let Some(mut options) = parse(env::args().skip(1))? else {
        return Ok(());
    };
    if env::var_os("BITNET_SIMULATE_ONLY").as_deref() == Some(std::ffi::OsStr::new("1"))
        && (options.scope != Scope::User || env::var_os("BITNET_DATA_DIR").is_none())
    {
        return Err("Simulation requires user scope and a separate data directory".into());
    }
    if options.mode != Mode::Serve {
        options.scope = bitnet_runtime::client::Client::new(options.scope).effective_scope();
    }
    let dir = paths(Scope::User)?
        .data_dir
        .join("web")
        .join(scope_name(options.scope));
    if options.mode == Mode::Stop && !dir.exists() {
        return Ok(());
    }
    prepare_private_directory(&dir)?;
    if options.mode == Mode::Serve {
        let _lock = lock(&dir, "worker.lock")?.ok_or("The history viewer is already running")?;
        let result = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?
            .block_on(server::serve(&dir, options.scope));
        cleanup(&dir);
        return result;
    }
    let _lock = lock(&dir, "launcher.lock")?.ok_or("A viewer command is already running")?;
    if options.mode == Mode::Stop {
        stop(&dir)
    } else {
        start(&dir, &options)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("BitNet history viewer: {error}");
            ExitCode::FAILURE
        }
    }
}
