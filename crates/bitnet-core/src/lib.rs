//! Shared, transport-independent application contract. No GUI or OS dependencies.
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    collections::BTreeMap,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ipv4Assignment {
    Dhcp,
    AutoPending,
    Manual,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DnsMode {
    Automatic,
    Manual,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DnsConfiguration {
    pub servers: Vec<IpAddr>,
    pub ipv4_mode: Option<DnsMode>,
    pub ipv6_mode: Option<DnsMode>,
}

/// Duplicate address detection state reported by the operating system.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6AddressState {
    #[default]
    Unknown,
    Preferred,
    Deprecated,
    Tentative,
    Duplicate,
    Invalid,
}

/// Where a prefix or an interface identifier came from.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Origin {
    #[default]
    Unknown,
    Other,
    Manual,
    WellKnown,
    Dhcp,
    RouterAdvertisement,
    LinkLayerAddress,
    Random,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Scope {
    #[default]
    Unknown,
    Loopback,
    LinkLocal,
    UniqueLocal,
    Global,
}

impl Ipv6Scope {
    /// Scope implied by the address block itself. Only the block is described.
    pub fn of(address: Ipv6Addr) -> Self {
        if address.is_loopback() {
            Self::Loopback
        } else if address.is_unicast_link_local() {
            Self::LinkLocal
        } else if address.is_unique_local() {
            Self::UniqueLocal
        } else if is_public_ipv6(address) {
            Self::Global
        } else {
            Self::Unknown
        }
    }
}

/// Remaining address lifetime. `Unknown` is not the same as a lifetime of zero.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Lifetime {
    #[default]
    Unknown,
    Infinite,
    Seconds(u32),
}

impl Ipv6Lifetime {
    /// IP Helper reports an all-ones lifetime for addresses that never expire.
    pub fn from_seconds(seconds: u32) -> Self {
        if seconds == u32::MAX {
            Self::Infinite
        } else {
            Self::Seconds(seconds)
        }
    }
}

/// Presentation class of a local address, derived from its reported state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ipv6AddressPresentation {
    Preferred,
    TemporaryPreferred,
    Deprecated,
    Tentative,
    Duplicate,
    Invalid,
    Unknown,
}

/// One unicast IPv6 address on an adapter. Missing facts stay unknown rather
/// than being inferred from the address text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ipv6AddressInfo {
    pub address: Ipv6Addr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_length: Option<u8>,
    #[serde(default)]
    pub state: Ipv6AddressState,
    #[serde(default)]
    pub prefix_origin: Ipv6Origin,
    #[serde(default)]
    pub suffix_origin: Ipv6Origin,
    #[serde(default)]
    pub preferred_lifetime: Ipv6Lifetime,
    #[serde(default)]
    pub valid_lifetime: Ipv6Lifetime,
    #[serde(default, skip_serializing_if = "is_false")]
    pub temporary: bool,
    #[serde(default)]
    pub scope: Ipv6Scope,
}

impl Ipv6AddressInfo {
    /// An address whose additional properties could not be read.
    pub fn unknown(address: Ipv6Addr) -> Self {
        Self {
            address,
            prefix_length: None,
            state: Ipv6AddressState::Unknown,
            prefix_origin: Ipv6Origin::Unknown,
            suffix_origin: Ipv6Origin::Unknown,
            preferred_lifetime: Ipv6Lifetime::Unknown,
            valid_lifetime: Ipv6Lifetime::Unknown,
            temporary: false,
            scope: Ipv6Scope::Unknown,
        }
    }

