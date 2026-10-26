#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod notifications;
mod tray;
mod views;

use bitnet_core::{
    AccountInfo, AppError, AuthState, Command, EngineState, Environment, ErrorCode, Health,
    HistoryEvent, NetworkRule, ReplyData, Scope, Secret, Settings, Snapshot, Statistics,
    APP_VERSION,
};
use bitnet_runtime::client::Client;
use bitnet_runtime::maintenance::{self, MaintenanceAction, MaintenanceResult};
use bitnet_runtime::update_policy::{self, UpdateOutcome, UpdatePolicy};
use iced::{widget::text_editor, window, Element, Font, Subscription, Task, Theme};
use std::{fmt, time::Duration};
use zeroize::Zeroize;

fn main() -> iced::Result {
    iced::daemon(App::new, App::update, App::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(default_font())
        .run()
}

fn default_font() -> Font {
    #[cfg(target_os = "windows")]
    {
        Font::with_name("Microsoft YaHei UI")
    }
    #[cfg(target_os = "macos")]
    {
        Font::with_name("PingFang SC")
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Font::with_name("Noto Sans CJK SC")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Status,
    Accounts,
    Networks,
    Settings,
    Service,
    History,
    Statistics,
    Diagnostics,
    Updates,
}
impl Page {
    const ALL: [Self; 9] = [
        Self::Status,
        Self::Accounts,
        Self::Networks,
        Self::Settings,
        Self::Service,
        Self::History,
        Self::Statistics,
        Self::Diagnostics,
        Self::Updates,
    ];
    fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::Status, false) => "连接状态",
            (Self::Status, true) => "Connection",
            (Self::Accounts, false) => "账户管理",
            (Self::Accounts, true) => "Accounts",
            (Self::Networks, false) => "网络与规则",
            (Self::Networks, true) => "Networks & rules",
            (Self::Settings, false) => "偏好与设置",
            (Self::Settings, true) => "Preferences",
            (Self::Service, false) => "后台服务",
            (Self::Service, true) => "Background service",
            (Self::History, false) => "日志与历史",
            (Self::History, true) => "Activity & history",
            (Self::Statistics, false) => "运行统计",
            (Self::Statistics, true) => "Statistics",
            (Self::Diagnostics, false) => "网络诊断",
            (Self::Diagnostics, true) => "Diagnostics",
            (Self::Updates, false) => "软件更新",
            (Self::Updates, true) => "Updates",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Chinese,
    English,
}
impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Chinese => "简体中文",
            Self::English => "English",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Appearance {
    System,
    Light,
    Dark,
}
impl fmt::Display for Appearance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "跟随系统 / System",
            Self::Light => "浅色 / Light",
            Self::Dark => "深色 / Dark",
        })
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    User,
    System,
}
impl fmt::Display for Mode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::User => "自动选择 / Automatic",
            Self::System => "系统后台 / System service",
        })
    }
}
impl From<Mode> for Scope {
    fn from(value: Mode) -> Self {
        match value {
            Mode::User => Self::User,
            Mode::System => Self::System,
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SettingFlag {
    Automatic,
    StartHidden,
    UserAutostart,
    GuiAutostart,
    CheckUpdates,
    AutoUpdate,
    NotifyErrors,
}
#[derive(Debug, Clone, Copy)]
enum NumericSetting {
    StatusInterval,
    HealthInterval,
    RetryMaximum,
    RequestTimeout,
    LogRetention,
    HistoryRetention,
}

#[derive(Clone)]
enum Confirmation {
    LoginOnce,
    TrustNetwork,
    Logout,
    ForceLogin,
    DeleteAccount(String),
    DeleteRule(String),
    DeleteProfile(String),
    Shutdown,
    Maintenance(MaintenanceAction),
}

#[derive(Clone)]
enum Message {
    Tick,
    TrayTick,
    NotificationSent(Result<(), String>),
    UpdateChecked(Result<UpdateOutcome, AppError>),
    Connected(Result<Snapshot, AppError>),
    Refreshed(u64, Result<Snapshot, AppError>),
    Completed(Result<ReplyData, AppError>),
    MaintenanceCompleted(Result<MaintenanceResult, AppError>),
    Maintain(MaintenanceAction),
    Navigate(Page),
    ModeChanged(Mode),
    LanguageChanged(Language),
    AppearanceChanged(Appearance),
    LogLevelChanged(String),
    FlagChanged(SettingFlag, bool),
    NumberChanged(NumericSetting, String),
    HealthTargetsChanged(String),
    SaveSettings,
    RevertSettings,
    Run(Command),
    Ask(Confirmation),
    Confirm,
    Cancel,
    DismissNotice,
    Refresh,
    UsernameChanged(String),
    PasswordChanged(String),
    AccountEdit(String),
    AccountCancel,
    AccountSave,
    RuleNameChanged(String),
    ProfileIdChanged(String),
    ProfileNameChanged(String),
    ProfileSave,
    ProfileSelect(String),
    RuleInterfaceChanged(String),
    RuleSsidChanged(String),
    RuleGatewayChanged(String),
    RuleAccountChanged(String),
    RulePriorityChanged(String),
    RuleIgnoreChanged(bool),
    RuleEdit(usize),
    RuleSave,
    RuleCancel,
    InterfaceSelected(String),
    HistoryFilterChanged(String),
    HistoryDetailToggled(i64),
    ImportAction(text_editor::Action),
    ImportSettings,
    CopyExport,
    CopyDiagnostics,
    WindowOpened(window::Id),
    WindowCloseRequested(window::Id),
}

// Text-input messages may contain a freshly entered password. Never derive a
// Debug implementation that would expose their payloads to development logs.
impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PasswordChanged(_) => "PasswordChanged([REDACTED])",
            _ => "GuiMessage",
        })
    }
}

