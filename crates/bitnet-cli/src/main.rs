use bitnet_core::*;
use bitnet_runtime::{
    client::Client,
    maintenance::{self, MaintenanceAction},
};
use clap::{Parser, Subcommand, ValueEnum};
use std::{
    io::{self, IsTerminal, Read, Write},
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(
    version,
    about = "BitNet 校园网管理：GUI 与 CLI 共用同一认证引擎",
    long_about = "BitNet 校园网管理。密码只能经隐藏输入或标准输入提交；命令行不接受密码参数。默认连接系统后台（如已安装），否则使用当前用户引擎。"
)]
struct Args {
    #[arg(long, global = true)]
    json: bool,
    #[arg(long, global = true, value_enum, default_value = "user")]
    scope: ScopeArg,
    #[arg(long, global = true)]
    non_interactive: bool,
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}
#[derive(Clone, Copy, ValueEnum)]
enum ScopeArg {
    User,
    System,
}
impl From<ScopeArg> for Scope {
    fn from(s: ScopeArg) -> Self {
        match s {
            ScopeArg::User => Scope::User,
            ScopeArg::System => Scope::System,
        }
    }
}
#[derive(Subcommand)]
enum Commands {
    /// 查看状态；不会在没有引擎时自行认证
    Status {
        #[arg(long)]
        watch: bool,
    },
    /// 启动当前用户引擎；模拟模式不会访问校园网
    Start {
        #[arg(long)]
        simulate: bool,
    },
    /// 停止引擎，不注销校园网
    Stop,
    Check,
    #[command(alias = "login")]
    Connect {
        #[arg(long)]
        account: Option<String>,
        #[arg(long)]
        force: bool,
        #[arg(long)]
        yes: bool,
    },
    #[command(alias = "logout")]
    Disconnect {
        #[arg(long)]
        yes: bool,
    },
    Pause,
    Resume,
    /// 允许当前网络自动认证；网络变化后可能需要再次确认
    TrustNetwork {
        #[arg(long)]
        remove: bool,
        #[arg(long)]
        yes: bool,
    },
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    Rule {
        #[command(subcommand)]
        action: RuleAction,
    },
    History {
        #[arg(long, default_value_t = 100)]
        limit: u32,
    },
    Statistics,
    Diagnose {
        #[arg(long)]
        output: Option<PathBuf>,
        /// 输出可直接粘贴到问题反馈中的脱敏 Markdown
        #[arg(long)]
        issue: bool,
    },
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    Update {
        #[command(subcommand)]
        action: UpdateAction,
    },
    /// 首次配置或修改账户：中文交互向导
    Configure,
    /// 主动导入第三方明文配置；不自动读取第三方文件
    Migrate {
        #[arg(long)]
        from: PathBuf,
        #[arg(long, default_value = "imported")]
        id: String,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        delete_source: bool,
    },
}
#[derive(Subcommand)]
enum ProfileAction {
    List,
    Save {
        #[arg(long)]
        id: String,
        #[arg(long)]
        name: String,
    },
    Select {
        id: String,
    },
    Remove {
        id: String,
    },
}
#[derive(Subcommand)]
enum AccountAction {
    List,
    Add {
        #[arg(long)]
        id: String,
        #[arg(long)]
        username: String,
        #[arg(long)]
        password_stdin: bool,
    },
    Remove {
        id: String,
        #[arg(long)]
        yes: bool,
    },
    Select {
        id: String,
    },
}
#[derive(Subcommand)]
enum ConfigAction {
    Get,
    Set { key: String, value: String },
    Export { file: PathBuf },
    Import { file: PathBuf },
}
#[derive(Subcommand)]
enum RuleAction {
    List,
    Put { file: PathBuf },
    Remove { id: String },
}
#[derive(Subcommand)]
enum ServiceAction {
    Status,
    Install,
    Start,
    Stop,
    Remove {
        #[arg(long)]
        purge: bool,
        #[arg(long)]
        yes: bool,
    },
    Repair,
}
#[derive(Subcommand)]
enum UpdateAction {
    Check,
    Install,
    Rollback {
        #[arg(long)]
        yes: bool,
    },
}