    pub fn presentation(&self) -> Ipv6AddressPresentation {
        match self.state {
            Ipv6AddressState::Preferred if self.temporary => {
                Ipv6AddressPresentation::TemporaryPreferred
            }
            Ipv6AddressState::Preferred => Ipv6AddressPresentation::Preferred,
            Ipv6AddressState::Deprecated => Ipv6AddressPresentation::Deprecated,
            Ipv6AddressState::Tentative => Ipv6AddressPresentation::Tentative,
            Ipv6AddressState::Duplicate => Ipv6AddressPresentation::Duplicate,
            Ipv6AddressState::Invalid => Ipv6AddressPresentation::Invalid,
            Ipv6AddressState::Unknown => Ipv6AddressPresentation::Unknown,
        }
    }
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
    /// IPv4 DNS servers in the operating system's preference order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<Ipv4Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_config: Option<DnsConfiguration>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv6: Vec<Ipv6Addr>,
    /// Per-address IPv6 facts. Older snapshots omit this list entirely.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv6_details: Vec<Ipv6AddressInfo>,
    /// Configured IPv4 assignment method, when available for this adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv4_mode: Option<Ipv4Assignment>,
    /// Whether the operating system reports IPv6 enabled for this adapter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6_enabled: Option<bool>,
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

pub fn is_public_ipv6(address: Ipv6Addr) -> bool {
    let [first, second, ..] = address.segments();
    // Global unicast space, excluding documentation prefixes.
    first & 0xe000 == 0x2000
        && (first, second) != (0x2001, 0x0db8)
        && !(first == 0x3fff && second & 0xf000 == 0)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EducationNetworkV6 {
    Cernet,
    Cernet2,
    Fiti,
}

/// Recognizable address allocations. This is a display hint, not a route or trust decision.
pub fn education_network_ipv6(address: Ipv6Addr) -> Option<EducationNetworkV6> {
    let [first, second, ..] = address.segments();
    match (first, second) {
        (0x2001, 0x0250 | 0x0251) => Some(EducationNetworkV6::Cernet),
        (0x2001, 0x0da8) | (0x240c, 0xc000..=0xcfff) => Some(EducationNetworkV6::Cernet2),
        (0x240a, 0xa000..=0xafff) => Some(EducationNetworkV6::Fiti),
        _ => None,
    }
}

pub fn is_cernet_ipv6(address: Ipv6Addr) -> bool {
    matches!(
        education_network_ipv6(address),
        Some(EducationNetworkV6::Cernet | EducationNetworkV6::Cernet2)
    )
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_ip: Option<Ipv4Addr>,
    #[serde(skip_serializing_if = "is_false")]
    pub dumb_terminal: bool,
}
fn is_false(value: &bool) -> bool {
    !value
}
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            base_url: "http://10.0.0.55".into(),
            ac_id: None,
            timeout_secs: 10,
            request_ip: None,
            dumb_terminal: false,
        }
    }
}

const DEFAULT_HEALTH_TEXT_URL: &str = "http://www.msftconnecttest.com/connecttest.txt";
const DEFAULT_HEALTH_STATUS_URL: &str = "http://connectivitycheck.gstatic.com/generate_204";
pub const MAX_HEALTH_TARGETS: usize = 8;
pub const MAX_IPV6_TARGETS: usize = 8;
// RFC 8484 GET for the fixed example.com AAAA question.
pub const DNS_FREE_IPV6_TARGET: &str =
    "https://[2606:4700:4700::1111]/dns-query?dns=AAABAAABAAAAAAAAB2V4YW1wbGUDY29tAAAcAAE";
const DEFAULT_IPV6_TARGETS: [&str; 3] = [
    "http://api-ipv6.ip.sb/ip",
    "http://v6.test-ipv6.com/ip/?callback=cb",
    DNS_FREE_IPV6_TARGET,
];

fn default_ipv6_targets_match(targets: &[String]) -> bool {
    targets == DEFAULT_IPV6_TARGETS
}

pub fn parse_ipv6_target(target: &str) -> Result<url::Url, AppError> {
    let url = url::Url::parse(target).ok().filter(|url| {
        target.len() <= 2048
            && matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    });
    url.ok_or_else(|| {
        AppError::new(
            ErrorCode::InvalidInput,
            "IPv6 探测地址须为不含凭据和片段的 HTTP 或 HTTPS 地址",
        )
    })
}

fn validate_ipv6_targets(targets: &[String]) -> Result<(), AppError> {
    if targets.len() > MAX_IPV6_TARGETS {
        return Err(AppError::new(ErrorCode::InvalidInput, "IPv6 探测地址过多"));
    }
    let mut seen = std::collections::BTreeSet::new();
    for target in targets {
        if !seen.insert(parse_ipv6_target(target)?) {
            return Err(AppError::new(ErrorCode::InvalidInput, "IPv6 探测地址重复"));
        }
    }
    Ok(())
}

pub const MAX_EDUCATION_PROBES: usize = 8;

/// A measurement point that checks IPv6 reachability of education networks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EducationProbe {
    /// Stable identifier. Results refer to this instead of a list position.
    pub id: String,
    pub name: String,
    pub url: String,
    /// The endpoint answers with a bot challenge page before its real content.
    #[serde(default, skip_serializing_if = "is_false")]
    pub needs_challenge: bool,
}

const DEFAULT_EDUCATION_PROBES: [(&str, &str, &str); 3] = [
    ("cernet", "CERNET 门户", "https://www.cernet.edu.cn"),
    ("cernet2", "CERNET2 门户", "https://www.cernet2.edu.cn"),
    ("fiti", "FITI 门户", "https://www.fiti.edu.cn"),
];

fn default_education_probes() -> Vec<EducationProbe> {
    DEFAULT_EDUCATION_PROBES
        .iter()
        .map(|(id, name, url)| EducationProbe {
            id: (*id).into(),
            name: (*name).into(),
            url: (*url).into(),
            needs_challenge: false,
        })
        .collect()
}