struct SettingsDraft {
    settings: Settings,
    status_interval: String,
    health_interval: String,
    retry_maximum: String,
    request_timeout: String,
    log_retention: String,
    history_retention: String,
    health_targets: String,
    dirty: bool,
}
impl SettingsDraft {
    fn new(settings: Settings) -> Self {
        Self {
            status_interval: settings.status_interval_secs.to_string(),
            health_interval: settings.health_interval_secs.to_string(),
            retry_maximum: settings.retry_max_secs.to_string(),
            request_timeout: settings.request_timeout_secs.to_string(),
            log_retention: settings.log_retention_days.to_string(),
            history_retention: settings.history_retention_days.to_string(),
            health_targets: settings.health_targets.join("; "),
            settings,
            dirty: false,
        }
    }
    fn validated(&self) -> Result<Settings, AppError> {
        fn number<T: std::str::FromStr>(s: &str) -> Result<T, AppError> {
            s.trim().parse().map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    "请输入有效的整数 / Enter a valid integer",
                )
            })
        }
        let mut settings = self.settings.clone();
        settings.status_interval_secs = number(&self.status_interval)?;
        settings.health_interval_secs = number(&self.health_interval)?;
        settings.retry_max_secs = number(&self.retry_maximum)?;
        settings.request_timeout_secs = number(&self.request_timeout)?;
        settings.log_retention_days = number(&self.log_retention)?;
        settings.history_retention_days = number(&self.history_retention)?;
        settings.health_targets = self
            .health_targets
            .split(['\n', ';'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        settings.validate()?;
        Ok(settings)
    }
}

#[derive(Default)]
struct AccountDraft {
    id: Option<String>,
    username: String,
    password: String,
}
impl Drop for AccountDraft {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}
#[derive(Default)]
struct RuleDraft {
    index: Option<usize>,
    name: String,
    interface_id: String,
    ssid: String,
    gateway: String,
    account_id: String,
    priority: String,
    ignore: bool,
}

struct App {
    client: Client,
    mode: Mode,
    snapshot: Option<Snapshot>,
    snapshot_current: bool,
    draft: SettingsDraft,
    page: Page,
    language: Language,
    appearance: Appearance,
    account: AccountDraft,
    rule: RuleDraft,
    profile_id: String,
    profile_name: String,
    history: Vec<HistoryEvent>,
    history_filter: String,
    expanded_history: Option<i64>,
    statistics: Option<Statistics>,
    diagnostics: String,
    exported: String,
    import_content: text_editor::Content,
    confirmation: Option<Confirmation>,
    notice: Option<(bool, String)>,
    connecting: bool,
    polling: bool,
    busy: bool,
    pending_settings: bool,
    settings_epoch: u64,
    saved_epoch: u64,
    request_epoch: u64,
    startup_hidden: bool,
    window: Option<window::Id>,
    tray: Option<tray::Tray>,
    notification_gate: notifications::Gate,
    pending_notification: Option<notifications::Kind>,
    update_policy: UpdatePolicy,
}

impl App {
    fn new() -> (Self, Task<Message>) {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mode = if args.iter().any(|s| s == "--system") {
            Mode::System
        } else {
            Mode::User
        };
        let client = Client::new(mode.into());
        let tray = tray::Tray::new().ok();
        let hide = args.iter().any(|s| s == "--hidden") && tray.is_some();
        let app = Self {
            client,
            mode,
            snapshot: None,
            snapshot_current: false,
            draft: SettingsDraft::new(Settings::default()),
            page: Page::Status,
            language: Language::Chinese,
            appearance: Appearance::System,
            account: AccountDraft::default(),
            rule: RuleDraft::default(),
            profile_id: String::new(),
            profile_name: String::new(),
            history: Vec::new(),
            history_filter: String::new(),
            expanded_history: None,
            statistics: None,
            diagnostics: String::new(),
            exported: String::new(),
            import_content: text_editor::Content::new(),
            confirmation: None,
            notice: None,
            connecting: true,
            polling: false,
            busy: false,
            pending_settings: false,
            settings_epoch: 0,
            saved_epoch: 0,
            request_epoch: 0,
            startup_hidden: hide,
            window: None,
            tray,
            notification_gate: notifications::Gate::default(),
            pending_notification: None,
            update_policy: UpdatePolicy::new(),
        };
        let connection = Self::connect(app.client.clone());
        (app, connection)
    }

    fn connect(client: Client) -> Task<Message> {
        Task::perform(
            async move {
                match client.status().await {
                    Ok(snapshot) => Ok(snapshot),
                    Err(_) => {
                        client.ensure_running().await?;
                        client.status().await
                    }
                }
            },
            Message::Connected,
        )
    }

