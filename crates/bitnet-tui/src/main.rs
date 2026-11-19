mod app;
mod ui;

use app::{App, Request};
use bitnet_core::Scope;
use bitnet_runtime::client::Client;
use clap::{Parser, ValueEnum};
use crossterm::{
    cursor::{Hide, Show},
    event::{Event, EventStream, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures_util::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::{
    io::{self, IsTerminal, Stdout},
    path::PathBuf,
    time::Duration,
};
use tokio::{task::JoinHandle, time::MissedTickBehavior};

#[derive(Parser)]
#[command(version, about = "BitNet 校园网终端界面")]
struct Args {
    /// 默认使用当前用户入口，已安装系统后台时连接系统后台
    #[arg(long, value_enum, default_value = "user")]
    scope: ScopeArg,
    #[arg(long)]
    data_dir: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum ScopeArg {
    User,
    System,
}

struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        restore_terminal();
    }
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
}

fn main() {
    let args = Args::parse();
    if let Err(error) = start(args) {
        eprintln!("bitnet-tui: {error}");
        std::process::exit(1);
    }
}

fn start(args: Args) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other("请在交互式终端中启动"));
    }
    if let Some(path) = args.data_dir {
        if !path.is_absolute() || matches!(args.scope, ScopeArg::System) {
            return Err(io::Error::other("--data-dir 仅支持用户模式的绝对路径"));
        }
        std::env::set_var("BITNET_DATA_DIR", path);
    }
    let scope = match args.scope {
        ScopeArg::User => Scope::User,
        ScopeArg::System => Scope::System,
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        restore_terminal();
        default_hook(info);
    }));
    enable_raw_mode()?;
    let _restore = RestoreTerminal;
    let mut output = io::stdout();
    execute!(output, EnterAlternateScreen, Hide)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(output))?;
    runtime.block_on(run(&mut terminal, Client::new(scope)))
}

struct Work {
    request: Request,
    task: JoinHandle<Result<bitnet_core::ReplyData, bitnet_core::AppError>>,
}

impl Work {
    fn start(client: &Client, request: Request) -> Self {
        let client = client.clone();
        let task = tokio::spawn(async move { request.send(&client).await });
        Self { request, task }
    }
}

async fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, client: Client) -> io::Result<()> {
    let mut app = App::new(client.scope);
    app.resize(terminal.size()?.height);
    let mut events = EventStream::new();
    let mut refresh = tokio::time::interval(Duration::from_secs(2));
    refresh.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut work = Some(Work::start(&client, Request::Status));
    let exit = exit_signal();
    tokio::pin!(exit);
    let mut redraw = true;

    while !app.exiting() {
        if redraw {
            terminal.draw(|frame| ui::draw(frame, &app, work.as_ref().map(|w| w.request)))?;
            redraw = false;
        }
        let mut next = None;
        tokio::select! {
            () = &mut exit => break,
            event = events.next() => match event {
                Some(Ok(Event::Key(key))) if key.kind == KeyEventKind::Press
                    || (key.kind == KeyEventKind::Repeat && app.can_repeat(key)) =>
                {
                    next = app.key(key, work.is_some());
                    redraw = true;
                }
                Some(Ok(Event::Resize(_, height))) => {
                    app.resize(height);
                    redraw = true;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => return Err(error),
                None => break,
            },
            _ = refresh.tick() => {
                if work.is_none() {
                    next = app.poll_request();
                }
            }
            outcome = async { (&mut work.as_mut().expect("active request").task).await }, if work.is_some() => {
                let request = work.take().expect("active request").request;
                let result = outcome.map_err(io::Error::other)?;
                redraw |= app.complete(request, result);
                next = app.pending_history();
            }
        }
        if let Some(request) = next {
            work = Some(Work::start(&client, request));
            redraw |= request != Request::Status;
        }
    }
    if let Some(work) = work {
        work.task.abort();
    }
    Ok(())
}

async fn exit_signal() {
    #[cfg(unix)]
    if let Ok(mut terminate) =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        return;
    }
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}