fn default_education_probes_match(probes: &[EducationProbe]) -> bool {
    probes.len() == DEFAULT_EDUCATION_PROBES.len()
        && probes
            .iter()
            .zip(DEFAULT_EDUCATION_PROBES)
            .all(|(probe, (id, name, url))| {
                probe.id == id && probe.name == name && probe.url == url && !probe.needs_challenge
            })
}

fn validate_education_probes(probes: &[EducationProbe]) -> Result<(), AppError> {
    if probes.len() > MAX_EDUCATION_PROBES {
        return Err(AppError::new(ErrorCode::InvalidInput, "教育网测点过多"));
    }
    let mut ids = std::collections::BTreeSet::new();
    for probe in probes {
        if probe.id.is_empty()
            || probe.id.len() > 64
            || !probe.id.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'-' | b'_' | b'.')
            })
        {
            return Err(AppError::new(ErrorCode::InvalidInput, "教育网测点标识无效"));
        }
        if probe.name.is_empty() || probe.name.len() > 128 {
            return Err(AppError::new(ErrorCode::InvalidInput, "教育网测点名称无效"));
        }
        parse_ipv6_target(&probe.url)?;
        if !ids.insert(&probe.id) {
            return Err(AppError::new(ErrorCode::InvalidInput, "教育网测点标识重复"));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum HealthExpect {
    #[default]
    AnySuccess,
    Status(u16),
    BodyEquals(String),
    BodyContains(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HealthTarget {
    pub url: String,
    pub expect: HealthExpect,
}

impl HealthTarget {
    pub fn parsed_url(&self) -> Result<url::Url, AppError> {
        let url = url::Url::parse(&self.url).ok().filter(|url| {
            self.url.len() <= 2048
                && matches!(url.scheme(), "http" | "https")
                && url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none()
        });
        url.ok_or_else(|| {
            AppError::new(
                ErrorCode::InvalidInput,
                "探测地址须为不含凭据的 HTTP 或 HTTPS 地址",
            )
        })
    }

    fn validate(&self) -> Result<(), AppError> {
        self.parsed_url()?;
        let valid = match &self.expect {
            HealthExpect::AnySuccess => true,
            HealthExpect::Status(code) => (100..=599).contains(code),
            HealthExpect::BodyEquals(body) | HealthExpect::BodyContains(body) => {
                !body.is_empty() && body.len() <= 4096
            }
        };
        if valid {
            Ok(())
        } else {
            Err(AppError::new(
                ErrorCode::InvalidInput,
                "探测期望值无效或过长",
            ))
        }
    }
}

fn legacy_health_expectation(url: &str) -> HealthExpect {
    match url {
        DEFAULT_HEALTH_TEXT_URL => HealthExpect::BodyEquals("Microsoft Connect Test".into()),
        DEFAULT_HEALTH_STATUS_URL => HealthExpect::Status(204),
        _ => HealthExpect::AnySuccess,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct NetworkPermission {
    pub fingerprint: String,
    pub interface_id: String,
    pub adapter_name: String,
    pub ssid: Option<String>,
    pub allowed: bool,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub network_permissions: Vec<NetworkPermission>,
    pub profiles: Vec<NetworkProfile>,
    pub active_profile: Option<String>,
    pub interface_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interface_ssid: Option<String>,
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
    // Older BitNet versions can still read otherwise compatible default settings.
    #[serde(skip_serializing_if = "default_ipv6_targets_match")]
    pub ipv6_targets: Vec<String>,
    /// Education-network measurement points; the defaults apply when absent.
    #[serde(skip_serializing_if = "default_education_probes_match")]
    pub education_probes: Vec<EducationProbe>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub health_expectations: BTreeMap<String, HealthExpect>,
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
            network_permissions: Vec::new(),
            profiles: Vec::new(),
            active_profile: None,
            interface_id: None,
            interface_ssid: None,
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
                DEFAULT_HEALTH_TEXT_URL.into(),
                DEFAULT_HEALTH_STATUS_URL.into(),
            ],
            health_expectations: BTreeMap::new(),
            ipv6_targets: DEFAULT_IPV6_TARGETS.map(str::to_owned).into(),
            education_probes: default_education_probes(),
            rules: Vec::new(),
        }
    }
}
impl Settings {
    pub fn configured_health_targets(&self) -> Vec<HealthTarget> {
        self.health_targets
            .iter()
            .map(|url| HealthTarget {
                url: url.clone(),
                expect: self
                    .health_expectations
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| legacy_health_expectation(url)),
            })
            .collect()
    }

    pub fn set_health_targets(&mut self, targets: Vec<HealthTarget>) -> Result<(), AppError> {
        if targets.len() > MAX_HEALTH_TARGETS {
            return Err(AppError::new(ErrorCode::InvalidInput, "探测目标过多"));
        }
        let mut urls = Vec::with_capacity(targets.len());
        let mut expectations = BTreeMap::new();
        for target in targets {
            target.validate()?;
            if urls.contains(&target.url) {
                return Err(AppError::new(ErrorCode::InvalidInput, "探测地址重复"));
            }
            if target.expect != legacy_health_expectation(&target.url) {
                expectations.insert(target.url.clone(), target.expect);
            }
            urls.push(target.url);
        }
        self.health_targets = urls;
        self.health_expectations = expectations;
        Ok(())
    }

    pub fn set_ipv6_targets(&mut self, targets: Vec<String>) -> Result<(), AppError> {
        let targets: Vec<_> = targets
            .into_iter()
            .map(|url| url.trim().to_owned())
            .collect();
        validate_ipv6_targets(&targets)?;
        self.ipv6_targets = targets;
        Ok(())
    }

    pub fn set_education_probes(&mut self, probes: Vec<EducationProbe>) -> Result<(), AppError> {
        let probes: Vec<_> = probes
            .into_iter()
            .map(|probe| EducationProbe {
                id: probe.id.trim().to_owned(),
                name: probe.name.trim().to_owned(),
                url: probe.url.trim().to_owned(),
                needs_challenge: probe.needs_challenge,
            })
            .collect();
        validate_education_probes(&probes)?;
        self.education_probes = probes;
        Ok(())
    }

    pub fn education_probe(&self, id: &str) -> Option<&EducationProbe> {
        self.education_probes.iter().find(|probe| probe.id == id)
    }

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
        validate_ipv6_targets(&self.ipv6_targets)?;
        validate_education_probes(&self.education_probes)?;
        if self.rules.len() > 128
            || self.health_targets.len() > MAX_HEALTH_TARGETS
            || self.health_expectations.len() > MAX_HEALTH_TARGETS
        {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "网络规则或探测目标过多",
            ));
        }
        for (url, expect) in &self.health_expectations {
            if !self.health_targets.contains(url) {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "找不到对应的探测地址",
                ));
            }
            HealthTarget {
                url: url.clone(),
                expect: expect.clone(),
            }
            .validate()?;
        }
        if self.profiles.len() > 16
            || self.profiles.iter().any(|p| {
                p.id.is_empty()
                    || p.id.len() > 64
                    || p.name.is_empty()
                    || p.name.len() > 128
                    || p.rules.len() > 128
                    || !valid_interface_ssid(&p.interface_id, &p.interface_ssid)
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
        let mut permissions = std::collections::HashSet::new();
        if self.network_permissions.len() > 128
            || self.network_permissions.iter().any(|record| {
                record.fingerprint.len() != 64
                    || !record.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
                    || !permissions.insert(&record.fingerprint)
                    || record.interface_id.is_empty()
                    || record.interface_id.len() > 512
                    || record.adapter_name.len() > 512
                    || record
                        .ssid
                        .as_ref()
                        .is_some_and(|ssid| ssid.is_empty() || ssid.len() > 128)
            })
            || !valid_interface_ssid(&self.interface_id, &self.interface_ssid)
        {
            return Err(AppError::new(ErrorCode::InvalidInput, "网络记录无效或过多"));
        }
        Ok(())
    }
}