    fn t(&self, zh: &'static str, en: &'static str) -> &'static str {
        if self.language == Language::English {
            en
        } else {
            zh
        }
    }
    fn english(&self) -> bool {
        self.language == Language::English
    }
    fn login_message(snapshot: &Snapshot) -> Message {
        if snapshot.network_trusted {
            Message::Run(Command::Login {
                account_id: None,
                force: false,
                confirmed: false,
            })
        } else {
            Message::Ask(Confirmation::LoginOnce)
        }
    }
    fn title(&self, _: window::Id) -> String {
        format!("BitNet · {}", self.page.label(self.english()))
    }
    fn theme(&self, _: window::Id) -> Option<Theme> {
        match self.appearance {
            Appearance::System => None,
            Appearance::Light => Some(Theme::Light),
            Appearance::Dark => Some(Theme::Dark),
        }
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick),
            iced::time::every(Duration::from_millis(250)).map(|_| Message::TrayTick),
            window::close_requests().map(Message::WindowCloseRequested),
        ])
    }
    fn open_window(&mut self) -> Task<Message> {
        if let Some(id) = self.window {
            return window::gain_focus(id);
        }
        let (id, task) = window::open(window::Settings {
            size: iced::Size::new(1120.0, 780.0),
            min_size: Some(iced::Size::new(840.0, 600.0)),
            exit_on_close_request: false,
            ..window::Settings::default()
        });
        self.window = Some(id);
        task.map(Message::WindowOpened)
    }
    fn apply_snapshot(&mut self, snapshot: Snapshot) {
        if let Some(kind) = self
            .notification_gate
            .observe(&snapshot, bitnet_core::now_secs())
        {
            self.pending_notification = Some(kind);
        }
        if !self.draft.dirty {
            self.draft = SettingsDraft::new(snapshot.settings.clone());
            self.language = if snapshot.settings.language == "en" {
                Language::English
            } else {
                Language::Chinese
            };
            self.appearance = match snapshot.settings.theme.as_str() {
                "light" => Appearance::Light,
                "dark" => Appearance::Dark,
                _ => Appearance::System,
            };
        }
        let english = self.english();
        if let Some(tray) = &mut self.tray {
            tray.update(&snapshot, english);
        }
        self.snapshot_current = true;
        self.snapshot = Some(snapshot);
    }
    fn can_start_service(&self) -> bool {
        !self.busy
            && !self.connecting
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.service.installed && (!self.snapshot_current || !snapshot.service.running)
            })
    }
    fn refresh(&mut self) -> Task<Message> {
        if self.polling || self.connecting {
            return Task::none();
        }
        self.polling = true;
        let epoch = self.request_epoch;
        Task::perform(
            {
                let client = self.client.clone();
                async move { client.status().await }
            },
            move |result| Message::Refreshed(epoch, result),
        )
    }
    fn execute(&mut self, command: Command) -> Task<Message> {
        if self.busy {
            return Task::none();
        }
        self.busy = true;
        self.request_epoch = self.request_epoch.wrapping_add(1);
        self.pending_settings = matches!(command, Command::SettingsPut { .. });
        if self.pending_settings {
            self.saved_epoch = self.settings_epoch;
        }
        self.notice = None;
        Task::perform(
            {
                let client = self.client.clone();
                async move { client.command(command).await }
            },
            Message::Completed,
        )
    }
    fn error(&mut self, error: AppError) {
        self.notice = Some((true, format!("{} · {:?}", error.message, error.code)));
    }
    fn mark_settings_dirty(&mut self) {
        self.draft.dirty = true;
        self.settings_epoch = self.settings_epoch.wrapping_add(1);
    }
    fn maintain(&mut self, action: MaintenanceAction) -> Task<Message> {
        if self.busy || self.connecting {
            return Task::none();
        }
        self.busy = true;
        self.request_epoch = self.request_epoch.wrapping_add(1);
        let scope = self
            .snapshot
            .as_ref()
            .map(|s| s.scope)
            .unwrap_or_else(|| self.mode.into());
        Task::perform(
            maintenance::execute(scope, action, true),
            Message::MaintenanceCompleted,
        )
    }
    fn save_settings(&mut self) -> Task<Message> {
        match self.draft.validated() {
            Ok(settings) => {
                // Edits belong to the revision they started from. A later status
                // poll must not silently turn a stale edit into a valid overwrite.
                let revision = self.draft.settings.revision;
                self.execute(Command::SettingsPut {
                    settings,
                    expected_revision: revision,
                })
            }
            Err(error) => {
                self.error(error);
                Task::none()
            }
        }
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::TrayTick => {
                if let Some(action) = tray::poll() {
                    return match action {
                        tray::Action::Open => self.open_window(),
                        tray::Action::Check => self.execute(Command::Check),
                        tray::Action::Pause => self.execute(Command::Pause),
                        tray::Action::Resume => self.execute(Command::Resume),
                        tray::Action::Exit => iced::exit(),
                    };
                }
                Task::none()
            }
            Message::Tick => {
                let status = self.refresh();
                let notification = self
                    .pending_notification
                    .take()
                    .map(|kind| {
                        Task::perform(
                            notifications::show(kind, self.english()),
                            Message::NotificationSent,
                        )
                    })
                    .unwrap_or_else(Task::none);
                let update = if !self.busy && !self.connecting {
                    self.snapshot
                        .as_ref()
                        .filter(|snapshot| {
                            self.update_policy
                                .due(&snapshot.settings, bitnet_core::now_secs())
                        })
                        .map(|snapshot| {
                            Task::perform(
                                update_policy::check(snapshot.scope, snapshot.settings.auto_update),
                                Message::UpdateChecked,
                            )
                        })
                        .unwrap_or_else(Task::none)
                } else {
                    Task::none()
                };
                Task::batch([status, notification, update])
            }
            Message::NotificationSent(result) => {
                if result.is_err() && self.notice.is_none() {
                    self.notice = Some((true, self.t("系统通知暂不可用，请在界面查看错误；系统通知设置未被更改。", "Desktop notifications are unavailable. Review errors here; system notification settings were not changed.").into()));
                }
                Task::none()
            }
            Message::UpdateChecked(result) => {
                self.update_policy
                    .complete(result.is_ok(), bitnet_core::now_secs());
                match result {
                    Ok(UpdateOutcome::Available { version }) => {
                        self.notice = Some((
                            false,
                            format!(
                                "{} {version}",
                                self.t(
                                    "发现新版本，请在软件更新页确认安装：",
                                    "Update available; review it in Updates:"
                                )
                            ),
                        ));
                        self.pending_notification
                            .get_or_insert(notifications::Kind::Update);
                    }
                    Ok(UpdateOutcome::InstallerStarted { message }) => {
                        self.notice = Some((false, message))
                    }
                    Ok(UpdateOutcome::NoUpdate | UpdateOutcome::Unconfigured) => {}
                    Err(error) => {
                        if self.notice.is_none() {
                            self.error(error);
                        }
                    }
                }
                Task::none()
            }
            Message::Connected(result) => {
                self.connecting = false;
                match result {
                    Ok(snapshot) => self.apply_snapshot(snapshot),
                    Err(error) => {
                        self.snapshot_current = false;
                        self.error(error);
                    }
                }
                let hidden = self.tray.is_some()
                    && (self.startup_hidden || self.draft.settings.start_hidden);
                if self.window.is_none() && !hidden {
                    self.open_window()
                } else {
                    Task::none()
                }
            }
            Message::Refreshed(epoch, result) => {
                self.polling = false;
                // A status request sent before a command cannot replace the
                // command's result even if its reply arrives afterwards.
                if epoch != self.request_epoch {
                    return Task::none();
                }
                match result {
                    Ok(snapshot) => self.apply_snapshot(snapshot),
                    Err(error) => {
                        self.snapshot_current = false;
                        self.error(error);
                    }
                }
                Task::none()
            }
            Message::Completed(result) => {
                self.busy = false;
                match result {
                    Ok(ReplyData::Snapshot(snapshot)) => {
                        if self.pending_settings && self.saved_epoch == self.settings_epoch {
                            self.draft.dirty = false;
                        }
                        self.apply_snapshot(*snapshot);
                        self.notice =
                            Some((false, self.t("操作已完成", "Operation completed").into()));
                    }
                    Ok(ReplyData::History(history)) => self.history = history,
                    Ok(ReplyData::Statistics(statistics)) => self.statistics = Some(statistics),
                    Ok(ReplyData::Diagnostics(value)) => {
                        self.diagnostics = serde_json::to_string_pretty(&value).unwrap_or_default()
                    }
                    Ok(ReplyData::Settings(settings)) => {
                        self.exported = serde_json::to_string_pretty(&settings).unwrap_or_default()
                    }
                    Ok(ReplyData::Ok { message }) => {
                        if self.pending_settings && self.saved_epoch == self.settings_epoch {
                            self.draft.dirty = false;
                        }
                        self.notice = Some((false, message));
                    }
                    Err(error) => self.error(error),
                }
                self.pending_settings = false;
                self.refresh()
            }
            Message::Navigate(page) => {
                self.page = page;
                match page {
                    Page::History => self.execute(Command::History { limit: 200 }),
                    Page::Statistics => self.execute(Command::Statistics),
                    _ => Task::none(),
                }
            }
            Message::Maintain(action) => self.maintain(action),
            Message::MaintenanceCompleted(result) => {
                self.busy = false;
                match result {
                    Ok(result) => self.notice = Some((false, result.message)),
                    Err(error) => self.error(error),
                }
                self.refresh()
            }
            Message::ModeChanged(mode) => {
                if self.busy || self.connecting {
                    return Task::none();
                }
                if self.draft.dirty || !self.account.password.is_empty() {
                    self.error(AppError::new(
                        ErrorCode::Busy,
                        self.t(
                            "请先保存或放弃当前修改",
                            "Save or discard your changes before switching modes",
                        ),
                    ));
                    return Task::none();
                }
                self.mode = mode;
                self.request_epoch = self.request_epoch.wrapping_add(1);
                self.client = Client::new(mode.into());
                self.snapshot = None;
                self.snapshot_current = false;
                self.draft = SettingsDraft::new(Settings::default());
                self.connecting = true;
                Self::connect(self.client.clone())
            }
            Message::Refresh => {
                // Maintenance may intentionally stop/replace the daemon. Polling
                // status is safe, but reconnect must not spawn a competing engine.
                if self.busy {
                    return Task::none();
                }
                if !self.snapshot_current && !self.connecting {
                    self.request_epoch = self.request_epoch.wrapping_add(1);
                    self.connecting = true;
                    Self::connect(self.client.clone())
                } else {
                    self.refresh()
                }
            }
            Message::Run(command) => self.execute(command),
            Message::Ask(confirmation) => {
                self.confirmation = Some(confirmation);
                self.open_window()
            }
            Message::Confirm => match self.confirmation.take() {
                Some(Confirmation::LoginOnce) => self.execute(Command::Login {
                    account_id: None,
                    force: false,
                    confirmed: true,
                }),
                Some(Confirmation::TrustNetwork) => {
                    self.execute(Command::TrustNetwork { trusted: true })
                }
                Some(Confirmation::Logout) => self.execute(Command::Logout { confirmed: true }),
                Some(Confirmation::ForceLogin) => self.execute(Command::Login {
                    account_id: None,
                    force: true,
                    confirmed: true,
                }),
                Some(Confirmation::DeleteAccount(id)) => {
                    self.execute(Command::AccountDelete { id })
                }
                Some(Confirmation::DeleteRule(id)) => {
                    self.draft.settings.rules.retain(|rule| rule.id != id);
                    self.mark_settings_dirty();
                    self.save_settings()
                }
                Some(Confirmation::DeleteProfile(id)) => {
                    self.execute(Command::ProfileDelete { id })
                }
                Some(Confirmation::Shutdown) => self.execute(Command::Shutdown),
                Some(Confirmation::Maintenance(action)) => self.maintain(action),
                None => Task::none(),
            },
            Message::Cancel => {
                self.confirmation = None;
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
            Message::UsernameChanged(value) => {
                self.account.username = value;
                Task::none()
            }
            Message::PasswordChanged(value) => {
                self.account.password.zeroize();
                self.account.password = value;
                Task::none()
            }
            Message::AccountEdit(id) => {
                if let Some(account) = self
                    .snapshot
                    .as_ref()
                    .and_then(|s| s.accounts.iter().find(|a| a.id == id))
                {
                    self.account = AccountDraft {
                        id: Some(account.id.clone()),
                        username: account.username.clone(),
                        password: String::new(),
                    };
                }
                Task::none()
            }
            Message::AccountCancel => {
                self.account = AccountDraft::default();
                Task::none()
            }
            Message::AccountSave => {
                let username = self.account.username.trim().to_owned();
                if username.is_empty() || self.account.password.is_empty() {
                    self.error(AppError::new(ErrorCode::InvalidInput, self.t("请输入账号和新密码；保存的密码不会回显", "Enter an account and a new password; saved passwords are never displayed")));
                    return Task::none();
                }
                let id = self
                    .account
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("account-{}", bitnet_core::now_ms()));
                let password = Secret::new(&self.account.password);
                self.account = AccountDraft::default();
                self.execute(Command::AccountPut {
                    id,
                    username,
                    password,
                })
            }
            Message::LanguageChanged(language) => {
                self.language = language;
                self.draft.settings.language = if language == Language::English {
                    "en"
                } else {
                    "zh-CN"
                }
                .into();
                self.mark_settings_dirty();
                Task::none()
            }
            Message::AppearanceChanged(appearance) => {
                self.appearance = appearance;
                self.draft.settings.theme = match appearance {
                    Appearance::System => "system",
                    Appearance::Light => "light",
                    Appearance::Dark => "dark",
                }
                .into();
                self.mark_settings_dirty();
                Task::none()
            }
            Message::LogLevelChanged(level) => {
                self.draft.settings.log_level = level;
                self.mark_settings_dirty();
                Task::none()
            }
            Message::FlagChanged(flag, value) => {
                match flag {
                    SettingFlag::Automatic => self.draft.settings.automatic = value,
                    SettingFlag::StartHidden => self.draft.settings.start_hidden = value,
                    SettingFlag::UserAutostart => self.draft.settings.user_autostart = value,
                    SettingFlag::GuiAutostart => self.draft.settings.gui_autostart = value,
                    SettingFlag::CheckUpdates => self.draft.settings.check_updates = value,
                    SettingFlag::AutoUpdate => self.draft.settings.auto_update = value,
                    SettingFlag::NotifyErrors => self.draft.settings.notify_errors = value,
                }
                self.mark_settings_dirty();
                Task::none()
            }
            Message::NumberChanged(field, value) => {
                match field {
                    NumericSetting::StatusInterval => self.draft.status_interval = value,
                    NumericSetting::HealthInterval => self.draft.health_interval = value,
                    NumericSetting::RetryMaximum => self.draft.retry_maximum = value,
                    NumericSetting::RequestTimeout => self.draft.request_timeout = value,
                    NumericSetting::LogRetention => self.draft.log_retention = value,
                    NumericSetting::HistoryRetention => self.draft.history_retention = value,
                }
                self.mark_settings_dirty();
                Task::none()
            }
            Message::HealthTargetsChanged(value) => {
                self.draft.health_targets = value;
                self.mark_settings_dirty();
                Task::none()
            }
            Message::SaveSettings => self.save_settings(),
            Message::RevertSettings => {
                if let Some(snapshot) = self.snapshot.clone() {
                    self.draft.dirty = false;
                    self.apply_snapshot(snapshot);
                }
                Task::none()
            }
            Message::InterfaceSelected(value) => {
                self.draft.settings.interface_id =
                    if value.is_empty() { None } else { Some(value) };
                self.mark_settings_dirty();
                self.save_settings()
            }
            Message::ProfileIdChanged(value) => {
                self.profile_id = value;
                Task::none()
            }
            Message::ProfileNameChanged(value) => {
                self.profile_name = value;
                Task::none()
            }
            Message::ProfileSave => {
                if self.draft.dirty {
                    self.error(AppError::new(
                        ErrorCode::Busy,
                        self.t(
                            "请先保存或放弃当前设置，再保存环境配置",
                            "Save or discard pending settings before capturing a profile",
                        ),
                    ));
                    return Task::none();
                }
                self.execute(Command::ProfileSave {
                    id: self.profile_id.trim().to_owned(),
                    name: self.profile_name.trim().to_owned(),
                })
            }
            Message::ProfileSelect(id) => {
                if self.draft.dirty {
                    self.error(AppError::new(
                        ErrorCode::Busy,
                        self.t(
                            "请先保存或放弃当前设置，再切换环境配置",
                            "Save or discard pending settings before switching profiles",
                        ),
                    ));
                    return Task::none();
                }
                self.execute(Command::ProfileSelect { id })
            }
            Message::RuleNameChanged(value) => {
                self.rule.name = value;
                Task::none()
            }
            Message::RuleInterfaceChanged(value) => {
                self.rule.interface_id = value;
                Task::none()
            }
            Message::RuleSsidChanged(value) => {
                self.rule.ssid = value;
                Task::none()
            }
            Message::RuleGatewayChanged(value) => {
                self.rule.gateway = value;
                Task::none()
            }
            Message::RuleAccountChanged(value) => {
                self.rule.account_id = value;
                Task::none()
            }
            Message::RulePriorityChanged(value) => {
                self.rule.priority = value;
                Task::none()
            }
            Message::RuleIgnoreChanged(value) => {
                self.rule.ignore = value;
                Task::none()
            }
            Message::RuleCancel => {
                self.rule = RuleDraft::default();
                Task::none()
            }
            Message::RuleEdit(index) => {
                if let Some(rule) = self.draft.settings.rules.get(index) {
                    self.rule = RuleDraft {
                        index: Some(index),
                        name: rule.name.clone(),
                        interface_id: rule.interface_id.clone().unwrap_or_default(),
                        ssid: rule.ssid.clone().unwrap_or_default(),
                        gateway: rule.gateway.map(|x| x.to_string()).unwrap_or_default(),
                        account_id: rule.account_id.clone().unwrap_or_default(),
                        priority: rule.priority.to_string(),
                        ignore: rule.ignore,
                    };
                    self.mark_settings_dirty();
                }
                Task::none()
            }
            Message::RuleSave => match self.validated_rule() {
                Ok(rule) => {
                    if let Some(index) = self.rule.index {
                        if let Some(existing) = self.draft.settings.rules.get_mut(index) {
                            *existing = rule;
                        }
                    } else {
                        self.draft.settings.rules.push(rule);
                    }
                    self.rule = RuleDraft::default();
                    self.mark_settings_dirty();
                    self.save_settings()
                }
                Err(error) => {
                    self.error(error);
                    Task::none()
                }
            },
            Message::HistoryFilterChanged(value) => {
                self.history_filter = value;
                Task::none()
            }
            Message::HistoryDetailToggled(id) => {
                self.expanded_history = if self.expanded_history == Some(id) {
                    None
                } else {
                    Some(id)
                };
                Task::none()
            }
            Message::ImportAction(action) => {
                self.import_content.perform(action);
                Task::none()
            }
            Message::ImportSettings => {
                match serde_json::from_str::<Settings>(&self.import_content.text()) {
                    Ok(mut settings) => {
                        if let Some(snapshot) = &self.snapshot {
                            // Import is a normal settings mutation; the daemon enforces
                            // administrative endpoint policy and revision checks too.
                            settings.revision = snapshot.settings.revision;
                        }
                        self.draft = SettingsDraft::new(settings);
                        self.mark_settings_dirty();
                        self.save_settings()
                    }
                    Err(_) => {
                        self.error(AppError::new(
                            ErrorCode::InvalidInput,
                            self.t(
                                "配置内容不是有效的 BitNet JSON",
                                "This is not a valid BitNet settings JSON document",
                            ),
                        ));
                        Task::none()
                    }
                }
            }
            Message::CopyExport => iced::clipboard::write(self.exported.clone()),
            Message::CopyDiagnostics => iced::clipboard::write(self.diagnostics.clone()),
            Message::WindowOpened(id) => {
                self.window = Some(id);
                Task::none()
            }
            Message::WindowCloseRequested(id) => {
                self.window = None;
                self.account = AccountDraft::default();
                if self.tray.is_some() {
                    window::close(id)
                } else {
                    iced::exit()
                }
            }
        }
    }

    fn validated_rule(&self) -> Result<NetworkRule, AppError> {
        let optional = |s: &str| {
            if s.trim().is_empty() {
                None
            } else {
                Some(s.trim().to_owned())
            }
        };
        if self.rule.name.trim().is_empty() {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                self.t("请填写规则名称", "Enter a rule name"),
            ));
        }
        let gateway = if self.rule.gateway.trim().is_empty() {
            None
        } else {
            Some(self.rule.gateway.trim().parse().map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    self.t(
                        "网关必须是有效的 IPv4 地址",
                        "Gateway must be a valid IPv4 address",
                    ),
                )
            })?)
        };
        let priority = if self.rule.priority.trim().is_empty() {
            0
        } else {
            self.rule.priority.trim().parse().map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    self.t("优先级必须是整数", "Priority must be an integer"),
                )
            })?
        };
        let account_id = optional(&self.rule.account_id);
        if let Some(id) = &account_id {
            if !self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.accounts.iter().any(|a| &a.id == id))
            {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    self.t(
                        "规则中的账户不存在",
                        "The rule refers to an unknown account",
                    ),
                ));
            }
        }
        Ok(NetworkRule {
            id: self
                .rule
                .index
                .and_then(|i| self.draft.settings.rules.get(i))
                .map(|r| r.id.clone())
                .unwrap_or_else(|| format!("rule-{}", bitnet_core::now_ms())),
            name: self.rule.name.trim().to_owned(),
            interface_id: optional(&self.rule.interface_id),
            ssid: optional(&self.rule.ssid),
            gateway,
            account_id,
            ignore: self.rule.ignore,
            priority,
        })
    }

    fn view(&self, _: window::Id) -> Element<'_, Message> {
        self.shell()
    }
    fn state_label(&self, state: &EngineState) -> &'static str {
        match state {
            EngineState::Initializing => self.t("正在初始化", "Starting"),
            EngineState::Checking => self.t("正在检查", "Checking"),
            EngineState::Idle => self.t("等待连接", "Idle"),
            EngineState::LoggingIn => self.t("正在认证", "Authenticating"),
            EngineState::Online => self.t("已联网", "Online"),
            EngineState::Retrying => self.t("等待重试", "Retry scheduled"),
            EngineState::Paused => self.t("自动认证已暂停", "Automatic authentication paused"),
            EngineState::NeedsCredentials => self.t("请添加账户", "Add an account"),
            EngineState::CredentialsUnavailable => {
                self.t("凭据暂不可用", "Credentials unavailable")
            }
            EngineState::AuthError => self.t("认证需要处理", "Authentication needs attention"),
            EngineState::NetworkDegraded => self.t("网络部分不可用", "Network degraded"),
            EngineState::Stopping => self.t("正在停止", "Stopping"),
        }
    }
    fn environment_label(&self, environment: &Environment) -> &'static str {
        match environment {
            Environment::Unknown => self.t("尚未确认", "Not confirmed"),
            Environment::NoNetwork => self.t("网络未连接", "No network"),
            Environment::NonCampus => self.t("非校园网络", "Outside campus"),
            Environment::Campus => self.t("校园直连", "Campus connection"),
            Environment::RoutedCampus => self.t("路由器共享网络", "Shared router connection"),
        }
    }
    fn health_label(&self, health: &Health) -> &'static str {
        match health {
            Health::Unknown => self.t("尚未检测", "Not checked"),
            Health::Healthy => self.t("正常", "Healthy"),
            Health::Degraded => self.t("部分异常", "Degraded"),
            Health::Unavailable => self.t("不可用", "Unavailable"),
        }
    }
}

