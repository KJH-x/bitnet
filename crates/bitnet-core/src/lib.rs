//! Shared, transport-independent application contract. No GUI or OS dependencies.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{fmt, net::Ipv4Addr};
use zeroize::Zeroizing;

pub const API_VERSION: u32 = 1;
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Default)]
pub struct Secret(Zeroizing<String>);
impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[REDACTED]")
    }
}
impl Serialize for Secret {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.expose())
    }
}
impl<'de> Deserialize<'de> for Secret {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::new)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    User,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkBinding {
    pub interface_id: String,
    pub interface_index: u32,
    pub source_ip: Ipv4Addr,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkInterface {
    pub id: String,
    pub name: String,
    pub index: u32,
    pub ipv4: Vec<Ipv4Addr>,
    pub gateway: Option<Ipv4Addr>,
    pub up: bool,
    pub wireless: bool,
    pub ssid: Option<String>,
    pub virtual_adapter: bool,
    /// Presentation hint only; it does not change the authentication route policy.
    #[serde(default)]
    pub alternate_adapter: bool,
}
impl NetworkInterface {
    pub fn binding(&self) -> Option<NetworkBinding> {
        let source_ip = *self
            .ipv4
            .iter()
            .find(|ip| !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified())?;
        self.up.then(|| NetworkBinding {
            interface_id: self.id.clone(),
            interface_index: self.index,
            source_ip,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    #[default]
    Unknown,
    Unauthenticated,
    Authenticated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct GatewaySession {
    pub auth: AuthState,
    pub username: Option<String>,
    pub ip: Option<Ipv4Addr>,
    pub bytes_used: Option<u64>,
    pub online_seconds: Option<u64>,
    pub balance: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct GatewayConfig {
    pub base_url: String,
    pub ac_id: Option<String>,
    pub timeout_secs: u64,
}
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            base_url: "http://10.0.0.55".into(),
            ac_id: None,
            timeout_secs: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub schema_version: u32,
    pub revision: u64,
    pub automatic: bool,
    pub paused: bool,
    pub selected_account: Option<String>,
    pub blocked_accounts: Vec<String>,
    /// Locally approved network fingerprints. Imported preferences cannot add trust.
    pub trusted_networks: Vec<String>,
    pub profiles: Vec<NetworkProfile>,
    pub active_profile: Option<String>,
    pub interface_id: Option<String>,
    pub gateway: GatewayConfig,
    pub status_interval_secs: u64,
    pub health_interval_secs: u64,
    pub retry_max_secs: u64,
    pub request_timeout_secs: u64,
    pub log_retention_days: u32,
    pub log_level: String,
    pub history_retention_days: u32,
    pub language: String,
    pub theme: String,
    pub start_hidden: bool,
    pub user_autostart: bool,
    pub gui_autostart: bool,
    pub check_updates: bool,
    pub auto_update: bool,
    pub notify_errors: bool,
    pub health_targets: Vec<String>,
    pub rules: Vec<NetworkRule>,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            automatic: false,
            paused: false,
            selected_account: None,
            blocked_accounts: Vec::new(),
            trusted_networks: Vec::new(),
            profiles: Vec::new(),
            active_profile: None,
            interface_id: None,
            gateway: GatewayConfig::default(),
            status_interval_secs: 60,
            health_interval_secs: 300,
            retry_max_secs: 300,
            request_timeout_secs: 5,
            log_retention_days: 14,
            log_level: "info".into(),
            history_retention_days: 30,
            language: "zh-CN".into(),
            theme: "system".into(),
            start_hidden: false,
            user_autostart: false,
            gui_autostart: false,
            check_updates: true,
            auto_update: false,
            notify_errors: true,
            health_targets: vec![
                "http://www.msftconnecttest.com/connecttest.txt".into(),
                "http://connectivitycheck.gstatic.com/generate_204".into(),
            ],
            rules: Vec::new(),
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.schema_version != 1 {
            return Err(AppError::new(ErrorCode::Incompatible, "不支持的配置版本"));
        }
        if !(5..=3600).contains(&self.status_interval_secs)
            || !(30..=86400).contains(&self.health_interval_secs)
            || !(5..=3600).contains(&self.retry_max_secs)
            || !(1..=60).contains(&self.request_timeout_secs)
            || !(1..=30).contains(&self.log_retention_days)
            || !(1..=365).contains(&self.history_retention_days)
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "检测间隔、超时或保留期限超出范围",
            ));
        }
        if !["zh-CN", "en"].contains(&self.language.as_str())
            || !["system", "light", "dark"].contains(&self.theme.as_str())
        {
            return Err(AppError::new(ErrorCode::InvalidInput, "不支持的语言或主题"));
        }
        if !["error", "warning", "info", "debug"].contains(&self.log_level.as_str()) {
            return Err(AppError::new(ErrorCode::InvalidInput, "不支持的日志级别"));
        }
        if self.rules.len() > 128 || self.health_targets.len() > 8 {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "网络规则或探测目标过多",
            ));
        }
        if self.profiles.len() > 16
            || self.profiles.iter().any(|p| {
                p.id.is_empty()
                    || p.id.len() > 64
                    || p.name.is_empty()
                    || p.name.len() > 128
                    || p.rules.len() > 128
            })
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "认证环境配置无效或过多",
            ));
        }
        let mut ids = std::collections::HashSet::new();
        if self.profiles.iter().any(|p| !ids.insert(&p.id)) {
            return Err(AppError::new(ErrorCode::InvalidInput, "认证环境标识重复"));
        }
        if self.trusted_networks.len() > 128
            || self
                .trusted_networks
                .iter()
                .any(|v| v.len() != 64 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "已确认网络记录无效或过多",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NetworkRule {
    pub id: String,
    pub name: String,
    pub interface_id: Option<String>,
    pub ssid: Option<String>,
    pub gateway: Option<Ipv4Addr>,
    pub account_id: Option<String>,
    pub ignore: bool,
    pub priority: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkProfile {
    pub id: String,
    pub name: String,
    pub gateway: GatewayConfig,
    pub interface_id: Option<String>,
    pub account_id: Option<String>,
    pub rules: Vec<NetworkRule>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountInfo {
    pub id: String,
    pub username: String,
    pub has_password: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountSecret {
    pub id: String,
    pub username: String,
    pub password: Secret,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    #[default]
    Unknown,
    NoNetwork,
    NonCampus,
    Campus,
    RoutedCampus,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum EngineState {
    #[default]
    Initializing,
    Checking,
    Idle,
    LoggingIn,
    Online,
    Retrying,
    Paused,
    NeedsCredentials,
    CredentialsUnavailable,
    AuthError,
    NetworkDegraded,
    Stopping,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Health {
    #[default]
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthStatus {
    pub internet: Health,
    pub dns: Health,
    pub ipv6: Health,
    pub checked_at: Option<u64>,
}
impl Default for HealthStatus {
    fn default() -> Self {
        Self {
            internet: Health::Unknown,
            dns: Health::Unknown,
            ipv6: Health::Unknown,
            checked_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub api_version: u32,
    pub app_version: String,
    pub scope: Scope,
    pub state: EngineState,
    pub environment: Environment,
    #[serde(default)]
    pub network_trusted: bool,
    #[serde(default)]
    pub verification_hold: bool,
    pub session: GatewaySession,
    pub health: HealthStatus,
    pub interfaces: Vec<NetworkInterface>,
    pub active_binding: Option<NetworkBinding>,
    pub settings: Settings,
    pub accounts: Vec<AccountInfo>,
    pub last_error: Option<AppError>,
    pub last_success_at: Option<u64>,
    pub next_retry_at: Option<u64>,
    pub retry_count: u32,
    pub network_generation: u64,
    pub sequence: u64,
    pub capabilities: Vec<String>,
    pub service: ServiceStatus,
}
impl Snapshot {
    pub fn new(scope: Scope) -> Self {
        Self {
            api_version: API_VERSION,
            app_version: APP_VERSION.into(),
            scope,
            state: EngineState::Initializing,
            environment: Environment::Unknown,
            network_trusted: false,
            verification_hold: false,
            session: GatewaySession::default(),
            health: HealthStatus::default(),
            interfaces: Vec::new(),
            active_binding: None,
            settings: Settings::default(),
            accounts: Vec::new(),
            last_error: None,
            last_success_at: None,
            next_retry_at: None,
            retry_count: 0,
            network_generation: 0,
            sequence: 0,
            capabilities: vec![
                "accounts".into(),
                "rules".into(),
                "history".into(),
                "diagnostics".into(),
                "network_trust".into(),
            ],
            service: ServiceStatus::default(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub enabled: bool,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "command", content = "args", rename_all = "snake_case")]
// This bounded wire enum is queued at most 24 times; keeping Settings inline
// avoids an extra allocation without materially increasing the IPC memory bound.
#[allow(clippy::large_enum_variant)]
pub enum Command {
    Status,
    Check,
    Login {
        account_id: Option<String>,
        force: bool,
        confirmed: bool,
    },
    Logout {
        confirmed: bool,
    },
    Pause,
    Resume,
    EnableAutomatic {
        expected_network: String,
    },
    TrustNetwork {
        trusted: bool,
    },
    ProfileSave {
        id: String,
        name: String,
    },
    ProfileSelect {
        id: String,
    },
    ProfileDelete {
        id: String,
    },
    AccountPut {
        id: String,
        username: String,
        password: Secret,
    },
    VerifyAccount {
        id: String,
        username: String,
        password: Secret,
        expected_network: String,
    },
    AccountDelete {
        id: String,
    },
    AccountSelect {
        id: String,
    },
    SettingsPut {
        settings: Settings,
        expected_revision: u64,
    },
    History {
        limit: u32,
    },
    Statistics,
    Diagnose,
    ExportSettings,
    Shutdown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Request {
    pub api_version: u32,
    pub request_id: String,
    pub command: Command,
}
impl Request {
    pub fn new(command: Command) -> Self {
        Self {
            api_version: API_VERSION,
            request_id: format!("{}-{}", std::process::id(), now_ms()),
            command,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reply {
    pub api_version: u32,
    pub request_id: String,
    pub result: Result<ReplyData, AppError>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ReplyData {
    Snapshot(Box<Snapshot>),
    History(Vec<HistoryEvent>),
    Statistics(Statistics),
    Diagnostics(serde_json::Value),
    Settings(Box<Settings>),
    Ok { message: String },
}
impl ReplyData {
    pub fn snapshot(self) -> Result<Snapshot, AppError> {
        match self {
            Self::Snapshot(s) => Ok(*s),
            _ => Err(AppError::new(ErrorCode::Internal, "返回类型不匹配")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEvent {
    pub id: i64,
    pub at: u64,
    pub kind: String,
    pub message: String,
    pub account: Option<String>,
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Statistics {
    pub login_successes: u64,
    pub login_failures: u64,
    pub reconnects: u64,
    pub last_success_at: Option<u64>,
    pub average_recovery_ms: Option<u64>,
    pub events: u64,
    pub observed_online_seconds: u64,
    pub longest_observed_online_seconds: u64,
    pub session_interruptions: u64,
    pub health_anomalies: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidInput,
    PermissionDenied,
    Busy,
    Network,
    GatewayUnavailable,
    Credentials,
    CredentialsUnavailable,
    ConfirmationRequired,
    Incompatible,
    Unavailable,
    Cancelled,
    Internal,
}
impl ErrorCode {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::InvalidInput => 2,
            Self::PermissionDenied => 3,
            Self::Busy => 4,
            Self::Network | Self::GatewayUnavailable => 5,
            Self::Credentials | Self::CredentialsUnavailable => 6,
            Self::ConfirmationRequired => 7,
            Self::Incompatible => 8,
            Self::Unavailable => 9,
            Self::Cancelled => 10,
            Self::Internal => 1,
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
#[error("{message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    pub retryable: bool,
}
impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            retryable: matches!(
                code,
                ErrorCode::Network | ErrorCode::GatewayUnavailable | ErrorCode::Busy
            ),
        }
    }
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// A bounded exponential schedule. Explicit credential errors never call this.
pub fn retry_delay(attempt: u32, max_secs: u64, jitter_percent: u64) -> u64 {
    let base = 5u64.saturating_mul(
        1u64.checked_shl(attempt.saturating_sub(1).min(16))
            .unwrap_or(u64::MAX),
    );
    let max = max_secs.max(5);
    base.min(max)
        .saturating_add(base.min(max) * jitter_percent.min(20) / 100)
        .min(max)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secrets_are_not_debugged() {
        let s = Secret::new("do-not-print");
        assert!(!format!("{s:?}").contains(s.expose()));
    }
    #[test]
    fn retry_is_bounded_and_keeps_retrying() {
        assert_eq!(
            (1..=8).map(|n| retry_delay(n, 300, 0)).collect::<Vec<_>>(),
            vec![5, 10, 20, 40, 80, 160, 300, 300]
        );
        assert_eq!(retry_delay(u32::MAX, 300, 20), 300);
    }
    #[test]
    fn settings_reject_unbounded_requests() {
        let s = Settings {
            status_interval_secs: 0,
            ..Default::default()
        };
        assert!(s.validate().is_err());
    }
    #[test]
    fn snapshot_never_contains_password() {
        let encoded = serde_json::to_string(&Snapshot::new(Scope::User)).unwrap();
        assert!(!encoded.contains("password\":"));
    }
}