fn valid_interface_ssid(interface_id: &Option<String>, ssid: &Option<String>) -> bool {
    ssid.as_ref()
        .is_none_or(|ssid| interface_id.is_some() && !ssid.is_empty() && ssid.len() <= 128)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interface_ssid: Option<String>,
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
pub struct HealthProbeObservation {
    pub target: usize,
    pub expect: HealthProbeExpectation,
    pub outcome: ProbeOutcome,
    pub status: Option<u16>,
    pub body_match: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_ipv6: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_selected_interface: Option<bool>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HealthProbeExpectation {
    AnySuccess,
    Status(u16),
    BodyEquals,
    BodyContains,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeOutcome {
    Match,
    UnexpectedStatus,
    UnexpectedBody,
    InvalidTarget,
    Timeout,
    RequestFailed,
    BodyLimit,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DnsProbeResult {
    Ok,
    Timeout,
    Empty,
    LookupFailed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DnsProbeObservation {
    pub host: String,
    pub result: DnsProbeResult,
    pub addresses: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_addresses: Vec<IpAddr>,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6ProbeKind {
    #[default]
    Address,
    TestIpv6,
    DualStack,
    LargePacket,
    /// An education-network measurement point; it confirms reachability, not
    /// necessarily a public IPv6 echo.
    Education,
}

/// What a probe response contained. Challenge pages never count as success.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6EchoKind {
    /// A complete public IPv6 address.
    PublicAddress,
    /// A complete address outside global unicast: IPv4, ULA, link-local.
    NonPublicAddress,
    /// An address-looking string that is not a complete address.
    Truncated,
    /// A body with neither an address nor a recognizable challenge page.
    Invalid,
    /// A bot or JavaScript challenge page instead of the echo.
    Challenge,
    /// The request timed out before any body arrived.
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ipv6ProbeObservation {
    pub target: usize,
    #[serde(default)]
    pub kind: Ipv6ProbeKind,
    /// Stable identifier of the configured education-network probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<ProbeOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<Ipv6EchoKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reported_address: Option<Ipv6Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_result: Option<DnsProbeResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addresses: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resolved_addresses: Vec<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_address: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer_address: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub non_public_target: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub reported_ipv4: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub literal_https_reachable: bool,
    pub attempted: usize,
    pub succeeded: usize,
    pub local_is_v6: Option<bool>,
    pub timed_out: bool,
    pub elapsed_ms: u64,
}

impl Ipv6ProbeObservation {
    pub fn confirmed_address(&self) -> Option<Ipv6Addr> {
        self.reported_address.filter(|address| {
            self.succeeded > 0
                && self
                    .outcome
                    .is_none_or(|outcome| outcome == ProbeOutcome::Match)
                && self
                    .echo
                    .is_none_or(|echo| echo == Ipv6EchoKind::PublicAddress)
                && is_public_ipv6(*address)
        })
    }

    pub fn is_education(&self) -> bool {
        self.kind == Ipv6ProbeKind::Education
    }

    pub fn dual_stack_reported_ipv4(&self) -> bool {
        self.kind == Ipv6ProbeKind::DualStack
            && self.succeeded > 0
            && self.reported_address.is_none()
            && (self.reported_ipv4 || self.local_is_v6 == Some(false))
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Ipv6Status {
    #[default]
    Unknown,
    Disabled,
    LinkLocalOnly,
    GlobalUnreachable,
    GlobalNotPreferred,
    Normal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HealthStatus {
    pub internet: Health,
    pub dns: Health,
    pub ipv6: Health,
    #[serde(default)]
    pub ipv6_status: Ipv6Status,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6_checked_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_ipv6: Option<Ipv6Addr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_ipv6: Option<Ipv6Addr>,
    pub checked_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub health_probes: Vec<HealthProbeObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_probe: Option<DnsProbeObservation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ipv6_probes: Vec<Ipv6ProbeObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dns_detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipv6_detail: Option<String>,
}

impl HealthStatus {
    pub fn confirmed_ipv6(&self) -> Option<Ipv6Addr> {
        self.ipv6_probes
            .iter()
            .find_map(Ipv6ProbeObservation::confirmed_address)
    }

    /// Probes that measure IPv6 internet reachability. Education-network
    /// measurement points are informational and never decide this verdict.
    pub fn connectivity_ipv6_probes(&self) -> impl Iterator<Item = &Ipv6ProbeObservation> {
        self.ipv6_probes
            .iter()
            .filter(|probe| !probe.is_education())
    }

    pub fn observed_ipv6_status(&self) -> Ipv6Status {
        if self.confirmed_ipv6().is_some()
            || self
                .ipv6_probes
                .iter()
                .any(|probe| probe.literal_https_reachable)
        {
            return if self
                .ipv6_probes
                .iter()
                .any(Ipv6ProbeObservation::dual_stack_reported_ipv4)
            {
                Ipv6Status::GlobalNotPreferred
            } else {
                Ipv6Status::Normal
            };
        }
        let ipv6_only = || {
            self.connectivity_ipv6_probes()
                .filter(|p| p.kind != Ipv6ProbeKind::DualStack)
        };
        let http_replied = self
            .connectivity_ipv6_probes()
            .any(|p| p.local_is_v6 == Some(true))
            || ipv6_only().any(|p| p.status.is_some());
        if !http_replied && ipv6_only().any(|p| p.attempted > 0 && !p.non_public_target) {
            return Ipv6Status::GlobalUnreachable;
        }
        if http_replied || ipv6_only().any(|p| p.non_public_target) {
            return Ipv6Status::Unknown;
        }
        match self.ipv6_status {
            Ipv6Status::Disabled | Ipv6Status::LinkLocalOnly => self.ipv6_status,
            _ => Ipv6Status::Unknown,
        }
    }

    pub fn redacted(&self) -> Self {
        let mut state = self.clone();
        state.local_ipv6 = None;
        state.public_ipv6 = None;
        if let Some(dns) = &mut state.dns_probe {
            dns.resolved_addresses.clear();
        }
        for probe in &mut state.ipv6_probes {
            probe.reported_address = None;
            probe.resolved_addresses.clear();
            probe.local_address = None;
            probe.peer_address = None;
        }
        state
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
    ProbeIpv6,
    Login {
        account_id: Option<String>,
        force: bool,
        confirmed: bool,
    },
    LoginOnce {
        username: String,
        password: Secret,
        expected_network: String,
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
    SetNetworkPermission {
        expected_network: String,
        allow: bool,
    },
    ForgetNetworkPermission {
        fingerprint: String,
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
    SwitchAccount {
        id: String,
        expected_network: String,
        expected_scope: Scope,
        expected_revision: u64,
        confirmed: bool,
    },
    SettingsPut {
        settings: Settings,
        expected_revision: u64,
    },
    History {
        limit: u32,
    },
    HistoryBefore {
        limit: u32,
        before_id: i64,
    },
    LogSize,
    ClearLogs {
        expected_scope: Scope,
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
    LogSize { scope: Scope, bytes: u64 },
    LogsCleared { scope: Scope, bytes: u64 },
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
    fn education_address_hints_respect_allocation_boundaries() {
        use EducationNetworkV6::*;
        for (address, expected) in [
            ("2001:da8:280::1", Some(Cernet2)),
            ("2001:250::1", Some(Cernet)),
            ("2001:251::1", Some(Cernet)),
            ("240c:c000::1", Some(Cernet2)),
            ("240c:cfff:ffff::1", Some(Cernet2)),
            ("240a:afff::1", Some(Fiti)),
            ("240c::1", None),
            ("240c:d000::1", None),
            ("2001:daa::1", None),
            ("fdfe:dcba:9876::1", None),
        ] {
            assert_eq!(education_network_ipv6(address.parse().unwrap()), expected);
        }
    }

    #[test]
    fn ipv6_status_uses_observations_even_with_a_legacy_normal_status() {
        let mut health = HealthStatus {
            ipv6_status: Ipv6Status::Normal,
            local_ipv6: Some("2001:da8::1".parse().unwrap()),
            ipv6_probes: vec![serde_json::from_value(serde_json::json!({
                "target": 1, "attempted": 1, "succeeded": 0, "timed_out": true,
                "elapsed_ms": 5000, "outcome": "timeout"
            }))
            .unwrap()],
            ..HealthStatus::default()
        };
        assert_eq!(health.observed_ipv6_status(), Ipv6Status::GlobalUnreachable);
        health.ipv6_probes[0] = serde_json::from_value(serde_json::json!({
            "target": 1, "attempted": 0, "succeeded": 0, "timed_out": false,
            "elapsed_ms": 10, "non_public_target": true
        }))
        .unwrap();
        assert_eq!(health.observed_ipv6_status(), Ipv6Status::Unknown);
        health.ipv6_probes.push(serde_json::from_value(serde_json::json!({
            "target": 3, "attempted": 1, "succeeded": 1, "timed_out": false,
            "elapsed_ms": 100, "status": 200, "outcome": "match", "literal_https_reachable": true
        })).unwrap());
        assert_eq!(health.observed_ipv6_status(), Ipv6Status::Normal);
        assert_eq!(health.confirmed_ipv6(), None);
    }

    #[test]
    fn dual_stack_preference_uses_the_echo_instead_of_the_proxy_socket() {
        let mut probe: Ipv6ProbeObservation = serde_json::from_value(serde_json::json!({
            "target": 1, "kind": "dual_stack", "attempted": 1, "succeeded": 1,
            "timed_out": false, "elapsed_ms": 10, "outcome": "match",
            "reported_address": "2001:4860::1234", "local_is_v6": false
        }))
        .unwrap();
        assert!(probe.confirmed_address().is_some());
        assert!(!probe.dual_stack_reported_ipv4());
        probe.reported_address = None;
        probe.local_is_v6 = Some(true);
        probe.reported_ipv4 = true;
        assert!(probe.confirmed_address().is_none());
        assert!(probe.dual_stack_reported_ipv4());
    }

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
    fn ipv6_target_defaults_preserve_legacy_configuration_and_accept_explicit_overrides() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.ipv6_targets, DEFAULT_IPV6_TARGETS);
        assert!(serde_json::to_value(&settings).unwrap()["ipv6_targets"].is_null());

        settings
            .set_ipv6_targets(vec![" https://ipv6.example/ip  ".into()])
            .unwrap();
        assert_eq!(settings.ipv6_targets, ["https://ipv6.example/ip"]);
        let serialized = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            serialized["ipv6_targets"],
            serde_json::json!(["https://ipv6.example/ip"])
        );
        let decoded: Settings = serde_json::from_value(serialized).unwrap();
        assert_eq!(settings, decoded);

        for invalid in [
            vec![
                "https://ipv6.example/ip".into(),
                "https://ipv6.example/ip".into(),
            ],
            vec!["https://person@ipv6.example/ip".into()],
            vec!["https://ipv6.example/ip#section".into()],
            vec!["file:///tmp/ipv6".into()],
            (0..=MAX_IPV6_TARGETS)
                .map(|i| format!("https://ipv6.example/{i}"))
                .collect(),
        ] {
            assert!(settings.set_ipv6_targets(invalid).is_err());
            assert_eq!(settings.ipv6_targets, ["https://ipv6.example/ip"]);
        }

        settings.set_ipv6_targets(Vec::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&settings).unwrap()["ipv6_targets"],
            serde_json::json!([])
        );
    }
    #[test]
    fn health_target_expectations_round_trip_without_changing_legacy_defaults() {
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "health_targets": [DEFAULT_HEALTH_TEXT_URL, DEFAULT_HEALTH_STATUS_URL, "https://fixture.test/check"]
        }))
        .unwrap();
        let mut targets = settings.configured_health_targets();
        assert_eq!(
            targets[0].expect,
            HealthExpect::BodyEquals("Microsoft Connect Test".into())
        );
        assert_eq!(targets[1].expect, HealthExpect::Status(204));
        assert_eq!(targets[2].expect, HealthExpect::AnySuccess);
        assert!(serde_json::to_value(&settings)
            .unwrap()
            .get("health_expectations")
            .is_none());

        targets[1].expect = HealthExpect::AnySuccess;
        targets[2].expect = HealthExpect::BodyContains("ready".into());
        settings.set_health_targets(targets.clone()).unwrap();
        let restored: Settings =
            serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
        assert_eq!(restored.configured_health_targets(), targets);

        targets[0].url = "https://user:password@fixture.test/check".into();
        assert!(settings.set_health_targets(targets).is_err());
        assert_eq!(settings, restored);
    }
    #[test]
    fn snapshot_never_contains_password() {
        let encoded = serde_json::to_string(&Snapshot::new(Scope::User)).unwrap();
        assert!(!encoded.contains("password\":"));
    }
    #[test]
    fn earlier_adapter_snapshots_keep_missing_facts_unknown() {
        let adapter: NetworkInterface = serde_json::from_value(serde_json::json!({
            "id": "ethernet",
            "name": "Ethernet",
            "index": 5,
            "ipv4": ["10.1.2.3"],
            "gateway": "10.1.2.1",
            "up": true,
            "wireless": false,
            "virtual_adapter": false
        }))
        .unwrap();

        assert!(adapter.binding().is_some());
        assert!(adapter.dns.is_empty());
        assert!(adapter.ipv6.is_empty());
        assert!(adapter.ipv6_details.is_empty());
        assert_eq!(adapter.ipv4_mode, None);
        assert_eq!(adapter.ipv6_enabled, None);
    }

    #[test]
    fn ipv6_address_details_round_trip_and_stay_optional() {
        let mut adapter: NetworkInterface = serde_json::from_value(serde_json::json!({
            "id": "ethernet",
            "name": "Ethernet",
            "index": 5,
            "ipv4": ["10.1.2.3"],
            "up": true,
            "wireless": false,
            "virtual_adapter": false
        }))
        .unwrap();
        assert!(adapter.ipv6_details.is_empty());
        assert!(serde_json::to_value(&adapter)
            .unwrap()
            .get("ipv6_details")
            .is_none());

        adapter.ipv6_details = vec![
            Ipv6AddressInfo {
                address: "2001:da8:204::1".parse().unwrap(),
                prefix_length: Some(64),
                state: Ipv6AddressState::Preferred,
                prefix_origin: Ipv6Origin::RouterAdvertisement,
                suffix_origin: Ipv6Origin::LinkLayerAddress,
                preferred_lifetime: Ipv6Lifetime::Seconds(3600),
                valid_lifetime: Ipv6Lifetime::Infinite,
                temporary: false,
                scope: Ipv6Scope::Global,
            },
            Ipv6AddressInfo::unknown("fdfe:dcba:9876::1".parse().unwrap()),
        ];
        let encoded = serde_json::to_value(&adapter).unwrap();
        assert_eq!(
            encoded["ipv6_details"][0],
            serde_json::json!({
                "address": "2001:da8:204::1",
                "prefix_length": 64,
                "state": "preferred",
                "prefix_origin": "router_advertisement",
                "suffix_origin": "link_layer_address",
                "preferred_lifetime": {"seconds": 3600},
                "valid_lifetime": "infinite",
                "scope": "global"
            })
        );
        assert_eq!(encoded["ipv6_details"][1]["state"], "unknown");
        assert_eq!(encoded["ipv6_details"][1]["preferred_lifetime"], "unknown");
        assert!(encoded["ipv6_details"][1].get("temporary").is_none());
        let decoded: NetworkInterface = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded.ipv6_details, adapter.ipv6_details);
    }

    #[test]
    fn ipv6_address_presentation_separates_dad_states() {
        use Ipv6AddressPresentation::*;
        let address = "2001:da8:204::1".parse().unwrap();
        let mut info = Ipv6AddressInfo::unknown(address);
        assert_eq!(info.presentation(), Unknown);
        for (state, temporary, expected) in [
            (Ipv6AddressState::Preferred, false, Preferred),
            (Ipv6AddressState::Preferred, true, TemporaryPreferred),
            (Ipv6AddressState::Deprecated, false, Deprecated),
            (Ipv6AddressState::Tentative, false, Tentative),
            (Ipv6AddressState::Duplicate, false, Duplicate),
            (Ipv6AddressState::Invalid, false, Invalid),
            (Ipv6AddressState::Unknown, false, Unknown),
        ] {
            info.state = state;
            info.temporary = temporary;
            assert_eq!(info.presentation(), expected, "{state:?}");
        }
        for (address, expected) in [
            ("2001:da8:204::1", Ipv6Scope::Global),
            ("fe80::1", Ipv6Scope::LinkLocal),
            ("fdfe:dcba:9876::1", Ipv6Scope::UniqueLocal),
            ("::1", Ipv6Scope::Loopback),
            ("ff02::1", Ipv6Scope::Unknown),
        ] {
            assert_eq!(Ipv6Scope::of(address.parse().unwrap()), expected);
        }
        assert_eq!(Ipv6Lifetime::from_seconds(u32::MAX), Ipv6Lifetime::Infinite);
        assert_eq!(Ipv6Lifetime::from_seconds(0), Ipv6Lifetime::Seconds(0));
        assert_eq!(Ipv6Lifetime::default(), Ipv6Lifetime::Unknown);
    }

    #[test]
    fn education_probe_group_keeps_legacy_defaults_and_validates_overrides() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(settings.education_probes, default_education_probes());
        assert!(serde_json::to_value(&settings)
            .unwrap()
            .get("education_probes")
            .is_none());
        assert!(settings.validate().is_ok());
        assert_eq!(
            settings.education_probe("cernet2").map(|p| p.url.as_str()),
            Some("https://www.cernet2.edu.cn")
        );

        let grouped = vec![EducationProbe {
            id: "lab".into(),
            name: "实验室测点".into(),
            url: "https://probe.example/ip".into(),
            needs_challenge: true,
        }];
        settings.set_education_probes(grouped.clone()).unwrap();
        let encoded = serde_json::to_value(&settings).unwrap();
        assert_eq!(encoded["education_probes"][0]["needs_challenge"], true);
        assert_eq!(
            serde_json::from_value::<Settings>(encoded).unwrap(),
            settings
        );

        for invalid in [
            vec![
                EducationProbe {
                    id: "lab".into(),
                    ..grouped[0].clone()
                },
                EducationProbe {
                    id: "lab".into(),
                    name: "重复".into(),
                    url: "https://probe.example/other".into(),
                    needs_challenge: false,
                },
            ],
            vec![EducationProbe {
                id: "BAD ID".into(),
                ..grouped[0].clone()
            }],
            vec![EducationProbe {
                name: String::new(),
                ..grouped[0].clone()
            }],
            vec![EducationProbe {
                url: "https://user:secret@probe.example/ip".into(),
                ..grouped[0].clone()
            }],
            (0..=MAX_EDUCATION_PROBES)
                .map(|index| EducationProbe {
                    id: format!("probe-{index}"),
                    name: format!("probe {index}"),
                    url: format!("https://probe.example/{index}"),
                    needs_challenge: false,
                })
                .collect(),
        ] {
            assert!(settings.set_education_probes(invalid).is_err());
            assert_eq!(settings.education_probes, grouped);
        }

        settings.set_education_probes(Vec::new()).unwrap();
        assert_eq!(
            serde_json::to_value(&settings).unwrap()["education_probes"],
            serde_json::json!([])
        );
        assert!(settings.validate().is_ok());
    }

    #[test]
    fn education_probes_never_decide_the_connectivity_verdict() {
        let education = serde_json::from_value(serde_json::json!({
            "target": 1, "kind": "education", "probe_id": "cernet",
            "attempted": 1, "succeeded": 1, "timed_out": false, "elapsed_ms": 30,
            "status": 200, "outcome": "match", "echo": "invalid", "local_is_v6": false
        }))
        .unwrap();
        let health = HealthStatus {
            ipv6_probes: vec![education],
            ..HealthStatus::default()
        };
        assert_eq!(health.observed_ipv6_status(), Ipv6Status::Unknown);
        assert_eq!(health.confirmed_ipv6(), None);

        let challenge: Ipv6ProbeObservation = serde_json::from_value(serde_json::json!({
            "target": 1, "kind": "education", "probe_id": "fiti",
            "attempted": 1, "succeeded": 0, "timed_out": false, "elapsed_ms": 30,
            "status": 200, "outcome": "unexpected_body", "echo": "challenge",
            "reported_address": "2001:da8:204::1", "local_is_v6": true
        }))
        .unwrap();
        assert_eq!(challenge.confirmed_address(), None);
        assert!(challenge.is_education());
    }
}