fn account_label(account: &AccountInfo) -> String {
    format!("{}  ·  {}", account.username, account.id)
}
fn format_bytes(bytes: u64) -> String {
    const GIB: f64 = 1_073_741_824.0;
    if bytes >= 1_073_741_824 {
        format!("{:.2} GiB", bytes as f64 / GIB)
    } else {
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    }
}
fn format_duration(seconds: u64) -> String {
    format!("{} h {:02} min", seconds / 3600, (seconds / 60) % 60)
}
fn format_timestamp(timestamp: Option<u64>) -> String {
    timestamp
        .map(|at| {
            let elapsed = bitnet_core::now_secs().saturating_sub(at);
            if elapsed < 60 {
                format!("{elapsed} s ago / 秒前")
            } else if elapsed < 3600 {
                format!("{} min ago / 分钟前", elapsed / 60)
            } else if elapsed < 86400 {
                format!("{} h ago / 小时前", elapsed / 3600)
            } else {
                format!("{} d ago / 天前", elapsed / 86400)
            }
        })
        .unwrap_or_else(|| "—".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    // Construct state without App::new: no windows, tray, tasks, IPC or engine.
    fn fixture() -> App {
        App {
            client: Client::new(Scope::User),
            mode: Mode::User,
            snapshot: Some(Snapshot::new(Scope::User)),
            snapshot_current: true,
            draft: SettingsDraft::new(Settings::default()),
            page: Page::Status,
            language: Language::Chinese,
            appearance: Appearance::System,
            account: AccountDraft::default(),
            rule: RuleDraft::default(),
            profile_id: String::new(),
            profile_name: String::new(),
            history: Vec::new(),
            history_filter: String::new(),
            expanded_history: None,
            statistics: None,
            diagnostics: String::new(),
            exported: String::new(),
            import_content: text_editor::Content::new(),
            confirmation: None,
            notice: None,
            connecting: false,
            polling: false,
            busy: false,
            pending_settings: false,
            settings_epoch: 0,
            saved_epoch: 0,
            request_epoch: 0,
            startup_hidden: false,
            window: None,
            tray: None,
            notification_gate: notifications::Gate::default(),
            pending_notification: None,
            update_policy: UpdatePolicy::new_at(0),
        }
    }
    #[test]
    fn message_debug_never_exposes_password() {
        assert!(
            !format!("{:?}", Message::PasswordChanged("sensitive-fixture".into()))
                .contains("sensitive-fixture")
        );
        assert!(!format!(
            "{:?}",
            Message::Run(Command::AccountPut {
                id: "x".into(),
                username: "u".into(),
                password: Secret::new("sensitive-fixture")
            })
        )
        .contains("sensitive-fixture"));
    }
    #[test]
    fn settings_draft_uses_shared_validation() {
        let mut draft = SettingsDraft::new(Settings::default());
        assert!(draft.validated().is_ok());
        draft.status_interval = "0".into();
        assert!(draft.validated().is_err());
        draft.status_interval = "not-a-number".into();
        assert!(draft.validated().is_err());
    }
    #[test]
    fn remote_status_preserves_unsaved_settings_and_their_base_revision() {
        let mut app = fixture();
        app.draft.settings.revision = 7;
        let _ = app.update(Message::LanguageChanged(Language::English));
        let mut remote = Snapshot::new(Scope::User);
        remote.settings.revision = 8;
        remote.settings.paused = true;
        app.apply_snapshot(remote);
        assert_eq!(app.draft.settings.language, "en");
        assert_eq!(app.draft.settings.revision, 7);
        assert_eq!(app.snapshot.as_ref().unwrap().settings.revision, 8);
        assert!(app.draft.dirty);
    }
    #[test]
    fn a_save_reply_does_not_discard_edits_made_while_saving() {
        let mut app = fixture();
        app.pending_settings = true;
        app.busy = true;
        app.saved_epoch = 0;
        let _ = app.update(Message::FlagChanged(SettingFlag::Automatic, true));
        let _ = app.update(Message::Completed(Ok(ReplyData::Snapshot(Box::new(
            Snapshot::new(Scope::User),
        )))));
        assert!(app.draft.dirty);
        assert!(app.draft.settings.automatic);
        assert!(!app.busy);
    }
    #[test]
    fn status_sent_before_a_command_cannot_replace_its_result() {
        let mut app = fixture();
        app.snapshot.as_mut().unwrap().settings.paused = true;
        app.request_epoch = 2;
        app.polling = true;
        let _ = app.update(Message::Refreshed(1, Ok(Snapshot::new(Scope::User))));
        assert!(app.snapshot.as_ref().unwrap().settings.paused);
        assert!(!app.polling);
    }
    #[test]
    fn maintenance_and_engine_startup_never_overlap() {
        let mut app = fixture();
        app.snapshot = None;
        app.busy = true;
        let _ = app.update(Message::Refresh);
        assert!(
            !app.connecting,
            "Maintenance must not trigger engine startup"
        );
        app.busy = false;
        app.connecting = true;
        let _ = app.update(Message::Maintain(MaintenanceAction::ServiceInstall));
        assert!(
            !app.busy,
            "Maintenance waits until pending startup completes"
        );
    }
    #[test]
    fn an_untrusted_network_login_requires_explicit_one_time_confirmation() {
        let mut snapshot = Snapshot::new(Scope::User);
        assert!(matches!(
            App::login_message(&snapshot),
            Message::Ask(Confirmation::LoginOnce)
        ));
        snapshot.network_trusted = true;
        assert!(matches!(
            App::login_message(&snapshot),
            Message::Run(Command::Login {
                force: false,
                confirmed: false,
                ..
            })
        ));
    }
    #[test]
    fn an_unavailable_service_does_not_leave_start_disabled_by_its_old_snapshot() {
        let mut app = fixture();
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.service.installed = true;
        snapshot.service.running = true;
        assert!(!app.can_start_service());
        let _ = app.update(Message::Refreshed(
            0,
            Err(AppError::new(ErrorCode::Unavailable, "offline")),
        ));
        assert!(!app.snapshot_current);
        assert!(app.can_start_service());
        assert!(
            !app.connecting,
            "Failed periodic status must not spawn an engine"
        );
        let _ = app.update(Message::Refresh);
        assert!(
            app.connecting,
            "Explicit reconnect can recover from a stale snapshot"
        );
    }
    #[test]
    fn maintenance_blocks_explicit_reconnect_even_with_a_stale_snapshot() {
        let mut app = fixture();
        app.snapshot_current = false;
        app.busy = true;
        let _ = app.update(Message::Refresh);
        assert!(!app.connecting);
    }
}
