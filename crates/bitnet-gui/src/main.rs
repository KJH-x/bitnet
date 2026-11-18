#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod design;
mod notifications;
mod tray;
mod views;

use bitnet_core::{
    AccountInfo, AppError, AuthState, Command, EngineState, Environment, ErrorCode, NetworkRule,
    ReplyData, Scope, Secret, Settings, Snapshot, Statistics, APP_VERSION,
};
use bitnet_core::{HealthExpect, HealthTarget as ConfigHealthTarget, MAX_HEALTH_TARGETS};
use bitnet_launcher::{LaunchError, WebAction};
use bitnet_runtime::client::Client;
use bitnet_runtime::maintenance::{self, InterfaceTarget, MaintenanceAction, MaintenanceResult};
use bitnet_runtime::update_policy::{self, UpdateOutcome, UpdatePolicy};
use iced::{widget::text_editor, window, Element, Font, Subscription, Task, Theme};
use std::{
    fmt,
    time::{Duration, Instant},
};
use zeroize::Zeroize;

fn main() -> iced::Result {
    iced::daemon(App::new, App::update, App::view)
        .settings(iced::Settings {
            font: design::family(),
            text_size: iced::Pixels(design::get().typography.body),
            line_height: iced::widget::text::LineHeight::Relative(
                design::get().typography.line_height,
            ),
            ..iced::Settings::default()
        })
        .title(App::title)
        .theme(App::theme)
        .style(|_, theme| design::app_style(theme))
        .subscription(App::subscription)
        .font(include_bytes!("../assets/fonts/BitNetHanSansSC-Regular-3755.otf").as_slice())
        .font(include_bytes!("../assets/fonts/BitNetHanSansSC-Medium-3755.otf").as_slice())
        .run()
}

fn preview_only() -> bool {
    std::env::var("BITNET_SIMULATE_ONLY").as_deref() == Ok("1")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Status,
    Accounts,
    Networks,
    Settings,
    Service,
    Health,
    Statistics,
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkSection {
    Adapters,
    Profiles,
    Rules,
    RuleEditor,
}
impl Page {
    /// Sub-tab label inside the connection area; never equal to the area name.
    fn tab_label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::Status, false) => "状态",
            (Self::Status, true) => "Status",
            _ => self.label(english),
        }
    }

    const ALL: [Self; 5] = [
        Self::Status,
        Self::Accounts,
        Self::Networks,
        Self::Service,
        Self::Settings,
    ];
    fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::Status, false) => "连接",
            (Self::Status, true) => "Connection",
            (Self::Accounts, false) => "账户",
            (Self::Accounts, true) => "Accounts",
            (Self::Networks, false) => "网络",
            (Self::Networks, true) => "Network",
            (Self::Settings, false) => "软件设置",
            (Self::Settings, true) => "Settings",
            (Self::Service, false) => "后台服务",
            (Self::Service, true) => "Server Mode",
            (Self::Health, false) => "详情",
            (Self::Health, true) => "Details",
            (Self::Statistics, false) => "统计",
            (Self::Statistics, true) => "Stats",
            (Self::Diagnostics, false) => "诊断",
            (Self::Diagnostics, true) => "Diagnostics",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Chinese,
    English,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    General,
    Checks,
    Trust,
    Diagnostics,
    Updates,
    Transfer,
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
impl Appearance {
    fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::System, false) => "跟随系统",
            (Self::System, true) => "System",
            (Self::Light, false) => "浅色",
            (Self::Light, true) => "Light",
            (Self::Dark, false) => "深色",
            (Self::Dark, true) => "Dark",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    User,
    System,
}
impl Mode {
    fn label(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::User, false) => "自动选择",
            (Self::User, true) => "Automatic",
            (Self::System, false) => "系统后台",
            (Self::System, true) => "System service",
        }
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
    TemporaryLogin {
        expected_network: String,
        username: String,
    },
    EnableAutomatic {
        expected_network: String,
    },
    SwitchAccount(SavedAccountTarget),
    Logout,
    ForceLogin,
    DeleteAccount(String),
    DeleteRule(String),
    DeleteProfile(String),
    Shutdown,
    Maintenance(MaintenanceAction),
    ClearLogs {
        scope: Scope,
        bytes: u64,
    },
}

#[derive(Clone)]
struct NetworkPrompt {
    fingerprint: String,
    label: String,
}

#[derive(Clone)]
struct SavedAccountTarget {
    id: String,
    username: String,
    network: String,
    scope: Scope,
    revision: u64,
}

impl SavedAccountTarget {
    fn capture(snapshot: &Snapshot, id: &str) -> Option<Self> {
        if !snapshot
            .capabilities
            .iter()
            .any(|item| item == "switch_account")
            || bitnet_runtime::engine::uses_system_authentication(snapshot)
        {
            return None;
        }
        let account = snapshot.accounts.iter().find(|account| account.id == id)?;
        account.has_password.then_some(Self {
            id: id.to_owned(),
            username: account.username.clone(),
            network: confirmation_target(snapshot)?.0,
            scope: snapshot.scope,
            revision: snapshot.settings.revision,
        })
    }

    fn matches(&self, snapshot: &Snapshot) -> bool {
        Self::capture(snapshot, &self.id).is_some_and(|current| {
            current.network == self.network
                && current.scope == self.scope
                && current.revision == self.revision
                && current.username == self.username
        })
    }
}

fn is_alternate_adapter(interface: &bitnet_core::NetworkInterface) -> bool {
    let name = interface.name.to_ascii_lowercase();
    interface.virtual_adapter
        || interface.alternate_adapter
        || [
            "bluetooth",
            "蓝牙",
            "loopback",
            "mihomo",
            "wi-fi direct",
            "local proxy",
            "vmware",
            "virtualbox",
            "vethernet",
            "hyper-v",
            "tailscale",
            "wireguard",
        ]
        .iter()
        .any(|kind| name.contains(kind))
        || name.starts_with("本地连接*")
        || name.starts_with("local area connection*")
}

fn selected_binding(snapshot: &Snapshot) -> Option<&bitnet_core::NetworkBinding> {
    snapshot.active_binding.as_ref().filter(|binding| {
        snapshot
            .settings
            .interface_id
            .as_ref()
            .is_none_or(|id| *id == binding.interface_id)
    })
}

fn ipv6_adapter(snapshot: &Snapshot) -> Option<&bitnet_core::NetworkInterface> {
    let binding = selected_binding(snapshot)?;
    snapshot.interfaces.iter().find(|interface| {
        interface.id == binding.interface_id && interface.index == binding.interface_index
    })
}

fn ipv6_maintenance_target(snapshot: &Snapshot) -> Option<InterfaceTarget> {
    if !cfg!(windows)
        || snapshot
            .capabilities
            .iter()
            .any(|item| item == "simulation")
    {
        return None;
    }
    InterfaceTarget::from_interface(ipv6_adapter(snapshot)?).ok()
}

fn confirmation_target(snapshot: &Snapshot) -> Option<(String, String)> {
    let binding = selected_binding(snapshot)?;
    let adapter = snapshot
        .interfaces
        .iter()
        .find(|adapter| adapter.id == binding.interface_id)?;
    let label = match &adapter.ssid {
        Some(ssid) => format!("{} · {ssid}", adapter.name),
        None => adapter.name.clone(),
    };
    Some((
        bitnet_runtime::engine::network_fingerprint(snapshot)?,
        label,
    ))
}