fn main() {
    let args = Args::parse();
    if let Some(ref path) = args.data_dir {
        if !path.is_absolute() || matches!(args.scope, ScopeArg::System) {
            eprintln!("--data-dir 必须为当前用户模式的绝对路径");
            std::process::exit(2);
        }
        std::env::set_var("BITNET_DATA_DIR", path);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    if let Err(error) = runtime.block_on(run(&args)) {
        if args.json {
            println!("{}", serde_json::json!({"ok":false,"error":error}));
        } else {
            eprintln!("{} ({:?})", error.message, error.code);
        }
        std::process::exit(error.code.exit_code());
    }
}

async fn run(args: &Args) -> Result<(), AppError> {
    let client = Client::new(args.scope.into());
    let Some(command) = &args.command else {
        return configure(&client, args).await;
    };
    match command {
        Commands::Status { watch } => loop {
            print_data(client.command(Command::Status).await?, args.json)?;
            if !watch {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        },
        Commands::Start { simulate } => {
            if *simulate {
                if args.data_dir.is_none() || matches!(args.scope, ScopeArg::System) {
                    return Err(input("模拟模式需要 --data-dir 指定独立的绝对路径"));
                }
                std::env::set_var("BITNET_SIMULATE_ONLY", "1");
                let mut process =
                    std::process::Command::new(bitnet_runtime::client::engine_path()?);
                process
                    .args(["--user", "--simulate"])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                #[cfg(windows)]
                {
                    use std::os::windows::process::CommandExt;
                    process.creation_flags(0x08000000);
                }
                process
                    .spawn()
                    .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法启动模拟引擎"))?;
                for _ in 0..50 {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    if client.status().await.is_ok() {
                        break;
                    }
                }
            } else {
                client.ensure_running().await?;
            }
            print_data(client.command(Command::Status).await?, args.json)?;
        }
        Commands::Service { action } => {
            if matches!(action, ServiceAction::Status) {
                let status = bitnet_platform::service::status();
                println!("{}", serde_json::to_string_pretty(&status).unwrap());
            } else {
                let action = match action {
                    ServiceAction::Install => MaintenanceAction::ServiceInstall,
                    ServiceAction::Start => MaintenanceAction::ServiceStart,
                    ServiceAction::Stop => MaintenanceAction::ServiceStop,
                    ServiceAction::Repair => MaintenanceAction::Repair,
                    ServiceAction::Remove { purge, yes } => {
                        confirm(*yes, args, "移除共享后台可能影响其他用户，是否继续？")?;
                        MaintenanceAction::ServiceRemove { purge: *purge }
                    }
                    _ => unreachable!(),
                };
                print_maintenance(
                    maintenance::execute(args.scope.into(), action, !args.non_interactive).await?,
                    args.json,
                )?;
            }
        }
        Commands::Update { action } => print_maintenance(
            maintenance::execute(
                args.scope.into(),
                match action {
                    UpdateAction::Check => MaintenanceAction::CheckUpdate,
                    UpdateAction::Install => MaintenanceAction::InstallUpdate,
                    UpdateAction::Rollback { yes } => {
                        confirm(
                            *yes,
                            args,
                            "回退到本机已验签缓存的前一版本？共享后台仍需系统授权。",
                        )?;
                        MaintenanceAction::RollbackUpdate
                    }
                },
                !args.non_interactive,
            )
            .await?,
            args.json,
        )?,
        Commands::Configure => configure(&client, args).await?,
        Commands::Migrate {
            from,
            id,
            yes,
            delete_source,
        } => {
            confirm(
                *yes,
                args,
                "导入将读取此文件中的明文密码并写入本软件安全存储，是否继续？",
            )?;
            let bytes = read_limited(from, 1024 * 1024)?;
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| input("无法解析来源配置"))?;
            let username = value
                .get("username")
                .or_else(|| value.get("user_name"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| input("来源文件缺少用户名"))?
                .to_string();
            let password = value
                .get("password")
                .and_then(|v| v.as_str())
                .ok_or_else(|| input("来源文件缺少密码"))?;
            client.ensure_running().await?;
            print_data(
                client
                    .command(Command::AccountPut {
                        id: id.clone(),
                        username,
                        password: Secret::new(password),
                    })
                    .await?,
                args.json,
            )?;
            if *delete_source {
                std::fs::remove_file(from)
                    .map_err(|_| input("账户已导入，但来源文件无法删除；请手动处理"))?;
            }
        }
        _ => {
            client.ensure_running().await?;
            let request = match command {
                Commands::Stop => Command::Shutdown,
                Commands::Check => Command::Check,
                Commands::Connect {
                    account,
                    force,
                    yes,
                } => {
                    let trusted = client.status().await?.network_trusted;
                    if *force {
                        confirm(*yes, args, "强制重新认证可能影响共享连接，是否继续？")?;
                    } else if !trusted {
                        confirm(
                            *yes,
                            args,
                            "此网络尚未确认，是否允许向当前网关提交本次校园网认证？",
                        )?;
                    }
                    Command::Login {
                        account_id: account.clone(),
                        force: *force,
                        confirmed: *yes || ((*force || !trusted) && !args.non_interactive),
                    }
                }
                Commands::Disconnect { yes } => {
                    confirm(
                        *yes,
                        args,
                        "注销会持续暂停自动认证，并可能影响共享设备，是否继续？",
                    )?;
                    Command::Logout { confirmed: true }
                }
                Commands::Pause => Command::Pause,
                Commands::Resume => Command::Resume,
                Commands::TrustNetwork { remove, yes } => {
                    if !remove {
                        confirm(
                            *yes,
                            args,
                            "确认此网络属于校园网连接，并允许此后自动提交认证？",
                        )?;
                    }
                    Command::TrustNetwork { trusted: !*remove }
                }
                Commands::Profile { action } => match action {
                    ProfileAction::List => {
                        let snapshot = client.status().await?;
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&snapshot.settings.profiles).unwrap()
                        );
                        return Ok(());
                    }
                    ProfileAction::Save { id, name } => Command::ProfileSave {
                        id: id.clone(),
                        name: name.clone(),
                    },
                    ProfileAction::Select { id } => Command::ProfileSelect { id: id.clone() },
                    ProfileAction::Remove { id } => Command::ProfileDelete { id: id.clone() },
                },
                Commands::Account { action } => match action {
                    AccountAction::List => Command::Status,
                    AccountAction::Add {
                        id,
                        username,
                        password_stdin,
                    } => Command::AccountPut {
                        id: id.clone(),
                        username: username.clone(),
                        password: read_password(*password_stdin, args)?,
                    },
                    AccountAction::Remove { id, yes } => {
                        confirm(*yes, args, "删除保存的账户？当前校园会话不会被注销。")?;
                        Command::AccountDelete { id: id.clone() }
                    }
                    AccountAction::Select { id } => Command::AccountSelect { id: id.clone() },
                },
                Commands::Config { action } => {
                    let snapshot = client.status().await?;
                    match action {
                        ConfigAction::Get => Command::ExportSettings,
                        ConfigAction::Export { file } => {
                            write_json(file, &snapshot.settings)?;
                            return Ok(());
                        }
                        ConfigAction::Import { file } => {
                            let settings: Settings =
                                serde_json::from_slice(&read_limited(file, 1024 * 1024)?)
                                    .map_err(|_| input("无效的设置文件"))?;
                            Command::SettingsPut {
                                settings,
                                expected_revision: snapshot.settings.revision,
                            }
                        }
                        ConfigAction::Set { key, value } => {
                            if [
                                "schema_version",
                                "revision",
                                "blocked_accounts",
                                "trusted_networks",
                                "paused",
                            ]
                            .contains(&key.as_str())
                            {
                                return Err(input("该字段由引擎管理，请使用对应管理操作"));
                            }
                            let mut settings =
                                serde_json::to_value(snapshot.settings.clone()).unwrap();
                            let object = settings.as_object_mut().unwrap();
                            if !object.contains_key(key) {
                                return Err(input("未知设置名称"));
                            }
                            object.insert(
                                key.clone(),
                                serde_json::from_str(value)
                                    .unwrap_or_else(|_| serde_json::Value::String(value.clone())),
                            );
                            let settings = serde_json::from_value(settings)
                                .map_err(|_| input("设置值类型不正确"))?;
                            Command::SettingsPut {
                                settings,
                                expected_revision: snapshot.settings.revision,
                            }
                        }
                    }
                }
                Commands::Rule { action } => {
                    let snapshot = client.status().await?;
                    let mut settings = snapshot.settings;
                    match action {
                        RuleAction::List => {
                            println!("{}", serde_json::to_string_pretty(&settings.rules).unwrap());
                            return Ok(());
                        }
                        RuleAction::Put { file } => {
                            let rule: NetworkRule =
                                serde_json::from_slice(&read_limited(file, 65536)?)
                                    .map_err(|_| input("无效网络规则"))?;
                            settings.rules.retain(|r| r.id != rule.id);
                            settings.rules.push(rule);
                        }
                        RuleAction::Remove { id } => settings.rules.retain(|r| &r.id != id),
                    }
                    let revision = settings.revision;
                    Command::SettingsPut {
                        settings,
                        expected_revision: revision,
                    }
                }
                Commands::History { limit } => Command::History { limit: *limit },
                Commands::Statistics => Command::Statistics,
                Commands::Diagnose { output, issue } => {
                    let result = client.command(Command::Diagnose).await?;
                    if *issue {
                        let ReplyData::Diagnostics(value) = result else {
                            return Err(AppError::new(ErrorCode::Internal, "诊断结果类型无效"));
                        };
                        let text=format!("## 问题描述\n\n请描述实际现象。\n\n## 重现步骤\n\n1. \n\n## 预期行为\n\n\n## 脱敏诊断\n\n```json\n{}\n```\n",serde_json::to_string_pretty(&value).map_err(|_|input("无法编码诊断报告"))?);
                        if let Some(path) = output {
                            bitnet_runtime::persistence::atomic_write(path, text.as_bytes())?;
                        } else {
                            println!("{text}");
                        }
                    } else if let Some(path) = output {
                        write_json(path, &result)?;
                    } else {
                        print_data(result, args.json)?;
                    }
                    return Ok(());
                }
                _ => unreachable!(),
            };
            let data = client.command(request).await?;
            if matches!(
                command,
                Commands::Account {
                    action: AccountAction::List
                }
            ) {
                if let ReplyData::Snapshot(snapshot) = data {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&snapshot.accounts).unwrap()
                    );
                }
            } else {
                print_data(data, args.json)?;
            }
        }
    }
    Ok(())
}

async fn configure(client: &Client, args: &Args) -> Result<(), AppError> {
    if args.non_interactive || !io::stdin().is_terminal() {
        return Err(input(
            "首次配置需要交互终端；自动部署请用 account add --password-stdin",
        ));
    }
    eprintln!("BitNet 首次配置 / Configure\n账号仅保存到系统保护的凭据存储；已有密码不会显示。");
    let username = prompt("校园网账号：")?;
    let password = read_password(false, args)?;
    client.ensure_running().await?;
    client
        .command(Command::AccountPut {
            id: "default".into(),
            username,
            password,
        })
        .await?;
    let mut snapshot = client.status().await?;
    snapshot.settings.automatic = prompt("启用自动认证？[y/N] ")?.eq_ignore_ascii_case("y");
    let revision = snapshot.settings.revision;
    let automatic = snapshot.settings.automatic;
    let result = client
        .command(Command::SettingsPut {
            settings: snapshot.settings,
            expected_revision: revision,
        })
        .await?;
    if automatic
        && prompt("确认当前网络为校园网连接，并允许以后自动认证？[y/N] ")?.eq_ignore_ascii_case("y")
    {
        print_data(
            client
                .command(Command::TrustNetwork { trusted: true })
                .await?,
            args.json,
        )
    } else {
        print_data(result, args.json)
    }
}
fn read_password(stdin: bool, args: &Args) -> Result<Secret, AppError> {
    if stdin {
        let mut bytes = Zeroizing::new(Vec::new());
        io::stdin()
            .take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| input("无法读取密码输入"))?;
        if bytes.len() > 4096 {
            return Err(input("密码输入过长"));
        }
        let mut text = String::from_utf8(bytes.to_vec()).map_err(|_| input("密码必须为 UTF-8"))?;
        while text.ends_with(['\n', '\r']) {
            text.pop();
        }
        Ok(Secret::new(text))
    } else {
        if args.non_interactive || !io::stdin().is_terminal() {
            return Err(input(
                "请使用交互终端或 --password-stdin，不接受命令行密码参数",
            ));
        }
        rpassword::prompt_password("校园网密码（不显示）：")
            .map(Secret::new)
            .map_err(|_| input("无法读取密码"))
    }
}
fn confirm(yes: bool, args: &Args, message: &str) -> Result<(), AppError> {
    if yes {
        return Ok(());
    }
    if args.non_interactive || !io::stdin().is_terminal() {
        return Err(AppError::new(
            ErrorCode::ConfirmationRequired,
            "此操作需要明确 --yes 确认",
        ));
    }
    if prompt(&format!("{message} [y/N] "))?.eq_ignore_ascii_case("y") {
        Ok(())
    } else {
        Err(AppError::new(ErrorCode::Cancelled, "用户取消操作"))
    }
}
fn prompt(message: &str) -> Result<String, AppError> {
    eprint!("{message}");
    io::stderr().flush().map_err(|_| input("无法显示提示"))?;
    let mut value = String::new();
    io::stdin()
        .read_line(&mut value)
        .map_err(|_| input("无法读取输入"))?;
    Ok(value.trim().to_string())
}
fn input(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, message)
}
fn read_limited(path: &PathBuf, limit: u64) -> Result<Zeroizing<Vec<u8>>, AppError> {
    let file = std::fs::File::open(path).map_err(|_| input("无法读取文件"))?;
    let mut bytes = Zeroizing::new(Vec::new());
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| input("无法读取文件"))?;
    if bytes.len() as u64 > limit {
        return Err(input("文件过大"));
    }
    Ok(bytes)
}
fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), AppError> {
    bitnet_runtime::persistence::atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|_| input("无法编码导出文件"))?,
    )
}
fn print_maintenance(value: maintenance::MaintenanceResult, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::json!({"ok":true,"message":value.message,"details":value.details})
        );
    } else {
        println!("{}", value.message);
    }
    Ok(())
}
fn print_data(value: ReplyData, json: bool) -> Result<(), AppError> {
    if json {
        println!("{}", serde_json::json!({"ok":true,"data":value}));
        return Ok(());
    }
    match value {
        ReplyData::Snapshot(s) => {
            println!("BitNet {} | {:?} | {:?}", s.app_version, s.scope, s.state);
            println!(
                "网络：{:?}  校园认证：{:?}  互联网：{:?}",
                s.environment, s.session.auth, s.health.internet
            );
            println!(
                "账户：{}  自动认证：{}  暂停：{}",
                s.session.username.as_deref().unwrap_or("未知"),
                s.settings.automatic,
                s.settings.paused || s.verification_hold
            );
            if let Some(e) = s.last_error {
                println!("提示：{}", e.message);
            }
            if let Some(next) = s.next_retry_at {
                println!("下次检查 Unix 时间：{next}");
            }
        }
        other => println!("{}", serde_json::to_string_pretty(&other).unwrap()),
    }
    Ok(())
}