#[derive(Clone)]
enum Message {
    Tick,
    TrayTick,
    #[cfg(windows)]
    TrayToggleWindow(window::Id, bool),
    NotificationSent(Result<(), String>),
    CheckUpdates,
    UpdateChecked(Result<UpdateOutcome, AppError>, bool),
    OpenReleasePage,
    ReleasePageOpened(bool),
    Connected(Result<Snapshot, AppError>),
    Refreshed(u64, Result<Snapshot, AppError>),
    Completed(Result<ReplyData, AppError>),
    MaintenanceCompleted(Result<MaintenanceResult, AppError>),
    Navigate(Page),
    ModeChanged(Mode),
    LanguageChanged(Language),
    AppearanceChanged(Appearance),
    LogLevelChanged(String),
    FlagChanged(SettingFlag, bool),
    AutomaticChanged(bool),
    BeginAutomatic,
    NumberChanged(NumericSetting, String),
    SettingsTabChanged(SettingsTab),
    OpenHealthEditor,
    HealthEntryChanged(usize, String),
    HealthEntryExpectChanged(usize, HealthCondition),
    HealthEntryValueChanged(usize, String),
    HealthEntryAdded,
    HealthEntryRemoved(usize),
    SaveHealthEditor,
    CancelHealthEditor,
    ToggleAlternateAdapters,
    NetworkBack,
    NetworkTab(NetworkSection),
    NetworkPage(usize),
    BeginRule,
    SaveSettings,
    RevertSettings,
    Run(Command),
    Ask(Confirmation),
    Confirm,
    Cancel,
    Escape,
    KeyboardEnter,
    MoveFocus(bool),
    DismissNotice,
    Refresh,
    UsernameChanged(String),
    PasswordChanged(String),
    AccountAdd,
    AccountPage(usize),
    AccountCancel,
    AccountSave(String),
    SwitchAccount(SavedAccountTarget),
    OpenQuickLogin,
    TemporaryLogin,
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
    InterfaceSelected(Option<(String, Option<String>)>),
    NetworkDecision {
        fingerprint: String,
        allow: Option<bool>,
    },
    TrustPage(usize),
    ImportAction(text_editor::Action),
    ImportSettings,
    CopyExport,
    CopyDiagnostics,
    WebLogs(WebAction),
    WebLogsFinished(WebAction, Result<(), LaunchError>),
    WindowOpened(window::Id),
    WindowCloseRequested(window::Id),
    TogglePassword,
    ToggleIps,
    RevealTick,
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
            settings,
            dirty: false,
        }
    }
    fn validated(&self, english: bool) -> Result<Settings, AppError> {
        fn number<T: std::str::FromStr>(s: &str, english: bool) -> Result<T, AppError> {
            s.trim().parse().map_err(|_| {
                AppError::new(
                    ErrorCode::InvalidInput,
                    if english {
                        "Enter a whole number"
                    } else {
                        "请输入整数"
                    },
                )
            })
        }
        let mut settings = self.settings.clone();
        settings.status_interval_secs = number(&self.status_interval, english)?;
        settings.health_interval_secs = number(&self.health_interval, english)?;
        settings.retry_max_secs = number(&self.retry_maximum, english)?;
        settings.request_timeout_secs = number(&self.request_timeout, english)?;
        settings.log_retention_days = number(&self.log_retention, english)?;
        settings.history_retention_days = number(&self.history_retention, english)?;
        settings.validate()?;
        Ok(settings)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum HealthCondition {
    #[default]
    AnySuccess,
    Status,
    BodyEquals,
    BodyContains,
}

#[derive(Clone, Default)]
struct HealthTarget {
    url: String,
    condition: HealthCondition,
    value: String,
}

impl From<ConfigHealthTarget> for HealthTarget {
    fn from(target: ConfigHealthTarget) -> Self {
        let (condition, value) = match target.expect {
            HealthExpect::AnySuccess => (HealthCondition::AnySuccess, String::new()),
            HealthExpect::Status(code) => (HealthCondition::Status, code.to_string()),
            HealthExpect::BodyEquals(body) => (HealthCondition::BodyEquals, body),
            HealthExpect::BodyContains(body) => (HealthCondition::BodyContains, body),
        };
        Self {
            url: target.url,
            condition,
            value,
        }
    }
}

#[derive(Default)]
struct AccountDraft {
    id: Option<String>,
    username: String,
    password: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CredentialRequest {
    VerifyAccount,
    VerifyQuickLogin,
    TemporaryLogin,
    SwitchAccount,
}
impl CredentialRequest {
    fn saves_account(self) -> bool {
        matches!(self, Self::VerifyAccount | Self::VerifyQuickLogin)
    }
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
    account_editing: bool,
    account_offset: usize,
    quick_login: Option<AccountDraft>,
    rule: RuleDraft,
    profile_id: String,
    profile_name: String,
    statistics: Option<Statistics>,
    log_viewer_busy: bool,
    log_size: Option<(Scope, u64)>,
    diagnostics: String,
    exported: String,
    import_content: text_editor::Content,
    confirmation: Option<Confirmation>,
    network_return: Page,
    settings_tab: SettingsTab,
    health_editor: Option<Vec<HealthTarget>>,
    enabling_automatic: bool,
    alternate_adapters: bool,
    network_section: NetworkSection,
    network_offset: usize,
    notice: Option<(bool, String)>,
    notice_until: Option<Instant>,
    connection_error: Option<AppError>,
    pending_check: bool,
    connecting: bool,
    polling: bool,
    busy: bool,
    pending_settings: bool,
    pending_account_settings: Option<Settings>,
    credential_request: Option<CredentialRequest>,
    settings_epoch: u64,
    saved_epoch: u64,
    request_epoch: u64,
    startup_hidden: bool,
    window: Option<window::Id>,
    tray: Option<tray::Tray>,
    notification_gate: notifications::Gate,
    pending_notification: Option<notifications::Kind>,
    update_policy: UpdatePolicy,
    /// Wall clock until which an entered password is shown in clear text.
    password_revealed_until: Option<Instant>,
    /// Whether IP addresses are shown in clear text. Never persisted.
    ips_revealed: bool,
    network_prompt: Option<NetworkPrompt>,
    skipped_network: Option<String>,
    trust_offset: usize,
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
        let hide = args
            .iter()
            .any(|s| matches!(s.as_str(), "--hidden" | "--silent"));
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
            account_editing: false,
            account_offset: 0,
            quick_login: None,
            rule: RuleDraft::default(),
            profile_id: String::new(),
            profile_name: String::new(),
            statistics: None,
            log_viewer_busy: false,
            log_size: None,
            diagnostics: String::new(),
            exported: String::new(),
            import_content: text_editor::Content::new(),
            confirmation: None,
            network_return: Page::Status,
            settings_tab: SettingsTab::General,
            health_editor: None,
            enabling_automatic: false,
            alternate_adapters: false,
            network_section: NetworkSection::Adapters,
            network_offset: 0,
            notice: None,
            notice_until: None,
            connection_error: None,
            pending_check: false,
            connecting: true,
            polling: false,
            busy: false,
            pending_settings: false,
            pending_account_settings: None,
            credential_request: None,
            settings_epoch: 0,
            saved_epoch: 0,
            request_epoch: 0,
            startup_hidden: hide,
            window: None,
            tray,
            notification_gate: notifications::Gate::default(),
            pending_notification: None,
            update_policy: UpdatePolicy::new(),
            password_revealed_until: None,
            ips_revealed: false,
            network_prompt: None,
            skipped_network: None,
            trust_offset: 0,
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

    fn password_visible(&self) -> bool {
        self.password_revealed_until
            .is_some_and(|until| Instant::now() < until)
    }

    fn reveal_password(&mut self) {
        self.password_revealed_until = if self.password_visible() {
            None
        } else {
            Some(Instant::now() + Duration::from_secs(5))
        };
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
    fn set_language(&mut self, language: Language) {
        self.language = language;
        if let Some(tray) = &mut self.tray {
            tray.set_language(language == Language::English);
        }
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
    fn login_draft(&self) -> &AccountDraft {
        self.quick_login.as_ref().unwrap_or(&self.account)
    }
    fn login_draft_mut(&mut self) -> &mut AccountDraft {
        self.quick_login.as_mut().unwrap_or(&mut self.account)
    }
    fn take_confirmed_password(
        &mut self,
        username: &str,
        expected_network: &str,
    ) -> Option<Secret> {
        if self.busy || !self.snapshot_current {
            return None;
        }
        if self
            .snapshot
            .as_ref()
            .and_then(confirmation_target)
            .is_none_or(|(target, _)| target != expected_network)
        {
            self.login_draft_mut().password.zeroize();
            self.show_notice(
                true,
                self.t(
                    "登录网络已变化，请重新确认",
                    "The sign-in network changed. Confirm the new network.",
                )
                .into(),
            );
            return None;
        }
        let draft = self.login_draft_mut();
        if draft.username.trim() != username || draft.password.is_empty() {
            return None;
        }
        Some(Secret::new(std::mem::take(&mut draft.password)))
    }
    fn prepare_login(&mut self, confirmed_network: Option<String>) -> Task<Message> {
        let temporary = confirmed_network.is_none();
        if self.busy
            || !self.snapshot_current
            || self.confirmation.is_some()
            || (temporary && self.quick_login.is_none())
        {
            return Task::none();
        }
        let username = self.login_draft().username.trim().to_owned();
        if username.is_empty() || self.login_draft().password.is_empty() {
            self.show_notice(
                true,
                self.t(
                    "请输入校园账号和密码",
                    "Enter your campus username and password.",
                )
                .into(),
            );
            return Task::none();
        }
        let Some(snapshot) = &self.snapshot else {
            return Task::none();
        };
        if temporary && snapshot.session.auth == AuthState::Authenticated {
            self.show_notice(
                true,
                self.t(
                    "已有校园网会话在线",
                    "A campus network session is already active.",
                )
                .into(),
            );
            return Task::none();
        }
        let capability = if temporary {
            "login_once"
        } else {
            "verify_account"
        };
        if !snapshot.capabilities.iter().any(|item| item == capability) {
            self.show_notice(
                true,
                self.t(
                    "请更新认证引擎后重试",
                    "Update the authentication engine and try again.",
                )
                .into(),
            );
            return Task::none();
        }
        if bitnet_runtime::engine::uses_system_authentication(snapshot) {
            self.show_notice(
                true,
                self.t(
                    "BIT-mobile 由系统登录。请在使用校园门户的网络上继续。",
                    "Your system signs in to BIT-mobile. Use a campus portal network to continue.",
                )
                .into(),
            );
            return Task::none();
        }
        let Some((expected_network, _)) = confirmation_target(snapshot) else {
            self.show_notice(
                true,
                self.t(
                    "请先连接校园网，或在「网络」中选择网卡。",
                    "Connect to the campus network or choose an adapter under Network.",
                )
                .into(),
            );
            return Task::none();
        };
        self.clear_notice();
        self.password_revealed_until = None;
        if temporary {
            self.confirmation = Some(Confirmation::TemporaryLogin {
                expected_network,
                username,
            });
            return Task::none();
        }
        let expected_network = confirmed_network.expect("account save supplies a network");
        let Some(password) = self.take_confirmed_password(&username, &expected_network) else {
            return Task::none();
        };
        let id = self
            .login_draft()
            .id
            .clone()
            .unwrap_or_else(|| format!("account-{}", bitnet_core::now_ms()));
        self.execute(Command::VerifyAccount {
            id,
            username,
            password,
            expected_network,
        })
    }
    fn switch_saved_account(
        &mut self,
        target: SavedAccountTarget,
        confirmed: bool,
    ) -> Task<Message> {
        if self.busy || !self.snapshot_current {
            return Task::none();
        }
        let Some(snapshot) = self
            .snapshot
            .as_ref()
            .filter(|snapshot| target.matches(snapshot))
        else {
            self.show_notice(
                true,
                self.t(
                    "账户或网络已变化，请刷新后重新选择",
                    "The account or network changed. Refresh and choose again.",
                )
                .into(),
            );
            return Task::none();
        };
        let disconnecting = snapshot.session.auth != AuthState::Unauthenticated
            && snapshot.session.username.as_deref() != Some(&target.username);
        if disconnecting && !confirmed {
            self.confirmation = Some(Confirmation::SwitchAccount(target));
            return self.open_window();
        }
        self.execute(Command::SwitchAccount {
            id: target.id,
            expected_network: target.network,
            expected_scope: target.scope,
            expected_revision: target.revision,
            confirmed,
        })
    }

    fn title(&self, _: window::Id) -> String {
        let app = if preview_only() {
            self.t("BitNet 预览", "BitNet Preview")
        } else {
            "BitNet"
        };
        format!("{app} · {}", self.page.label(self.english()))
    }
    fn theme(&self, _: window::Id) -> Option<Theme> {
        match self.appearance {
            Appearance::System => None,
            Appearance::Light => Some(design::theme("light")),
            Appearance::Dark => Some(design::theme("dark")),
        }
    }
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch([
            iced::time::every(Duration::from_secs(2)).map(|_| Message::Tick),
            if self.password_revealed_until.is_some() {
                iced::time::every(Duration::from_millis(250)).map(|_| Message::RevealTick)
            } else {
                Subscription::none()
            },
            if self.tray.is_some() {
                iced::time::every(Duration::from_millis(250)).map(|_| Message::TrayTick)
            } else {
                Subscription::none()
            },
            window::close_requests().map(Message::WindowCloseRequested),
            iced::keyboard::listen().filter_map(|event| match event {
                iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                } => Some(Message::Escape),
                iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Tab),
                    modifiers,
                    ..
                } if modifiers.is_empty() || modifiers == iced::keyboard::Modifiers::SHIFT => {
                    Some(Message::MoveFocus(modifiers.shift()))
                }
                iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter),
                    modifiers,
                    repeat: false,
                    ..
                } if modifiers.is_empty() => Some(Message::KeyboardEnter),
                _ => None,
            }),
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
        let target = confirmation_target(&snapshot);
        if let Some((fingerprint, _)) = &target {
            if self
                .skipped_network
                .as_ref()
                .is_some_and(|previous| previous != fingerprint)
            {
                self.skipped_network = None;
            }
        }
        self.network_prompt = target.and_then(|(fingerprint, label)| {
            (snapshot
                .capabilities
                .iter()
                .any(|item| item == "network_permissions")
                && snapshot.settings.automatic
                && !snapshot.settings.paused
                && !snapshot.verification_hold
                && !snapshot.network_trusted
                && !snapshot.accounts.is_empty()
                && snapshot.environment != Environment::NonCampus
                && snapshot.session.auth != AuthState::Unknown
                && !bitnet_runtime::engine::uses_system_authentication(&snapshot)
                && !bitnet_runtime::engine::network_permission_denied(&snapshot)
                && self.skipped_network.as_ref() != Some(&fingerprint))
            .then_some(NetworkPrompt { fingerprint, label })
        });
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
        self.connection_error = None;
        self.snapshot = Some(snapshot);
    }
    fn can_start_service(&self) -> bool {
        !self.busy
            && !self.connecting
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.service.installed && (!self.snapshot_current || !snapshot.service.running)
            })
    }
    fn can_read_log_size(&self) -> bool {
        self.snapshot_current
            && !self.busy
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .capabilities
                    .iter()
                    .any(|item| item == "clear_logs")
            })
    }

    fn current_log_size(&self) -> Option<(Scope, u64)> {
        let (scope, bytes) = self.log_size?;
        self.snapshot
            .as_ref()
            .filter(|snapshot| snapshot.scope == scope && self.snapshot_current)?;
        Some((scope, bytes))
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
        self.pending_check = matches!(command, Command::Check);
        self.credential_request = match &command {
            Command::VerifyAccount { .. } if self.quick_login.is_some() => {
                Some(CredentialRequest::VerifyQuickLogin)
            }
            Command::VerifyAccount { .. } => Some(CredentialRequest::VerifyAccount),
            Command::LoginOnce { .. } => Some(CredentialRequest::TemporaryLogin),
            Command::SwitchAccount { .. } => Some(CredentialRequest::SwitchAccount),
            _ => None,
        };
        if self.pending_settings {
            self.saved_epoch = self.settings_epoch;
        }
        if let (
            Command::AccountPut { id, .. }
            | Command::VerifyAccount { id, .. }
            | Command::SwitchAccount { id, .. },
            Some(snapshot),
        ) = (&command, &self.snapshot)
        {
            // An account save advances the revision and clears its rejected marker.
            let mut expected = snapshot.settings.clone();
            self.pending_account_settings = expected.revision.checked_add(1).map(|revision| {
                expected.revision = revision;
                if self
                    .credential_request
                    .is_some_and(CredentialRequest::saves_account)
                    || matches!(command, Command::SwitchAccount { .. })
                    || expected.selected_account.is_none()
                {
                    expected.selected_account = Some(id.clone());
                }
                expected.blocked_accounts.retain(|blocked| blocked != id);
                expected
            });
        }
        self.clear_notice();
        Task::perform(
            {
                let client = self.client.clone();
                async move { client.command(command).await }
            },
            Message::Completed,
        )
    }
    fn error(&mut self, error: AppError) {
        self.show_notice(true, format!("{} · {:?}", error.message, error.code));
    }
    fn show_notice(&mut self, is_error: bool, message: String) {
        self.notice = Some((is_error, message));
        self.notice_until =
            Some(Instant::now() + Duration::from_secs(if is_error { 10 } else { 5 }));
    }
    fn clear_notice(&mut self) {
        self.notice = None;
        self.notice_until = None;
    }
    fn mark_settings_dirty(&mut self) {
        self.draft.dirty = true;
        self.settings_epoch = self.settings_epoch.wrapping_add(1);
    }
    fn maintain(&mut self, action: MaintenanceAction) -> Task<Message> {
        if self.busy || self.connecting {
            return Task::none();
        }
        if preview_only() {
            self.show_notice(
                false,
                self.t(
                    "预览模式不执行系统维护",
                    "System maintenance is disabled in preview mode",
                )
                .into(),
            );
            return Task::none();
        }
        if let Some(target) = action.ipv6_target() {
            let actual = self
                .snapshot
                .as_ref()
                .filter(|_| self.snapshot_current)
                .and_then(ipv6_maintenance_target);
            if actual.as_ref() != Some(target) {
                self.error(AppError::new(
                    ErrorCode::Unavailable,
                    self.t(
                        "当前网卡已更换，请刷新后再试",
                        "The active adapter changed. Refresh and try again.",
                    ),
                ));
                return Task::none();
            }
        }
        self.busy = true;
        self.request_epoch = self.request_epoch.wrapping_add(1);
        let current_scope = self
            .snapshot
            .as_ref()
            .map(|s| s.scope)
            .unwrap_or_else(|| self.mode.into());
        let scope = match action {
            MaintenanceAction::ServiceInstall
            | MaintenanceAction::ServiceStart
            | MaintenanceAction::ServiceStop
            | MaintenanceAction::ServiceRemove { .. }
            | MaintenanceAction::EnableIpv6 { .. }
            | MaintenanceAction::DisableIpv6 { .. }
            | MaintenanceAction::ResetIpv6 { .. } => Scope::System,
            _ => current_scope,
        };
        Task::perform(
            maintenance::execute(scope, action, true),
            Message::MaintenanceCompleted,
        )
    }
    fn save_settings(&mut self) -> Task<Message> {
        match self.draft.validated(self.english()) {
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
                if self
                    .tray
                    .as_ref()
                    .is_some_and(|tray| !tray.is_starting() && !tray.is_available())
                {
                    self.tray = None;
                    if self.window.is_none() && !self.connecting {
                        return self.open_window();
                    }
                }
                if let Some(action) = self.tray.as_ref().and_then(tray::Tray::poll) {
                    return match action {
                        tray::Action::Open => self.open_window(),
                        #[cfg(windows)]
                        tray::Action::Toggle => match self.window {
                            Some(id) => window::is_minimized(id).map(move |minimized| {
                                Message::TrayToggleWindow(id, minimized.unwrap_or(false))
                            }),
                            None => self.open_window(),
                        },
                        tray::Action::Check => self.execute(Command::Check),
                        tray::Action::Pause => self.execute(Command::Pause),
                        tray::Action::Resume => self.update(Message::Run(Command::Resume)),
                        tray::Action::Exit => iced::exit(),
                    };
                }
                Task::none()
            }
            #[cfg(windows)]
            Message::TrayToggleWindow(id, minimized) => {
                if self.window != Some(id) {
                    return Task::none();
                }
                if minimized {
                    window::minimize(id, false).chain(window::gain_focus(id))
                } else {
                    self.update(Message::WindowCloseRequested(id))
                }
            }
            Message::Tick => {
                if self
                    .notice_until
                    .is_some_and(|until| until <= Instant::now())
                {
                    self.clear_notice();
                }
                let status = self.refresh();
                let notification = self
                    .pending_notification
                    .take()
                    .filter(|_| !preview_only())
                    .map(|kind| {
                        Task::perform(
                            notifications::show(kind, self.english()),
                            Message::NotificationSent,
                        )
                    })
                    .unwrap_or_else(Task::none);
                let update = if !self.busy && !self.connecting && !preview_only() {
                    self.snapshot
                        .as_ref()
                        .filter(|snapshot| {
                            self.update_policy
                                .due(&snapshot.settings, bitnet_core::now_secs())
                        })
                        .map(|snapshot| {
                            Task::perform(
                                update_policy::check_desktop(
                                    snapshot.scope,
                                    snapshot.settings.auto_update,
                                ),
                                |result| Message::UpdateChecked(result, false),
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
                    self.show_notice(
                        true,
                        self.t("系统通知暂不可用", "Desktop notifications are unavailable")
                            .into(),
                    );
                }
                Task::none()
            }
            Message::OpenReleasePage => open_release_page(),
            Message::ReleasePageOpened(opened) => {
                if !opened {
                    self.show_notice(
                        true,
                        self.t(
                            "无法打开发布页面：https://github.com/KJH-x/bitnet/releases",
                            "Could not open https://github.com/KJH-x/bitnet/releases",
                        )
                        .into(),
                    );
                }
                Task::none()
            }
            Message::CheckUpdates => {
                if preview_only() || !self.update_policy.begin_manual() {
                    return Task::none();
                }
                let scope = self
                    .snapshot
                    .as_ref()
                    .map_or(self.mode.into(), |snapshot| snapshot.scope);
                Task::perform(update_policy::check_desktop(scope, false), |result| {
                    Message::UpdateChecked(result, true)
                })
            }
            Message::UpdateChecked(result, manual) => {
                self.update_policy
                    .complete(result.is_ok(), bitnet_core::now_secs());
                match result {
                    Ok(UpdateOutcome::Available { version }) => {
                        self.show_notice(
                            false,
                            format!(
                                "{} {version}",
                                self.t(
                                    "发现新版本，请在软件更新页确认安装：",
                                    "Update available; review it in Updates:"
                                )
                            ),
                        );
                        self.pending_notification
                            .get_or_insert(notifications::Kind::Update);
                    }
                    Ok(UpdateOutcome::InstallerStarted { message }) => {
                        self.show_notice(false, message)
                    }
                    Ok(UpdateOutcome::PublicRelease { version }) => {
                        self.show_notice(
                            false,
                            format!("{} {version}", self.t(
                            "GitHub 已发布新版，可在「发布说明与下载」获取",
                            "New GitHub release available under Release notes and downloads",
                        )),
                        )
                    }
                    Ok(UpdateOutcome::NoUpdate) if manual => self.show_notice(
                        false,
                        self.t("当前没有新版", "No newer release is available")
                            .into(),
                    ),
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
                        self.connection_error = Some(error);
                    }
                }
                let hidden = self
                    .tray
                    .as_ref()
                    .is_some_and(|tray| tray.is_starting() || tray.is_available())
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
                        self.connection_error = Some(error);
                    }
                }
                Task::none()
            }
            Message::Completed(result) => {
                self.busy = false;
                let credential_request = self.credential_request.take();
                let checked = std::mem::take(&mut self.pending_check);
                let expected_account_settings = self.pending_account_settings.take();
                match result {
                    Ok(ReplyData::Snapshot(snapshot)) => {
                        if self.pending_settings && self.saved_epoch == self.settings_epoch {
                            self.draft.dirty = false;
                        }
                        // Rebase only the revision produced by our account
                        // write. A poll or a different settings write must
                        // never make a stale draft valid.
                        if self.draft.dirty
                            && self.draft.settings.revision.checked_add(1)
                                == Some(snapshot.settings.revision)
                            && expected_account_settings.as_ref() == Some(&snapshot.settings)
                        {
                            self.draft.settings.revision = snapshot.settings.revision;
                            self.draft.settings.selected_account =
                                snapshot.settings.selected_account.clone();
                            self.draft.settings.blocked_accounts =
                                snapshot.settings.blocked_accounts.clone();
                        }
                        let (attention, message) = if credential_request
                            .is_some_and(CredentialRequest::saves_account)
                        {
                            if credential_request == Some(CredentialRequest::VerifyQuickLogin) {
                                self.quick_login = None;
                            } else {
                                self.account = AccountDraft::default();
                                self.account_editing = false;
                            }
                            (
                                false,
                                self.t(
                                    "登录验证通过，账户已保存",
                                    "Sign-in verified. Account saved.",
                                ),
                            )
                        } else if credential_request == Some(CredentialRequest::TemporaryLogin) {
                            self.quick_login = None;
                            (
                                false,
                                self.t("已登录，未保存凭据", "Signed in for this session"),
                            )
                        } else if credential_request == Some(CredentialRequest::SwitchAccount) {
                            (
                                false,
                                self.t("已切换并确认账户在线", "Account switched and connected"),
                            )
                        } else if !checked {
                            (false, self.t("操作已完成", "Operation completed"))
                        } else if snapshot.active_binding.is_none() {
                            (
                                true,
                                self.t(
                                    "未检测：请选择可用网卡",
                                    "Check skipped: select an available adapter",
                                ),
                            )
                        } else if snapshot.environment == Environment::NonCampus {
                            (
                                true,
                                self.t(
                                    "未检测：当前网络已跳过",
                                    "Check skipped on the current network",
                                ),
                            )
                        } else {
                            (
                                false,
                                self.t("已查询校园网状态", "Campus network status checked"),
                            )
                        };
                        self.apply_snapshot(*snapshot);
                        self.show_notice(attention, message.into());
                    }
                    Ok(ReplyData::History(_)) => {}
                    Ok(ReplyData::LogSize { scope, bytes }) => {
                        self.log_size = Some((scope, bytes));
                    }
                    Ok(ReplyData::LogsCleared { scope, bytes }) => {
                        self.log_size = Some((scope, 0));
                        self.show_notice(
                            false,
                            format!(
                                "{} · {}",
                                self.t("已清除诊断日志", "Diagnostic logs cleared"),
                                format_log_size(bytes)
                            ),
                        );
                    }
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
                        self.show_notice(false, message);
                    }
                    Err(error) if credential_request.is_some() => {
                        self.show_notice(true, error.message)
                    }
                    Err(error) => self.error(error),
                }
                self.pending_settings = false;
                self.refresh()
            }
            Message::Navigate(page) => match page {
                Page::Networks => {
                    if self.page != Page::Networks {
                        self.network_return = self.page;
                    }
                    self.page = page;
                    self.network_section = NetworkSection::Adapters;
                    self.network_offset = 0;
                    Task::none()
                }
                Page::Statistics => {
                    self.enabling_automatic = false;
                    self.page = page;
                    if self.snapshot_current {
                        self.execute(Command::Statistics)
                    } else {
                        Task::none()
                    }
                }
                Page::Diagnostics => {
                    self.enabling_automatic = false;
                    self.page = Page::Diagnostics;
                    Task::none()
                }
                Page::Settings => {
                    self.enabling_automatic = false;
                    self.page = page;
                    if self.settings_tab == SettingsTab::Diagnostics && self.can_read_log_size() {
                        self.log_size = None;
                        self.execute(Command::LogSize)
                    } else {
                        Task::none()
                    }
                }
                _ => {
                    self.enabling_automatic = false;
                    self.page = page;
                    Task::none()
                }
            },
            Message::MaintenanceCompleted(result) => {
                self.busy = false;
                match result {
                    Ok(result) => {
                        let message = match result.details["ipv6_enabled"].as_bool() {
                            Some(true) => self.t("IPv6 已启用", "IPv6 enabled").into(),
                            Some(false) => self.t("IPv6 已关闭", "IPv6 disabled").into(),
                            None => result.message,
                        };
                        self.show_notice(false, message);
                    }
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
                self.connection_error = None;
                self.quick_login = None;
                self.confirmation = None;
                self.account_editing = false;
                self.password_revealed_until = None;
                self.network_prompt = None;
                self.skipped_network = None;
                self.trust_offset = 0;
                self.account_offset = 0;
                self.account = AccountDraft::default();
                self.enabling_automatic = false;
                self.network_section = NetworkSection::Adapters;
                self.network_offset = 0;
                self.statistics = None;
                self.log_size = None;
                self.diagnostics.clear();
                self.exported.clear();
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
            Message::Run(Command::Resume)
                if self
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| !snapshot.settings.automatic) =>
            {
                let task = self.update(Message::AutomaticChanged(true));
                Task::batch([task, self.open_window()])
            }
            Message::Run(command) => self.execute(command),
            Message::Ask(confirmation) => {
                self.confirmation = Some(confirmation);
                self.open_window()
            }
            Message::Confirm => match self.confirmation.take() {
                Some(Confirmation::TemporaryLogin {
                    expected_network,
                    username,
                    ..
                }) => {
                    let Some(password) = self.take_confirmed_password(&username, &expected_network)
                    else {
                        return Task::none();
                    };
                    self.execute(Command::LoginOnce {
                        username,
                        password,
                        expected_network,
                    })
                }
                Some(Confirmation::EnableAutomatic {
                    expected_network, ..
                }) => {
                    self.enabling_automatic = false;
                    if self.page == Page::Networks {
                        self.page = self.network_return;
                    }
                    self.execute(Command::EnableAutomatic { expected_network })
                }
                Some(Confirmation::LoginOnce) => self.execute(Command::Login {
                    account_id: None,
                    force: false,
                    confirmed: true,
                }),
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
                Some(Confirmation::SwitchAccount(target)) => {
                    self.switch_saved_account(target, true)
                }
                Some(Confirmation::ClearLogs { scope, .. }) => {
                    if !self.can_read_log_size()
                        || self.current_log_size().map(|entry| entry.0) != Some(scope)
                    {
                        return Task::none();
                    }
                    self.log_size = None;
                    self.execute(Command::ClearLogs {
                        expected_scope: scope,
                    })
                }
                None => Task::none(),
            },
            Message::Cancel => {
                if self.credential_request.is_some() {
                    return Task::none();
                }
                self.password_revealed_until = None;
                if self.confirmation.take().is_none()
                    && self.health_editor.take().is_none()
                    && self.quick_login.take().is_none()
                {
                    self.enabling_automatic = false;
                }
                Task::none()
            }
            Message::Escape => {
                if self.confirmation.is_some()
                    || self.health_editor.is_some()
                    || self.quick_login.is_some()
                {
                    return self.update(Message::Cancel);
                }
                if self.account_editing {
                    return self.update(Message::AccountCancel);
                }
                if let Some(prompt) = self
                    .network_prompt
                    .as_ref()
                    .filter(|_| !self.enabling_automatic)
                {
                    return self.update(Message::NetworkDecision {
                        fingerprint: prompt.fingerprint.clone(),
                        allow: None,
                    });
                }
                if self.page == Page::Networks {
                    return self.update(Message::NetworkBack);
                }
                match self.window {
                    Some(id) if self.tray.as_ref().is_some_and(tray::Tray::is_available) => {
                        self.update(Message::WindowCloseRequested(id))
                    }
                    _ => Task::none(),
                }
            }
            Message::KeyboardEnter => {
                if matches!(self.confirmation, Some(Confirmation::TemporaryLogin { .. })) {
                    self.update(Message::Confirm)
                } else {
                    Task::none()
                }
            }
            Message::MoveFocus(previous) => {
                if previous {
                    iced::widget::operation::focus_previous()
                } else {
                    iced::widget::operation::focus_next()
                }
            }
            Message::TogglePassword => {
                self.reveal_password();
                Task::none()
            }
            Message::ToggleIps => {
                self.ips_revealed = !self.ips_revealed;
                Task::none()
            }
            Message::RevealTick => {
                if self
                    .password_revealed_until
                    .is_some_and(|until| Instant::now() >= until)
                {
                    self.password_revealed_until = None;
                }
                Task::none()
            }
            Message::DismissNotice => {
                self.clear_notice();
                Task::none()
            }
            Message::UsernameChanged(value) => {
                self.login_draft_mut().username = value;
                Task::none()
            }
            Message::PasswordChanged(value) => {
                let draft = self.login_draft_mut();
                draft.password.zeroize();
                draft.password = value;
                Task::none()
            }
            Message::AccountAdd => {
                self.clear_notice();
                self.password_revealed_until = None;
                self.account = AccountDraft::default();
                self.account_editing = true;
                iced::widget::operation::focus("add-account-username")
            }
            Message::AccountPage(offset) => {
                self.account_offset = offset;
                Task::none()
            }
            Message::AccountCancel => {
                if self.credential_request == Some(CredentialRequest::VerifyAccount) {
                    return Task::none();
                }
                self.clear_notice();
                self.password_revealed_until = None;
                self.account = AccountDraft::default();
                self.account_editing = false;
                Task::none()
            }
            Message::OpenQuickLogin => {
                if self.busy || !self.snapshot_current {
                    return Task::none();
                }
                self.clear_notice();
                self.password_revealed_until = None;
                self.quick_login = Some(AccountDraft::default());
                iced::widget::operation::focus("quick-login-username")
            }
            Message::TemporaryLogin => self.prepare_login(None),
            Message::AccountSave(fingerprint) => self.prepare_login(Some(fingerprint)),
            Message::SwitchAccount(target) => self.switch_saved_account(target, false),
            Message::LanguageChanged(language) => {
                self.set_language(language);
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
            Message::SaveSettings => self.save_settings(),
            Message::AutomaticChanged(false) => {
                self.draft.settings.automatic = false;
                self.mark_settings_dirty();
                Task::none()
            }
            Message::AutomaticChanged(true) => {
                let Some(snapshot) = self
                    .snapshot
                    .as_ref()
                    .filter(|_| self.snapshot_current && !self.busy)
                else {
                    return Task::none();
                };
                if snapshot.settings.automatic {
                    self.draft.settings.automatic = true;
                    return Task::none();
                }
                if !snapshot
                    .capabilities
                    .iter()
                    .any(|item| item == "enable_automatic")
                {
                    self.error(AppError::new(
                        ErrorCode::Incompatible,
                        self.t(
                            "请先更新后台，再开启自动保持登录",
                            "Update the background engine before enabling automatic authentication",
                        ),
                    ));
                    return Task::none();
                }
                if !snapshot.accounts.iter().any(|account| account.has_password) {
                    self.page = Page::Accounts;
                    self.error(AppError::new(
                        ErrorCode::Credentials,
                        self.t("请先保存校园账户", "Save a campus account first"),
                    ));
                    return Task::none();
                }
                if self.draft.dirty {
                    self.error(AppError::new(
                        ErrorCode::Busy,
                        self.t(
                            "请先保存或放弃当前设置",
                            "Save or discard pending settings first",
                        ),
                    ));
                    return Task::none();
                }
                let mut candidates = snapshot.interfaces.iter().filter(|adapter| {
                    adapter.up
                        && !is_alternate_adapter(adapter)
                        && adapter
                            .ipv4
                            .iter()
                            .chain(adapter.gateway.iter())
                            .any(|ip| ip.octets()[0] == 10)
                });
                let unique = candidates.next().filter(|_| candidates.next().is_none());
                self.enabling_automatic = true;
                if !bitnet_runtime::engine::uses_system_authentication(snapshot)
                    && selected_binding(snapshot).is_some_and(|binding| {
                        snapshot.settings.interface_id.is_some()
                            || unique.is_some_and(|adapter| binding.interface_id == adapter.id)
                    })
                {
                    return self.update(Message::BeginAutomatic);
                }
                self.update(Message::Navigate(Page::Networks))
            }
            Message::BeginAutomatic => {
                if self.busy || !self.snapshot_current {
                    return Task::none();
                }
                if self.snapshot.as_ref().is_some_and(|snapshot| {
                    bitnet_runtime::engine::uses_system_authentication(snapshot)
                        || (selected_binding(snapshot).is_some()
                            && snapshot.environment == Environment::NonCampus)
                }) {
                    self.error(AppError::new(
                        ErrorCode::Network,
                        self.t("当前网络不使用校园门户认证，请选择其他网卡", "This network does not use campus portal sign-in. Select another adapter."),
                    ));
                    return Task::none();
                }
                let candidate = self.snapshot.as_ref().and_then(confirmation_target);
                match candidate {
                    Some((expected_network, _)) => {
                        self.confirmation =
                            Some(Confirmation::EnableAutomatic { expected_network });
                    }
                    None => self.error(AppError::new(
                        ErrorCode::Network,
                        self.t(
                            "请选择可用的校园网网卡",
                            "Select an available campus network adapter",
                        ),
                    )),
                }
                Task::none()
            }
            Message::RevertSettings => {
                let settings = self
                    .snapshot
                    .as_ref()
                    .map(|snapshot| snapshot.settings.clone())
                    .unwrap_or_default();
                self.set_language(if settings.language == "en" {
                    Language::English
                } else {
                    Language::Chinese
                });
                self.appearance = match settings.theme.as_str() {
                    "light" => Appearance::Light,
                    "dark" => Appearance::Dark,
                    _ => Appearance::System,
                };
                self.draft = SettingsDraft::new(settings);
                self.settings_epoch = self.settings_epoch.wrapping_add(1);
                Task::none()
            }
            Message::SettingsTabChanged(tab) => {
                self.settings_tab = tab;
                if tab == SettingsTab::Diagnostics && self.can_read_log_size() {
                    self.log_size = None;
                    self.execute(Command::LogSize)
                } else {
                    Task::none()
                }
            }
            Message::OpenHealthEditor => {
                self.clear_notice();
                self.health_editor = Some(
                    self.draft
                        .settings
                        .configured_health_targets()
                        .into_iter()
                        .map(Into::into)
                        .collect(),
                );
                Task::none()
            }
            Message::HealthEntryChanged(index, address) => {
                if let Some(entries) = self
                    .health_editor
                    .as_mut()
                    .filter(|entries| index < entries.len())
                {
                    entries[index].url = address;
                }
                Task::none()
            }
            Message::HealthEntryExpectChanged(index, condition) => {
                if let Some(entries) = self
                    .health_editor
                    .as_mut()
                    .filter(|entries| index < entries.len())
                {
                    let entry = &mut entries[index];
                    if entry.condition != condition {
                        entry.condition = condition;
                        entry.value.clear();
                    }
                }
                Task::none()
            }
            Message::HealthEntryValueChanged(index, value) => {
                if let Some(entries) = self
                    .health_editor
                    .as_mut()
                    .filter(|entries| index < entries.len())
                {
                    entries[index].value = value;
                }
                Task::none()
            }
            Message::HealthEntryAdded => {
                if let Some(entries) = &mut self.health_editor {
                    if entries.len() < MAX_HEALTH_TARGETS {
                        entries.push(HealthTarget::default());
                    }
                }
                Task::none()
            }
            Message::HealthEntryRemoved(index) => {
                if let Some(entries) = &mut self.health_editor {
                    if index < entries.len() {
                        entries.remove(index);
                    }
                }
                Task::none()
            }
            Message::SaveHealthEditor => {
                if let Some(entries) = &self.health_editor {
                    let mut targets = Vec::with_capacity(entries.len());
                    for entry in entries {
                        let expect = match entry.condition {
                            HealthCondition::AnySuccess => HealthExpect::AnySuccess,
                            HealthCondition::Status => {
                                let code = entry
                                    .value
                                    .trim()
                                    .parse()
                                    .ok()
                                    .filter(|code| (100..=599).contains(code));
                                let Some(code) = code else {
                                    let error = self.t(
                                        "请输入 100 到 599 之间的 HTTP 状态码。",
                                        "Enter an HTTP status code from 100 to 599.",
                                    );
                                    self.show_notice(true, error.into());
                                    return Task::none();
                                };
                                HealthExpect::Status(code)
                            }
                            HealthCondition::BodyEquals => {
                                HealthExpect::BodyEquals(entry.value.clone())
                            }
                            HealthCondition::BodyContains => {
                                HealthExpect::BodyContains(entry.value.clone())
                            }
                        };
                        targets.push(ConfigHealthTarget {
                            url: entry.url.trim().into(),
                            expect,
                        });
                    }
                    match self.draft.settings.set_health_targets(targets) {
                        Ok(()) => {
                            self.health_editor = None;
                            self.clear_notice();
                            self.mark_settings_dirty();
                        }
                        Err(error) => self.error(error),
                    }
                }
                Task::none()
            }
            Message::CancelHealthEditor => {
                self.health_editor = None;
                self.clear_notice();
                Task::none()
            }
            Message::ToggleAlternateAdapters => {
                self.alternate_adapters = !self.alternate_adapters;
                self.network_offset = 0;
                Task::none()
            }
            Message::NetworkBack => {
                self.enabling_automatic = false;
                self.page = self.network_return;
                Task::none()
            }
            Message::NetworkTab(section) => {
                self.network_section = section;
                self.network_offset = 0;
                Task::none()
            }
            Message::NetworkPage(offset) => {
                self.network_offset = offset;
                Task::none()
            }
            Message::InterfaceSelected(value) => {
                if self.busy || !self.snapshot_current || self.draft.dirty {
                    return Task::none();
                }
                let selected = if let Some((id, ssid)) = value {
                    let Some(interface) = self
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| {
                            snapshot
                                .interfaces
                                .iter()
                                .find(|interface| interface.id == id && interface.ssid == ssid)
                        })
                        .filter(|interface| interface.binding().is_some())
                    else {
                        self.show_notice(
                            true,
                            self.t(
                                "网卡连接已变化，请重新选择。",
                                "The adapter connection changed. Choose again.",
                            )
                            .into(),
                        );
                        return Task::none();
                    };
                    Some(interface)
                } else {
                    None
                };
                self.draft.settings.interface_ssid = selected
                    .filter(|_| {
                        self.snapshot.as_ref().is_some_and(|snapshot| {
                            snapshot
                                .capabilities
                                .iter()
                                .any(|item| item == "network_permissions")
                        })
                    })
                    .and_then(|interface| interface.ssid.clone());
                self.draft.settings.interface_id = selected.map(|interface| interface.id.clone());
                self.network_offset = 0;
                self.mark_settings_dirty();
                self.save_settings()
            }
            Message::NetworkDecision { fingerprint, allow } => {
                if self.busy || !self.snapshot_current {
                    return Task::none();
                }
                if self
                    .network_prompt
                    .as_ref()
                    .is_none_or(|prompt| prompt.fingerprint != fingerprint)
                {
                    return Task::none();
                }
                self.network_prompt = None;
                if let Some(allow) = allow {
                    self.execute(Command::SetNetworkPermission {
                        expected_network: fingerprint,
                        allow,
                    })
                } else {
                    self.skipped_network = Some(fingerprint);
                    Task::none()
                }
            }
            Message::TrustPage(offset) => {
                self.trust_offset = offset;
                Task::none()
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
            Message::BeginRule => {
                self.rule = RuleDraft::default();
                self.network_section = NetworkSection::RuleEditor;
                Task::none()
            }
            Message::RuleCancel => {
                self.rule = RuleDraft::default();
                self.network_section = NetworkSection::Rules;
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
                    self.network_section = NetworkSection::RuleEditor;
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
                    self.network_section = NetworkSection::Rules;
                    self.mark_settings_dirty();
                    self.save_settings()
                }
                Err(error) => {
                    self.error(error);
                    Task::none()
                }
            },
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
            Message::CopyExport => iced::clipboard::write(self.exported.clone()).discard(),
            Message::CopyDiagnostics => iced::clipboard::write(self.diagnostics.clone()).discard(),
            Message::WebLogs(action) => {
                if self.log_viewer_busy {
                    return Task::none();
                }
                self.log_viewer_busy = true;
                let client = self.client.clone();
                let english = self.english();
                let (sender, receiver) = iced::futures::channel::oneshot::channel();
                let _ = std::thread::Builder::new().spawn(move || {
                    let _ = sender.send(bitnet_launcher::web(
                        client.effective_scope(),
                        action,
                        english,
                    ));
                });
                Task::perform(
                    async move { receiver.await.unwrap_or(Err(LaunchError::Failed)) },
                    move |result| Message::WebLogsFinished(action, result),
                )
            }
            Message::WebLogsFinished(action, result) => {
                self.log_viewer_busy = false;
                let (error, text) = match result {
                    Ok(()) if action == WebAction::Open => (
                        false,
                        self.t("已请求打开日志 Web 界面", "Requested to open web logs"),
                    ),
                    Ok(()) => (
                        false,
                        self.t("日志 Web 服务已停止", "Web log service stopped"),
                    ),
                    Err(LaunchError::Missing) => (
                        true,
                        self.t(
                            "请先安装日志 Web 组件",
                            "Install the web log component first",
                        ),
                    ),
                    Err(LaunchError::Failed) => (
                        true,
                        self.t("日志 Web 服务操作失败", "Web log service operation failed"),
                    ),
                };
                self.show_notice(error, text.into());
                Task::none()
            }
            Message::WindowOpened(id) => {
                if self.window == Some(id) {
                    Task::none()
                } else {
                    window::close(id)
                }
            }
            Message::WindowCloseRequested(id) => {
                if self.window != Some(id) {
                    return Task::none();
                }
                self.window = None;
                self.account = AccountDraft::default();
                self.account_editing = false;
                self.quick_login = None;
                self.confirmation = None;
                self.password_revealed_until = None;
                if self.tray.as_ref().is_some_and(tray::Tray::is_available) {
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
}

fn open_release_page() -> Task<Message> {
    const RELEASES: &str = "https://github.com/KJH-x/bitnet/releases";
    #[cfg(windows)]
    let launcher = match std::env::var_os("SystemRoot") {
        Some(root) => std::path::PathBuf::from(root).join("explorer.exe"),
        None => return Task::done(Message::ReleasePageOpened(false)),
    };
    #[cfg(target_os = "macos")]
    let launcher = "/usr/bin/open";
    #[cfg(all(unix, not(target_os = "macos")))]
    let launcher = "xdg-open";
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    let _ = std::thread::Builder::new().spawn(move || {
        let opened = std::process::Command::new(launcher)
            .arg(RELEASES)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            // Explorer may exit with code 1 after forwarding the URL.
            .is_ok_and(|status| cfg!(windows) || status.success());
        let _ = sender.send(opened);
    });
    Task::perform(
        async move { receiver.await.unwrap_or(false) },
        Message::ReleasePageOpened,
    )
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
fn format_log_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    }
}
fn format_duration(seconds: u64) -> String {
    format!("{} h {:02} min", seconds / 3600, (seconds / 60) % 60)
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
            account_editing: false,
            account_offset: 0,
            quick_login: None,
            rule: RuleDraft::default(),
            profile_id: String::new(),
            profile_name: String::new(),
            statistics: None,
            log_viewer_busy: false,
            log_size: None,
            diagnostics: String::new(),
            exported: String::new(),
            import_content: text_editor::Content::new(),
            confirmation: None,
            network_return: Page::Status,
            settings_tab: SettingsTab::General,
            health_editor: None,
            enabling_automatic: false,
            alternate_adapters: false,
            network_section: NetworkSection::Adapters,
            network_offset: 0,
            notice: None,
            notice_until: None,
            connection_error: None,
            pending_check: false,
            connecting: false,
            polling: false,
            busy: false,
            pending_settings: false,
            pending_account_settings: None,
            credential_request: None,
            settings_epoch: 0,
            saved_epoch: 0,
            request_epoch: 0,
            startup_hidden: false,
            window: None,
            tray: None,
            notification_gate: notifications::Gate::default(),
            pending_notification: None,
            update_policy: UpdatePolicy::new_at(0),
            password_revealed_until: None,
            ips_revealed: false,
            network_prompt: None,
            skipped_network: None,
            trust_offset: 0,
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
        assert!(draft.validated(false).is_ok());
        draft.status_interval = "0".into();
        assert!(draft.validated(false).is_err());
        draft.status_interval = "not-a-number".into();
        assert!(draft.validated(false).is_err());
    }
    #[test]
    fn health_editor_preserves_legacy_and_custom_response_conditions() {
        let mut app = fixture();
        let legacy = app.draft.settings.configured_health_targets();
        let _ = app.update(Message::OpenHealthEditor);
        let _ = app.update(Message::SaveHealthEditor);
        assert_eq!(app.draft.settings.configured_health_targets(), legacy);

        let _ = app.update(Message::OpenHealthEditor);
        let _ = app.update(Message::HealthEntryExpectChanged(
            0,
            HealthCondition::BodyContains,
        ));
        let _ = app.update(Message::HealthEntryValueChanged(0, "ready".into()));
        let _ = app.update(Message::HealthEntryValueChanged(1, "202".into()));
        let _ = app.update(Message::SaveHealthEditor);

        let mut expected = legacy;
        expected[0].expect = HealthExpect::BodyContains("ready".into());
        expected[1].expect = HealthExpect::Status(202);
        assert_eq!(app.draft.settings.configured_health_targets(), expected);
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
    fn saving_an_account_rebases_the_pending_automatic_setting() {
        let mut app = fixture();
        let mut reply = Snapshot::new(Scope::User);
        reply.settings.revision = 7;
        reply.settings.automatic = true;
        reply.settings.blocked_accounts.push("first".into());
        app.apply_snapshot(reply.clone());
        let _ = app.update(Message::AutomaticChanged(false));
        let _ = app.execute(Command::AccountPut {
            id: "first".into(),
            username: "test-user".into(),
            password: Secret::new("test-password"),
        });
        reply.settings.revision = 8;
        reply.settings.selected_account = Some("first".into());
        reply.settings.blocked_accounts.clear();
        let _ = app.update(Message::Completed(Ok(ReplyData::Snapshot(Box::new(reply)))));

        let submitted = app.draft.validated(app.english()).unwrap();
        assert!(app.draft.dirty);
        assert!(!submitted.automatic);
        assert_eq!(submitted.revision, 8);
        assert_eq!(submitted.selected_account.as_deref(), Some("first"));
        assert!(submitted.blocked_accounts.is_empty());
    }
    #[test]
    fn saving_an_account_preserves_a_concurrent_settings_conflict() {
        let mut app = fixture();
        let mut reply = Snapshot::new(Scope::User);
        reply.settings.revision = 7;
        reply.settings.automatic = true;
        app.apply_snapshot(reply.clone());
        let _ = app.update(Message::AutomaticChanged(false));
        let _ = app.execute(Command::AccountPut {
            id: "first".into(),
            username: "test-user".into(),
            password: Secret::new("test-password"),
        });
        reply.settings.revision = 9;
        reply.settings.selected_account = Some("first".into());
        reply.settings.language = "en".into();
        let _ = app.update(Message::Completed(Ok(ReplyData::Snapshot(Box::new(reply)))));

        assert!(app.draft.dirty);
        assert_eq!(app.draft.settings.revision, 7);
        assert_eq!(app.snapshot.as_ref().unwrap().settings.revision, 9);
    }
    fn login_fixture() -> App {
        let mut app = fixture();
        let mut snapshot = Snapshot::new(Scope::User);
        let adapter = bitnet_core::NetworkInterface {
            id: "ethernet".into(),
            name: "Ethernet".into(),
            index: 5,
            ipv4: vec![std::net::Ipv4Addr::new(10, 1, 2, 3)],
            gateway: Some(std::net::Ipv4Addr::new(10, 1, 2, 1)),
            up: true,
            wireless: false,
            ssid: None,
            virtual_adapter: false,
            alternate_adapter: false,
            dns: Vec::new(),
            ipv6: Vec::new(),
            ipv4_mode: None,
            ipv6_enabled: None,
        };
        snapshot.active_binding = adapter.binding();
        snapshot.interfaces.push(adapter);
        snapshot.capabilities.extend([
            "verify_account".into(),
            "login_once".into(),
            "switch_account".into(),
        ]);
        app.apply_snapshot(snapshot);
        app
    }

    #[test]
    fn saved_account_switch_binds_confirmation_to_the_displayed_network() {
        let mut app = login_fixture();
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.session.auth = AuthState::Authenticated;
        snapshot.session.username = Some("old-user".into());
        snapshot.accounts.push(AccountInfo {
            id: "other".into(),
            username: "other-user".into(),
            has_password: true,
        });
        let target = SavedAccountTarget::capture(snapshot, "other").unwrap();
        let _ = app.update(Message::SwitchAccount(target));
        assert!(matches!(
            app.confirmation,
            Some(Confirmation::SwitchAccount(_))
        ));
        assert!(!app.busy);

        let mut replacement = app.snapshot.clone().unwrap();
        replacement.interfaces[0].gateway = Some(std::net::Ipv4Addr::new(10, 1, 2, 254));
        app.apply_snapshot(replacement);
        let _ = app.update(Message::Confirm);
        assert!(!app.busy);
        assert!(app.notice.as_ref().is_some_and(|(error, _)| *error));
    }

    #[test]
    fn offline_saved_account_switch_starts_without_a_disconnect_confirmation() {
        let mut app = login_fixture();
        let snapshot = app.snapshot.as_mut().unwrap();
        snapshot.session.auth = AuthState::Unauthenticated;
        snapshot.accounts.push(AccountInfo {
            id: "other".into(),
            username: "other-user".into(),
            has_password: true,
        });
        let target = SavedAccountTarget::capture(snapshot, "other").unwrap();
        let _ = app.update(Message::SwitchAccount(target));
        assert!(app.confirmation.is_none());
        assert!(app.busy);
        assert!(app.credential_request == Some(CredentialRequest::SwitchAccount));
    }

    #[test]
    fn account_save_confirms_in_place_and_failure_keeps_the_account_draft() {
        let mut app = login_fixture();
        let _ = app.update(Message::AccountAdd);
        app.account.id = Some("existing".into());
        app.account.username = "test-user".into();
        app.account.password = "incorrect".into();

        let (fingerprint, _) = confirmation_target(app.snapshot.as_ref().unwrap()).unwrap();
        let _ = app.update(Message::AccountSave(fingerprint));
        assert!(app.confirmation.is_none());
        assert!(app.account_editing);
        assert!(app.busy);
        assert!(app.credential_request == Some(CredentialRequest::VerifyAccount));
        assert!(app.account.password.is_empty());
        let _ = app.update(Message::Completed(Err(AppError::new(
            ErrorCode::Credentials,
            "Login failed",
        ))));
        assert!(app.account_editing);
        assert_eq!(app.account.username, "test-user");
        assert_eq!(app.account.id.as_deref(), Some("existing"));
        assert_eq!(
            app.notice
                .as_ref()
                .map(|(err, message)| (*err, message.as_str())),
            Some((true, "Login failed"))
        );
    }

    #[test]
    fn decisions_from_a_replaced_network_view_cannot_use_the_new_network() {
        let mut app = login_fixture();
        let (previous, _) = confirmation_target(app.snapshot.as_ref().unwrap()).unwrap();
        let mut current = app.snapshot.clone().unwrap();
        current.interfaces[0].gateway = Some(std::net::Ipv4Addr::new(10, 1, 2, 254));
        let (fingerprint, label) = confirmation_target(&current).unwrap();
        assert_ne!(previous, fingerprint);
        app.apply_snapshot(current);
        app.network_prompt = Some(NetworkPrompt {
            fingerprint: fingerprint.clone(),
            label,
        });
        let _ = app.update(Message::NetworkDecision {
            fingerprint: previous.clone(),
            allow: Some(true),
        });
        assert_eq!(
            app.network_prompt.as_ref().unwrap().fingerprint,
            fingerprint
        );
        assert!(!app.busy);
        let _ = app.update(Message::AccountAdd);
        app.account.username = "fixture-user".into();
        app.account.password = "fixture-password".into();
        let _ = app.update(Message::AccountSave(previous));
        assert!(!app.busy);
        assert!(app.account.password.is_empty());
        assert!(app.notice.as_ref().is_some_and(|(error, _)| *error));
    }
    #[test]
    fn quick_login_uses_explicit_consent_without_changing_account_draft() {
        let mut app = login_fixture();
        app.account.username = "saved-draft".into();
        app.account.password = "draft-secret".into();
        let _ = app.update(Message::OpenQuickLogin);
        let _ = app.update(Message::UsernameChanged("one-time".into()));
        let _ = app.update(Message::PasswordChanged("one-time-secret".into()));
        let _ = app.update(Message::TemporaryLogin);
        assert!(matches!(
            app.confirmation,
            Some(Confirmation::TemporaryLogin { .. })
        ));
        assert!(!app.busy);

        let _ = app.update(Message::Escape);
        assert!(app.quick_login.is_some());
        assert!(!app.busy);
        let _ = app.update(Message::TemporaryLogin);
        let _ = app.update(Message::KeyboardEnter);
        assert!(app.credential_request == Some(CredentialRequest::TemporaryLogin));
        assert!(app.quick_login.as_ref().unwrap().password.is_empty());
        let _ = app.update(Message::Completed(Ok(ReplyData::Snapshot(Box::new(
            Snapshot::new(Scope::User),
        )))));
        assert!(app.quick_login.is_none());
        assert_eq!(app.account.username, "saved-draft");
        assert_eq!(app.account.password, "draft-secret");
    }
    #[test]
    fn a_save_reply_does_not_discard_edits_made_while_saving() {
        let mut app = fixture();
        app.pending_settings = true;
        app.busy = true;
        app.saved_epoch = 0;
        let _ = app.update(Message::FlagChanged(SettingFlag::StartHidden, true));
        let _ = app.update(Message::Completed(Ok(ReplyData::Snapshot(Box::new(
            Snapshot::new(Scope::User),
        )))));
        assert!(app.draft.dirty);
        assert!(app.draft.settings.start_hidden);
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
    fn successful_refresh_clears_only_the_disconnected_warning() {
        let mut app = fixture();
        let disconnected = AppError::new(ErrorCode::Unavailable, "engine disconnected");
        let _ = app.update(Message::Refreshed(0, Err(disconnected)));
        assert!(app.connection_error.is_some());
        app.show_notice(true, "operation failed".into());

        let _ = app.update(Message::Refreshed(0, Ok(Snapshot::new(Scope::User))));
        assert!(app.connection_error.is_none());
        assert!(app.snapshot_current);
        assert!(app.notice.is_some());
    }
    #[test]
    fn automatic_setup_waits_for_the_newly_selected_adapter_binding() {
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.settings.interface_id = Some("ethernet".into());
        snapshot.active_binding = Some(bitnet_core::NetworkBinding {
            interface_id: "wireless".into(),
            interface_index: 5,
            source_ip: std::net::Ipv4Addr::new(192, 168, 1, 2),
        });
        assert!(selected_binding(&snapshot).is_none());
        snapshot.active_binding.as_mut().unwrap().interface_id = "ethernet".into();
        assert!(selected_binding(&snapshot).is_some());
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
        let _ = app.maintain(MaintenanceAction::ServiceInstall);
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
