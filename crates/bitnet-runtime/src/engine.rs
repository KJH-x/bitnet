use crate::{
    history::History,
    ipc::{Caller, Envelope},
    persistence,
};
use bitnet_core::*;
use bitnet_gateway::GatewayClient;
use bitnet_platform::{credentials::CredentialStore, AppPaths};
use fs2::FileExt;
use std::{
    fs::{File, OpenOptions},
    io::Write,
    net::Ipv4Addr,
    time::Duration,
};
use tokio::sync::{mpsc, watch};

const SYSTEM_AUTHENTICATION_MESSAGE: &str = "BIT-mobile 由系统认证，无需校园门户登录或注销";
const VERIFICATION_STEP_TIMEOUT_SECS: u64 = 4;

enum Vault {
    Real(CredentialStore),
    Simulation,
}
pub struct Engine {
    snapshot: Snapshot,
    paths: AppPaths,
    accounts: Vec<AccountSecret>,
    vault: Option<Vault>,
    gateway: GatewayClient,
    history: Option<History>,
    simulation: bool,
    next_check: u64,
    next_health: u64,
    failed_credential: Option<String>,
    publish: watch::Sender<Snapshot>,
    simulation_session: GatewaySession,
    stopping: bool,
    domain_lock: Option<crate::controller::DomainLock>,
    domain_key: Option<String>,
    recovery_started: Option<std::time::Instant>,
    stable_checks: u32,
    online_window: Option<(std::time::Instant, std::time::Instant)>,
    network_changes: Option<watch::Receiver<u64>>,
}

impl Engine {
    pub fn open(
        scope: Scope,
        simulation: bool,
        publish: watch::Sender<Snapshot>,
    ) -> Result<Self, AppError> {
        let paths = bitnet_platform::paths(scope)?;
        Self::open_in(scope, simulation, publish, paths)
    }
    fn open_in(
        scope: Scope,
        simulation: bool,
        publish: watch::Sender<Snapshot>,
        paths: AppPaths,
    ) -> Result<Self, AppError> {
        std::fs::create_dir_all(&paths.data_dir)
            .map_err(|_| AppError::new(ErrorCode::PermissionDenied, "无法创建状态目录"))?;
        let settings = persistence::read_settings(&paths.config_file)?;
        let gateway =
            GatewayClient::new(settings.gateway.clone()).map_err(|e| e.into_app_error())?;
        let mut snapshot = Snapshot::new(scope);
        snapshot.settings = settings;
        snapshot.service = bitnet_platform::service::status();
        snapshot.capabilities.push("enable_automatic".into());
        snapshot.capabilities.push("verify_account".into());
        snapshot.capabilities.push("login_once".into());
        let (vault, accounts) = if simulation {
            snapshot.capabilities.push("simulation".into());
            (
                Some(Vault::Simulation),
                vec![AccountSecret {
                    id: "demo".into(),
                    username: "demo".into(),
                    password: Secret::new("simulation-only"),
                }],
            )
        } else {
            match CredentialStore::open(scope, &paths.data_dir) {
                Ok(store) => {
                    let accounts = match store.load() {
                        Ok(a) => a,
                        Err(e) => {
                            snapshot.last_error = Some(e);
                            snapshot.state = EngineState::CredentialsUnavailable;
                            Vec::new()
                        }
                    };
                    (Some(Vault::Real(store)), accounts)
                }
                Err(e) => {
                    snapshot.last_error = Some(e);
                    snapshot.state = EngineState::CredentialsUnavailable;
                    (None, Vec::new())
                }
            }
        };
        let history = History::open(&paths.history_file).ok();
        if let Some(h) = &history {
            let _ = h.prune(snapshot.settings.history_retention_days);
            snapshot.last_success_at = h.statistics().ok().and_then(|s| s.last_success_at);
        }
        let mut result = Self {
            snapshot,
            paths,
            accounts,
            vault,
            gateway,
            history,
            simulation,
            next_check: 0,
            next_health: 0,
            failed_credential: None,
            publish,
            simulation_session: GatewaySession {
                auth: AuthState::Unauthenticated,
                ip: Some(Ipv4Addr::new(127, 0, 0, 1)),
                ..Default::default()
            },
            stopping: false,
            domain_lock: None,
            domain_key: None,
            recovery_started: None,
            stable_checks: 0,
            online_window: None,
            network_changes: None,
        };
        result.account_metadata();
        prune_logs(
            &result.paths.log_dir,
            result.snapshot.settings.log_retention_days,
        );
        result.event(
            "engine_start",
            if simulation {
                "模拟认证引擎启动，不访问校园网"
            } else {
                "认证引擎启动"
            },
            None,
            None,
        );
        result.emit();
        Ok(result)
    }
    fn account_metadata(&mut self) {
        self.snapshot.accounts = self
            .accounts
            .iter()
            .map(|a| AccountInfo {
                id: a.id.clone(),
                username: a.username.clone(),
                has_password: !a.password.is_empty(),
            })
            .collect();
    }
    fn emit(&mut self) {
        self.snapshot.sequence = self.snapshot.sequence.wrapping_add(1);
        self.publish.send_replace(self.snapshot.clone());
    }
    fn event(&self, kind: &str, message: &str, account: Option<&str>, duration: Option<u64>) {
        let level = match kind {
            "error" | "login_failure" => "error",
            "health_degraded" | "session_lost" => "warning",
            _ => "info",
        };
        let details = serde_json::json!({"level":level,"state":self.snapshot.state,"environment":self.snapshot.environment,"auth_state":self.snapshot.session.auth,"network":network_fingerprint(&self.snapshot),"generation":self.snapshot.network_generation,"retry_count":self.snapshot.retry_count,"next_retry_at":self.snapshot.next_retry_at,"error_code":self.snapshot.last_error.as_ref().map(|e|e.code)});
        if let Some(history) = &self.history {
            let _ = history.record_details(kind, message, account, duration, &details);
        }
        if log_rank(level) > log_rank(&self.snapshot.settings.log_level) {
            return;
        }
        let value = serde_json::json!({"at":now_secs(),"level":level,"kind":kind,"message":message,"details":details});
        if !self.simulation && self.snapshot.scope == Scope::System {
            crate::os_log::write(level, message);
        }
        let _ = std::fs::create_dir_all(&self.paths.log_dir);
        let path = self.paths.log_dir.join("engine.jsonl");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 5 * 1024 * 1024) {
            for n in (1..4).rev() {
                let from = self.paths.log_dir.join(format!("engine.{n}.jsonl"));
                let to = self.paths.log_dir.join(format!("engine.{}.jsonl", n + 1));
                if to.exists() {
                    let _ = std::fs::remove_file(&to);
                }
                let _ = std::fs::rename(from, to);
            }
            let _ = std::fs::rename(&path, self.paths.log_dir.join("engine.1.jsonl"));
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
            let _ = writeln!(file, "{value}");
        }
    }
    fn close_online_window(&mut self) {
        if let Some((started, confirmed)) = self.online_window.take() {
            self.event(
                "online_segment",
                "已记录一段在线时间",
                None,
                Some(confirmed.duration_since(started).as_millis() as u64),
            );
        }
    }
    fn observe_online(&mut self) {
        let now = std::time::Instant::now();
        if self.online_window.is_some_and(|(_, last)| {
            now.duration_since(last).as_secs()
                > self
                    .snapshot
                    .settings
                    .status_interval_secs
                    .saturating_mul(2)
        }) {
            self.close_online_window();
        }
        match &mut self.online_window {
            Some((_, last)) => *last = now,
            None => self.online_window = Some((now, now)),
        }
    }
    fn persist(&mut self) -> Result<(), AppError> {
        self.snapshot.settings.revision = self.snapshot.settings.revision.saturating_add(1);
        persistence::write_settings(&self.paths.config_file, &self.snapshot.settings)
    }
    fn save_accounts(&mut self, accounts: &[AccountSecret]) -> Result<(), AppError> {
        if self.vault.is_none() {
            self.vault = Some(Vault::Real(CredentialStore::open(
                self.snapshot.scope,
                &self.paths.data_dir,
            )?));
        }
        match self.vault.as_ref().unwrap() {
            Vault::Real(store) => store.save(accounts),
            Vault::Simulation => Ok(()),
        }
    }
    fn set_error(&mut self, error: AppError) {
        self.snapshot.last_error = Some(error.clone());
        if matches!(
            error.code,
            ErrorCode::Credentials | ErrorCode::CredentialsUnavailable
        ) {
            self.snapshot.state = if error.code == ErrorCode::Credentials {
                EngineState::AuthError
            } else {
                EngineState::CredentialsUnavailable
            };
            self.snapshot.next_retry_at = None;
        } else {
            self.snapshot.retry_count = self.snapshot.retry_count.saturating_add(1);
            let delay = retry_delay(
                self.snapshot.retry_count,
                self.snapshot.settings.retry_max_secs,
                now_secs() % 21,
            );
            self.snapshot.next_retry_at = Some(now_secs() + delay);
            self.next_check = now_secs() + delay;
            self.snapshot.state = EngineState::Retrying;
        }
        self.event("error", &error.message, None, None);
    }
    fn selected(
        &self,
        binding: Option<&NetworkBinding>,
        explicit: Option<&str>,
    ) -> Result<Option<&AccountSecret>, AppError> {
        let selected = if let Some(id) = explicit {
            Some(id.to_string())
        } else {
            let interface = binding.and_then(|b| {
                self.snapshot
                    .interfaces
                    .iter()
                    .find(|i| i.id == b.interface_id)
            });
            resolve_account(&self.snapshot.settings, interface)?
        };
        Ok(selected.and_then(|id| self.accounts.iter().find(|a| a.id == id)))
    }
    async fn gateway_status(&self) -> Result<GatewaySession, AppError> {
        if self.simulation {
            Ok(self.simulation_session.clone())
        } else {
            let mut config = self.snapshot.settings.gateway.clone();
            config.timeout_secs = self.snapshot.settings.request_timeout_secs;
            GatewayClient::new(config)
                .map_err(|e| e.into_app_error())?
                .status(self.snapshot.active_binding.as_ref())
                .await
                .map_err(|e| e.into_app_error())
        }
    }
    async fn refresh_network(&mut self) -> Result<(), AppError> {
        let interfaces = if self.simulation {
            vec![NetworkInterface {
                id: "simulation".into(),
                name: "本地模拟网络 / Simulation".into(),
                index: 1,
                ipv4: vec![Ipv4Addr::LOCALHOST],
                gateway: Some(Ipv4Addr::LOCALHOST),
                up: true,
                wireless: false,
                ssid: None,
                virtual_adapter: false,
                alternate_adapter: false,
            }]
        } else {
            bitnet_platform::network::interfaces()?
        };
        let binding = if self.simulation {
            Some(NetworkBinding {
                interface_id: "simulation".into(),
                interface_index: 1,
                source_ip: Ipv4Addr::LOCALHOST,
            })
        } else if let Some(id) = &self.snapshot.settings.interface_id {
            interfaces
                .iter()
                .find(|i| &i.id == id)
                .and_then(|i| i.binding())
        } else {
            let ip = url::Url::parse(&self.snapshot.settings.gateway.base_url)
                .ok()
                .and_then(|u| u.host_str()?.parse::<Ipv4Addr>().ok())
                .ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidInput, "认证网关必须为明确的 IPv4 地址")
                })?;
            bitnet_platform::network::preferred_binding(ip)?
        };
        let network_changed =
            self.snapshot.active_binding != binding || self.snapshot.interfaces != interfaces;
        if network_changed {
            self.close_online_window();
            if self.snapshot.session.auth == AuthState::Authenticated
                && self.recovery_started.is_none()
            {
                self.recovery_started = Some(std::time::Instant::now());
            }
            self.stable_checks = 0;
            self.snapshot.network_generation = self.snapshot.network_generation.wrapping_add(1);
            self.snapshot.retry_count = 0;
            self.snapshot.next_retry_at = None;
            self.snapshot.session = GatewaySession::default();
            self.snapshot.health = HealthStatus::default();
        }
        self.snapshot.interfaces = interfaces;
        self.snapshot.active_binding = binding;
        if network_changed {
            self.event(
                "network_change",
                "网络接口或出口发生变化，重新评估认证环境",
                None,
                None,
            );
        }
        let system_authentication = uses_system_authentication(&self.snapshot);
        if !self.simulation {
            let key = self.snapshot.active_binding.as_ref().and_then(|binding| {
                (!system_authentication).then(|| {
                    crate::controller::domain_key(&self.snapshot.settings.gateway.base_url, binding)
                })
            });
            if self.domain_key != key {
                // Acquire before publishing a usable binding. A contending
                // engine remains available over IPC for diagnosis and repair.
                let reservation = key
                    .as_ref()
                    .map(|key| crate::controller::DomainLock::acquire(key))
                    .transpose()?;
                self.domain_lock = reservation;
                self.domain_key = key;
            }
        }
        self.snapshot.network_trusted = !system_authentication
            && (self.simulation
                || network_fingerprint(&self.snapshot)
                    .is_some_and(|key| self.snapshot.settings.trusted_networks.contains(&key)));
        if self.snapshot.active_binding.is_none() {
            self.snapshot.environment = if self
                .snapshot
                .interfaces
                .iter()
                .any(|i| i.up && !i.ipv4.is_empty())
            {
                Environment::Unknown
            } else {
                Environment::NoNetwork
            };
            self.snapshot.state = EngineState::Idle;
        }
        Ok(())
    }
    pub async fn check(&mut self, allow_login: bool) -> Result<(), AppError> {
        self.check_network(allow_login, false).await
    }
    async fn check_network(
        &mut self,
        allow_login: bool,
        explicit_login: bool,
    ) -> Result<(), AppError> {
        self.snapshot.service = bitnet_platform::service::status();
        self.next_check = now_secs() + self.snapshot.settings.status_interval_secs;
        self.refresh_network().await?;
        if self.snapshot.active_binding.is_none() {
            self.emit();
            return Ok(());
        }
        let system_authentication = uses_system_authentication(&self.snapshot);
        if system_authentication
            || (!explicit_login
                && ignored_network(&self.snapshot.settings, active_interface(&self.snapshot)))
        {
            self.close_online_window();
            self.recovery_started = None;
            self.stable_checks = 0;
            self.snapshot.retry_count = 0;
            self.snapshot.next_retry_at = None;
            self.snapshot.session = GatewaySession::default();
            self.snapshot.health = HealthStatus::default();
            self.snapshot.environment = Environment::NonCampus;
            self.snapshot.state = EngineState::Idle;
            if system_authentication {
                self.snapshot.last_error = None;
            }
            self.emit();
            return Ok(());
        }
        self.snapshot.state = EngineState::Checking;
        self.emit();
        let session = self.gateway_status().await?;
        // A private default gateway is normal on campus too. Only report an
        // upstream router when the gateway actually observes a different IP.
        let routed = session
            .ip
            .zip(self.snapshot.active_binding.as_ref())
            .is_some_and(|(observed, binding)| {
                binding.source_ip.is_private() && observed != binding.source_ip
            });
        let previous_environment = self.snapshot.environment.clone();
        let previous_auth = self.snapshot.session.auth.clone();
        self.snapshot.environment = if !self.snapshot.network_trusted {
            Environment::Unknown
        } else if routed {
            Environment::RoutedCampus
        } else {
            Environment::Campus
        };
        if self.snapshot.session.auth == AuthState::Authenticated
            && session.auth == AuthState::Unauthenticated
            && self.recovery_started.is_none()
        {
            self.recovery_started = Some(std::time::Instant::now());
            self.event("session_lost", "认证会话已失效，等待恢复条件", None, None);
        }
        if session.auth != AuthState::Authenticated {
            self.close_online_window();
        }
        self.snapshot.session = session;
        if previous_environment != self.snapshot.environment
            || previous_auth != self.snapshot.session.auth
        {
            self.event(
                "environment_observed",
                "已检查当前网络和认证状态",
                None,
                None,
            );
        }
        self.reconcile_auth(allow_login).await
    }
    async fn enable_automatic(&mut self, expected_network: &str) -> Result<(), AppError> {
        self.check(false).await?;
        if uses_system_authentication(&self.snapshot) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                SYSTEM_AUTHENTICATION_MESSAGE,
            ));
        }
        if self.snapshot.environment == Environment::NonCampus {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "当前网络被网络规则忽略，无法启用自动认证",
            ));
        }
        if self.snapshot.session.auth == AuthState::Unknown {
            return Err(AppError::new(
                ErrorCode::GatewayUnavailable,
                "未能确认当前网关的认证状态，请检查连接后重试",
            ));
        }
        if self
            .selected(self.snapshot.active_binding.as_ref(), None)?
            .is_none_or(|account| account.password.is_empty())
        {
            return Err(AppError::new(
                ErrorCode::Credentials,
                "请先保存并选择校园网账户",
            ));
        }
        // The read-only gateway probe above awaited network I/O. Check again
        // before storing the user's approval for the displayed network.
        let network_generation = self.snapshot.network_generation;
        self.refresh_network().await?;
        let key = network_fingerprint(&self.snapshot).filter(|key| {
            self.snapshot.network_generation == network_generation && key == expected_network
        });
        let Some(key) = key else {
            self.next_check = 0;
            return Err(AppError::new(
                ErrorCode::ConfirmationRequired,
                "当前网络已变化，请检查网卡并重新确认",
            ));
        };
        let mut settings = self.snapshot.settings.clone();
        if !settings.trusted_networks.contains(&key) {
            if settings.trusted_networks.len() >= 128 {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "已确认网络过多，请先删除旧记录",
                ));
            }
            settings.trusted_networks.push(key);
        }
        settings.automatic = true;
        if settings != self.snapshot.settings {
            settings.revision = settings.revision.saturating_add(1);
            persistence::write_settings(&self.paths.config_file, &settings)?;
            self.snapshot.settings = settings;
            self.event("network_trust", "已允许在当前网络自动认证", None, None);
        }
        self.snapshot.network_trusted = true;
        if self
            .snapshot
            .last_error
            .as_ref()
            .is_some_and(|error| error.code == ErrorCode::ConfirmationRequired)
        {
            self.snapshot.last_error = None;
        }
        self.next_check = 0;
        Ok(())
    }
    async fn reconcile_auth(&mut self, allow_login: bool) -> Result<(), AppError> {
        if self.snapshot.session.auth == AuthState::Authenticated {
            self.observe_online();
            if let Some(recovery) = self.recovery_started.take() {
                self.event(
                    "reconnect",
                    "已复核认证会话恢复",
                    None,
                    Some(recovery.elapsed().as_millis() as u64),
                );
            }
            self.stable_checks = self.stable_checks.saturating_add(1);
            self.next_check = now_secs()
                + stable_check_delay(
                    self.stable_checks,
                    self.snapshot.settings.status_interval_secs,
                );
            self.snapshot.state = if self.snapshot.health.internet == Health::Degraded {
                EngineState::NetworkDegraded
            } else {
                EngineState::Online
            };
            self.snapshot.retry_count = 0;
            self.snapshot.next_retry_at = None;
            self.snapshot.last_error = None;
        } else if self.snapshot.settings.paused || self.snapshot.verification_hold {
            self.close_online_window();
            self.stable_checks = 0;
            self.snapshot.state = EngineState::Paused;
        } else if self.snapshot.session.auth == AuthState::Unknown {
            self.snapshot.state = EngineState::Idle;
        } else if self.accounts.is_empty() {
            self.snapshot.state = if self.vault.is_none() {
                EngineState::CredentialsUnavailable
            } else {
                EngineState::NeedsCredentials
            };
        } else if allow_login && self.snapshot.settings.automatic && !self.snapshot.network_trusted
        {
            self.snapshot.state = EngineState::Idle;
            self.snapshot.last_error = Some(AppError::new(
                ErrorCode::ConfirmationRequired,
                "此网络尚未确认；请先信任当前网络再启用自动登录",
            ));
        } else if allow_login && self.snapshot.settings.automatic {
            let selected = self
                .selected(self.snapshot.active_binding.as_ref(), None)?
                .map(|a| a.id.clone());
            if selected.as_ref().is_some_and(|id| {
                self.failed_credential.as_ref() == Some(id)
                    || self.snapshot.settings.blocked_accounts.contains(id)
            }) {
                self.snapshot.state = EngineState::AuthError;
            } else if selected.is_some() {
                self.login_selected(selected.as_deref()).await?;
            } else {
                self.snapshot.state = EngineState::NeedsCredentials;
            }
        } else {
            self.snapshot.state = EngineState::Idle;
        }
        self.emit();
        Ok(())
    }
    async fn login_selected(&mut self, id: Option<&str>) -> Result<(), AppError> {
        let account = self
            .selected(self.snapshot.active_binding.as_ref(), id)?
            .cloned()
            .ok_or_else(|| AppError::new(ErrorCode::Credentials, "请先添加并选择校园网账户"))?;
        self.snapshot.state = EngineState::LoggingIn;
        self.emit();
        let start = std::time::Instant::now();
        let result = if self.simulation {
            self.simulate_login(&account)
        } else {
            self.gateway
                .login(
                    &account.username,
                    account.password.expose(),
                    self.snapshot.active_binding.as_ref(),
                )
                .await
                .map_err(|e| e.into_app_error())
        };
        match result {
            Ok(session) => {
                self.snapshot.session = session;
                self.observe_online();
                self.snapshot.state = EngineState::Online;
                self.snapshot.last_error = None;
                self.snapshot.last_success_at = Some(now_secs());
                self.snapshot.retry_count = 0;
                self.snapshot.next_retry_at = None;
                self.failed_credential = None;
                if self
                    .snapshot
                    .settings
                    .blocked_accounts
                    .contains(&account.id)
                {
                    self.snapshot
                        .settings
                        .blocked_accounts
                        .retain(|id| id != &account.id);
                    self.persist()?;
                }
                self.event(
                    "login_success",
                    "认证成功并已复核",
                    Some(&account.id),
                    Some(start.elapsed().as_millis() as u64),
                );
                if let Some(recovery) = self.recovery_started.take() {
                    self.event(
                        "reconnect",
                        "认证会话已恢复",
                        Some(&account.id),
                        Some(recovery.elapsed().as_millis() as u64),
                    );
                }
                Ok(())
            }
            Err(e) => {
                if e.code == ErrorCode::Credentials {
                    self.failed_credential = Some(account.id.clone());
                    if !self
                        .snapshot
                        .settings
                        .blocked_accounts
                        .contains(&account.id)
                    {
                        self.snapshot
                            .settings
                            .blocked_accounts
                            .push(account.id.clone());
                        self.persist()?;
                    }
                }
                self.event(
                    "login_failure",
                    &e.message,
                    Some(&account.id),
                    Some(start.elapsed().as_millis() as u64),
                );
                Err(e)
            }
        }
    }
    fn simulate_login(&mut self, account: &AccountSecret) -> Result<GatewaySession, AppError> {
        if account.password.expose() == "wrong" {
            return Err(AppError::new(ErrorCode::Credentials, "模拟密码错误"));
        }
        self.simulation_session = GatewaySession {
            auth: AuthState::Authenticated,
            username: Some(account.username.clone()),
            ip: Some(Ipv4Addr::LOCALHOST),
            bytes_used: Some(1024),
            online_seconds: Some(0),
            balance: Some(20.0),
        };
        Ok(self.simulation_session.clone())
    }

    async fn verification_binding(
        &mut self,
        expected_network: &str,
        expected_binding: Option<&NetworkBinding>,
    ) -> Result<NetworkBinding, AppError> {
        self.refresh_network().await?;
        if uses_system_authentication(&self.snapshot) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                SYSTEM_AUTHENTICATION_MESSAGE,
            ));
        }
        if ignored_network(&self.snapshot.settings, active_interface(&self.snapshot)) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "当前网络已设置为忽略，无法进行此项认证",
            ));
        }
        if network_fingerprint(&self.snapshot).as_deref() != Some(expected_network) {
            return Err(verification_network_changed());
        }
        if expected_binding
            .is_some_and(|expected| self.snapshot.active_binding.as_ref() != Some(expected))
        {
            return Err(verification_network_changed());
        }
        self.snapshot
            .active_binding
            .clone()
            .ok_or_else(verification_network_changed)
    }

    async fn verification_status(
        &self,
        gateway: &GatewayClient,
        binding: &NetworkBinding,
        changes: &mut Option<watch::Receiver<u64>>,
    ) -> Result<GatewaySession, AppError> {
        if self.simulation {
            return Ok(self.simulation_session.clone());
        }
        observe_verification(gateway.status(Some(binding)), changes).await
    }

    async fn login_once(
        &mut self,
        username: String,
        password: Secret,
        expected_network: &str,
    ) -> Result<(), AppError> {
        GatewayClient::validate_credentials(&username, password.expose())
            .map_err(|error| error.into_app_error())?;
        let binding = self.verification_binding(expected_network, None).await?;
        let mut config = self.snapshot.settings.gateway.clone();
        config.timeout_secs = self
            .snapshot
            .settings
            .request_timeout_secs
            .min(VERIFICATION_STEP_TIMEOUT_SECS);
        let gateway = GatewayClient::new(config).map_err(|error| error.into_app_error())?;
        let current = self
            .verification_status(&gateway, &binding, &mut None)
            .await?;
        if current.auth != AuthState::Unauthenticated {
            return Err(AppError::new(
                ErrorCode::ConfirmationRequired,
                "请确认校园网当前未登录，再使用仅本次登录",
            ));
        }
        self.verification_binding(expected_network, Some(&binding))
            .await?;
        let candidate = AccountSecret {
            id: String::new(),
            username,
            password,
        };
        let session = if self.simulation {
            self.simulate_login(&candidate)?
        } else {
            gateway
                .verify_credentials(
                    &candidate.username,
                    candidate.password.expose(),
                    Some(&binding),
                )
                .await
                .map_err(|error| error.into_app_error())?
        };
        self.verification_binding(expected_network, Some(&binding))
            .await?;
        self.snapshot.session = session;
        self.snapshot.last_success_at = Some(now_secs());
        self.snapshot.last_error = None;
        self.reconcile_auth(false).await?;
        self.event("login_success", "仅本次登录成功", None, None);
        Ok(())
    }

    async fn verify_account(
        &mut self,
        candidate: AccountSecret,
        expected_network: &str,
    ) -> Result<(), AppError> {
        validate_account_id(&candidate.id)?;
        GatewayClient::validate_credentials(&candidate.username, candidate.password.expose())
            .map_err(|error| error.into_app_error())?;
        if self.accounts.len() >= 32 && !self.accounts.iter().any(|a| a.id == candidate.id) {
            return Err(AppError::new(ErrorCode::InvalidInput, "最多保存 32 个账户"));
        }
        let stored_accounts = match self.vault.as_ref() {
            Some(Vault::Real(store)) => Some(store.load()?),
            Some(Vault::Simulation) => None,
            None => {
                return Err(AppError::new(
                    ErrorCode::CredentialsUnavailable,
                    "凭据存储不可用，请修复后再验证校园账户",
                ))
            }
        };
        let mut changes = self.network_changes.clone().filter(|_| !self.simulation);
        if let Some(changes) = &mut changes {
            changes.borrow_and_update();
        }
        let binding = self.verification_binding(expected_network, None).await?;
        let mut config = self.snapshot.settings.gateway.clone();
        config.timeout_secs = self
            .snapshot
            .settings
            .request_timeout_secs
            .min(VERIFICATION_STEP_TIMEOUT_SECS);
        let gateway = GatewayClient::new(config).map_err(|error| error.into_app_error())?;
        let original = self
            .verification_status(&gateway, &binding, &mut changes)
            .await?;
        if original.auth == AuthState::Unknown {
            return Err(AppError::new(
                ErrorCode::GatewayUnavailable,
                "无法确认网关会话，账户验证未开始",
            ));
        }
        self.verification_binding(expected_network, Some(&binding))
            .await?;
        let mut previous_accounts = stored_accounts
            .as_deref()
            .unwrap_or(&self.accounts)
            .iter()
            .filter(|account| {
                Some(account.username.as_str()) == original.username.as_deref()
                    && !account.password.is_empty()
            });
        let previous = previous_accounts
            .next()
            .filter(|_| previous_accounts.next().is_none())
            .cloned();
        drop(stored_accounts);
        let was_held = self.snapshot.verification_hold;
        self.snapshot.verification_hold = true;
        self.recovery_started = None;
        let started = std::time::Instant::now();
        let result = self
            .test_account_credentials(
                &gateway,
                &candidate,
                &original,
                (expected_network, &binding),
                &mut changes,
            )
            .await;
        let session = match result {
            Ok(session) => session,
            Err(error) => {
                let restored = if error.code == ErrorCode::Cancelled {
                    self.snapshot.session = GatewaySession::default();
                    false
                } else {
                    self.restore_previous_session(
                        &gateway,
                        &original,
                        previous.as_ref(),
                        (expected_network, &binding),
                        &mut changes,
                    )
                    .await
                };
                self.snapshot.verification_hold =
                    was_held || !restored || original.auth != AuthState::Authenticated;
                self.next_check = 0;
                self.snapshot.state = if self.snapshot.session.auth == AuthState::Authenticated {
                    EngineState::Online
                } else if self.snapshot.verification_hold {
                    EngineState::Paused
                } else {
                    EngineState::Idle
                };
                let status = if restored && original.auth == AuthState::Authenticated {
                    "已确认原账户在线"
                } else if restored {
                    "原网络会话未登录，自动认证已暂停"
                } else {
                    "未确认恢复原会话，自动认证已暂停"
                };
                let error = AppError::new(
                    error.code,
                    format!("账户验证失败，凭据未保存。{status}。{}", error.message),
                );
                self.snapshot.last_error = Some(error.clone());
                self.event("account_verification_failed", &error.message, None, None);
                self.emit();
                return Err(error);
            }
        };
        self.snapshot.session = session;
        self.observe_online();
        self.snapshot.state = EngineState::Online;
        self.snapshot.last_success_at = Some(now_secs());
        self.snapshot.retry_count = 0;
        self.snapshot.next_retry_at = None;
        self.next_check = 0;
        if let Err(error) = self.save_verified_account(candidate) {
            self.snapshot.last_error = Some(error.clone());
            self.emit();
            return Err(error);
        }
        self.snapshot.verification_hold = false;
        self.snapshot.last_error = None;
        self.event(
            "login_success",
            "新凭据认证成功并已复核",
            self.snapshot.settings.selected_account.as_deref(),
            Some(started.elapsed().as_millis() as u64),
        );
        Ok(())
    }

    async fn test_account_credentials(
        &mut self,
        gateway: &GatewayClient,
        candidate: &AccountSecret,
        original: &GatewaySession,
        (expected_network, expected_binding): (&str, &NetworkBinding),
        changes: &mut Option<watch::Receiver<u64>>,
    ) -> Result<GatewaySession, AppError> {
        if let Some(username) = original.username.as_deref() {
            let binding = self
                .verification_binding(expected_network, Some(expected_binding))
                .await?;
            if self.simulation {
                self.logout_gateway().await?;
            } else {
                self.close_online_window();
                self.snapshot.session =
                    observe_verification(gateway.logout(username, Some(&binding)), changes).await?;
            }
        }
        let binding = self
            .verification_binding(expected_network, Some(expected_binding))
            .await?;
        self.snapshot.state = EngineState::LoggingIn;
        self.emit();
        let session = if self.simulation {
            self.simulate_login(candidate)?
        } else {
            observe_verification(
                gateway.verify_credentials(
                    &candidate.username,
                    candidate.password.expose(),
                    Some(&binding),
                ),
                changes,
            )
            .await?
        };
        self.verification_binding(expected_network, Some(expected_binding))
            .await?;
        if changes
            .as_ref()
            .is_some_and(|changes| changes.has_changed().unwrap_or(false))
        {
            return Err(verification_network_changed());
        }
        Ok(session)
    }

    async fn restore_previous_session(
        &mut self,
        gateway: &GatewayClient,
        original: &GatewaySession,
        previous: Option<&AccountSecret>,
        (expected_network, expected_binding): (&str, &NetworkBinding),
        changes: &mut Option<watch::Receiver<u64>>,
    ) -> bool {
        // An unsuccessful or interrupted request may have changed the gateway state.
        self.snapshot.session = GatewaySession::default();
        let Ok(binding) = self
            .verification_binding(expected_network, Some(expected_binding))
            .await
        else {
            return false;
        };
        let Ok(session) = self.verification_status(gateway, &binding, changes).await else {
            return false;
        };
        self.snapshot.session = session;
        if self.snapshot.session.auth == original.auth
            && self.snapshot.session.username == original.username
        {
            return true;
        }
        let Some(previous) =
            previous.filter(|_| self.snapshot.session.auth == AuthState::Unauthenticated)
        else {
            return false;
        };
        let Ok(binding) = self
            .verification_binding(expected_network, Some(expected_binding))
            .await
        else {
            return false;
        };
        let restored = if self.simulation {
            self.simulate_login(previous)
        } else {
            observe_verification(
                gateway.login(
                    &previous.username,
                    previous.password.expose(),
                    Some(&binding),
                ),
                changes,
            )
            .await
        };
        if let Ok(session) = restored {
            if self
                .verification_binding(expected_network, Some(expected_binding))
                .await
                .is_ok()
            {
                self.snapshot.session = session;
                self.observe_online();
                return true;
            }
        }
        false
    }

    fn save_verified_account(&mut self, candidate: AccountSecret) -> Result<(), AppError> {
        let id = candidate.id.clone();
        let mut settings = self.snapshot.settings.clone();
        settings.selected_account = Some(id.clone());
        settings.blocked_accounts.retain(|blocked| blocked != &id);
        settings.revision = settings.revision.saturating_add(1);
        let mut accounts = self.accounts.clone();
        accounts.retain(|account| account.id != id);
        accounts.push(candidate);
        self.save_accounts(&accounts).map_err(|_| {
            AppError::new(
                ErrorCode::Unavailable,
                "新账户已登录，但凭据保存失败。请检查凭据存储后重试",
            )
        })?;
        if persistence::write_settings(&self.paths.config_file, &settings).is_err() {
            let previous = self.accounts.clone();
            if self.save_accounts(&previous).is_err() {
                self.accounts = accounts;
                self.account_metadata();
                return Err(AppError::new(
                    ErrorCode::Unavailable,
                    "新账户已登录。设置保存失败，凭据回退也失败，请检查账户存储",
                ));
            }
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "新账户已登录，但设置保存失败，之前的凭据已恢复",
            ));
        }
        self.snapshot.settings = settings;
        self.accounts = accounts;
        if self.failed_credential.as_deref() == Some(&id) {
            self.failed_credential = None;
        }
        self.account_metadata();
        self.event("account_saved", "验证成功的账户已安全保存", Some(&id), None);
        Ok(())
    }
    pub async fn execute(
        &mut self,
        command: Command,
        caller: &Caller,
    ) -> Result<ReplyData, AppError> {
        if self.snapshot.scope == Scope::User && !caller.owner && !caller.administrator {
            return Err(AppError::new(
                ErrorCode::PermissionDenied,
                "只有当前用户可以修改个人配置",
            ));
        }
        match command {
            Command::Status => {}
            Command::Check => {
                self.check(true).await?;
            }
            Command::Pause => {
                self.snapshot.settings.paused = true;
                self.persist()?;
                self.snapshot.state = EngineState::Paused;
                self.event("paused", "用户暂停自动认证", None, None);
            }
            Command::Resume => {
                self.snapshot.settings.paused = false;
                self.snapshot.settings.automatic = true;
                self.failed_credential = None;
                self.persist()?;
                self.snapshot.verification_hold = false;
                self.check(true).await?;
            }
            Command::EnableAutomatic { expected_network } => {
                self.enable_automatic(&expected_network).await?;
            }
            Command::TrustNetwork { trusted } => {
                self.check(false).await?;
                if uses_system_authentication(&self.snapshot) {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        SYSTEM_AUTHENTICATION_MESSAGE,
                    ));
                }
                let key = network_fingerprint(&self.snapshot).ok_or_else(|| {
                    AppError::new(ErrorCode::Network, "请先连接网络并确认认证网关可用")
                })?;
                self.snapshot
                    .settings
                    .trusted_networks
                    .retain(|known| known != &key);
                if trusted {
                    if self.snapshot.settings.trusted_networks.len() >= 128 {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "已确认网络过多，请先删除旧记录",
                        ));
                    }
                    self.snapshot.settings.trusted_networks.push(key);
                }
                self.persist()?;
                self.snapshot.network_trusted = trusted;
                self.next_check = 0;
                self.event(
                    "network_trust",
                    if trusted {
                        "用户已允许在当前网络自动认证"
                    } else {
                        "已取消当前网络的自动认证许可"
                    },
                    None,
                    None,
                );
            }
            Command::ProfileSave { id, name } => {
                if id.is_empty()
                    || id.len() > 64
                    || !id
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                    || name.trim().is_empty()
                    || name.len() > 128
                {
                    return Err(AppError::new(ErrorCode::InvalidInput, "环境标识或名称无效"));
                }
                let mut settings = self.snapshot.settings.clone();
                settings.profiles.retain(|p| p.id != id);
                settings.profiles.push(NetworkProfile {
                    id: id.clone(),
                    name,
                    gateway: settings.gateway.clone(),
                    interface_id: settings.interface_id.clone(),
                    account_id: settings.selected_account.clone(),
                    rules: settings.rules.clone(),
                });
                settings.active_profile = Some(id);
                settings.validate()?;
                self.snapshot.settings = settings;
                self.persist()?;
                self.event("profile_saved", "当前认证环境已保存", None, None);
            }
            Command::ProfileSelect { id } => {
                let profile = self
                    .snapshot
                    .settings
                    .profiles
                    .iter()
                    .find(|p| p.id == id)
                    .cloned()
                    .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "认证环境不存在"))?;
                if self.snapshot.scope == Scope::System
                    && profile.gateway != self.snapshot.settings.gateway
                    && !caller.administrator
                {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "切换后台认证目标需要管理员授权",
                    ));
                }
                let gateway =
                    GatewayClient::new(profile.gateway.clone()).map_err(|e| e.into_app_error())?;
                self.snapshot.settings.gateway = profile.gateway;
                self.snapshot.settings.interface_id = profile.interface_id;
                self.snapshot.settings.selected_account = profile.account_id;
                self.snapshot.settings.rules = profile.rules;
                self.snapshot.settings.active_profile = Some(id);
                self.persist()?;
                self.gateway = gateway;
                self.snapshot.session = GatewaySession::default();
                self.snapshot.environment = Environment::Unknown;
                self.snapshot.network_trusted = false;
                self.stable_checks = 0;
                self.next_check = 0;
                self.event(
                    "profile_selected",
                    "已切换认证环境，等待重新检查",
                    None,
                    None,
                );
            }
            Command::ProfileDelete { id } => {
                self.snapshot.settings.profiles.retain(|p| p.id != id);
                if self.snapshot.settings.active_profile.as_ref() == Some(&id) {
                    self.snapshot.settings.active_profile = None;
                }
                self.persist()?;
                self.event("profile_deleted", "已删除保存的认证环境", None, None);
            }
            Command::Login {
                account_id,
                force,
                confirmed,
            } => {
                self.check_network(false, true).await?;
                if uses_system_authentication(&self.snapshot) {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        SYSTEM_AUTHENTICATION_MESSAGE,
                    ));
                }
                if self.snapshot.active_binding.is_none() {
                    return Err(AppError::new(
                        ErrorCode::Network,
                        "没有可安全使用的认证出口，请检查网络或选择网卡",
                    ));
                }
                if !self.snapshot.network_trusted && !confirmed {
                    return Err(AppError::new(
                        ErrorCode::ConfirmationRequired,
                        "此网络尚未确认；请确认本次登录，或先信任当前网络",
                    ));
                }
                if self.snapshot.session.auth == AuthState::Authenticated {
                    let requested = self
                        .selected(self.snapshot.active_binding.as_ref(), account_id.as_deref())?;
                    if !force
                        && requested.is_some_and(|a| {
                            self.snapshot.session.username.as_deref() != Some(&a.username)
                        })
                    {
                        return Err(AppError::new(
                            ErrorCode::ConfirmationRequired,
                            "已有其他账户在线；切换账户需要确认强制重新认证",
                        ));
                    }
                    if !force {
                        return Ok(ReplyData::Snapshot(Box::new(self.snapshot.clone())));
                    }
                    if !confirmed {
                        return Err(AppError::new(
                            ErrorCode::ConfirmationRequired,
                            "强制重新认证可能影响共享连接，请确认",
                        ));
                    }
                    self.logout_gateway().await?;
                }
                self.snapshot.settings.paused = false;
                self.persist()?;
                self.failed_credential = None;
                self.login_selected(account_id.as_deref()).await?;
                self.snapshot.verification_hold = false;
            }
            Command::LoginOnce {
                username,
                password,
                expected_network,
            } => {
                self.login_once(username, password, &expected_network)
                    .await?;
            }
            Command::Logout { confirmed } => {
                if !confirmed {
                    return Err(AppError::new(
                        ErrorCode::ConfirmationRequired,
                        "注销可能影响共享此认证的设备，请确认",
                    ));
                }
                let network = self.refresh_network().await;
                if network.is_ok() && uses_system_authentication(&self.snapshot) {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        SYSTEM_AUTHENTICATION_MESSAGE,
                    ));
                }
                // Persist intent BEFORE sending a potentially uncertain logout request.
                self.snapshot.settings.paused = true;
                self.persist()?;
                self.snapshot.state = EngineState::Paused;
                self.emit();
                self.recovery_started = None;
                network?;
                self.logout_gateway().await?;
                self.snapshot.state = EngineState::Paused;
                self.event("logout", "手动注销；自动认证已持续暂停", None, None);
            }
            Command::AccountPut {
                id,
                username,
                password,
            } => {
                validate_account_id(&id)?;
                if username.trim().is_empty()
                    || username.len() > 128
                    || password.is_empty()
                    || password.expose().len() > 1024
                {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "账号标识、用户名或密码无效",
                    ));
                }
                let mut accounts = self.accounts.clone();
                accounts.retain(|a| a.id != id);
                if accounts.len() >= 32 {
                    return Err(AppError::new(ErrorCode::InvalidInput, "最多保存 32 个账户"));
                }
                accounts.push(AccountSecret {
                    id: id.clone(),
                    username,
                    password,
                });
                self.save_accounts(&accounts)?;
                self.accounts = accounts;
                if self.snapshot.settings.selected_account.is_none() {
                    self.snapshot.settings.selected_account = Some(id.clone());
                }
                self.snapshot
                    .settings
                    .blocked_accounts
                    .retain(|blocked| blocked != &id);
                self.persist()?;
                if self.failed_credential.as_ref() == Some(&id) {
                    self.failed_credential = None;
                }
                self.snapshot.last_error = None;
                self.next_check = 0;
                self.account_metadata();
                self.event("account_saved", "账户凭据已安全更新", Some(&id), None);
            }
            Command::VerifyAccount {
                id,
                username,
                password,
                expected_network,
            } => {
                self.verify_account(
                    AccountSecret {
                        id,
                        username,
                        password,
                    },
                    &expected_network,
                )
                .await?;
            }
            Command::AccountDelete { id } => {
                let accounts = self
                    .accounts
                    .iter()
                    .filter(|a| a.id != id)
                    .cloned()
                    .collect::<Vec<_>>();
                self.save_accounts(&accounts)?;
                self.accounts = accounts;
                if self.snapshot.settings.selected_account.as_ref() == Some(&id) {
                    self.snapshot.settings.selected_account = None;
                    self.persist()?;
                }
                self.account_metadata();
                self.event(
                    "account_deleted",
                    "已删除保存的账户；现有会话保持不变",
                    Some(&id),
                    None,
                );
            }
            Command::AccountSelect { id } => {
                if !self.accounts.iter().any(|a| a.id == id) {
                    return Err(AppError::new(ErrorCode::InvalidInput, "账户不存在"));
                }
                self.snapshot.settings.selected_account = Some(id);
                self.persist()?;
                self.failed_credential = None;
                self.next_check = 0;
            }
            Command::SettingsPut {
                mut settings,
                expected_revision,
            } => {
                settings.validate()?;
                if expected_revision != self.snapshot.settings.revision {
                    return Err(AppError::new(
                        ErrorCode::Busy,
                        "设置已被其他用户修改，请刷新后重试",
                    ));
                }
                if self.snapshot.scope == Scope::System
                    && settings.gateway != self.snapshot.settings.gateway
                    && !caller.administrator
                {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "修改后台认证目标需要管理员授权",
                    ));
                }
                // Runtime safety intent is not an importable preference.
                settings.blocked_accounts = self.snapshot.settings.blocked_accounts.clone();
                settings.paused = self.snapshot.settings.paused;
                settings
                    .trusted_networks
                    .retain(|key| self.snapshot.settings.trusted_networks.contains(key));
                let gateway =
                    GatewayClient::new(settings.gateway.clone()).map_err(|e| e.into_app_error())?;
                if self.snapshot.scope == Scope::User {
                    if !self.simulation
                        && settings.user_autostart != self.snapshot.settings.user_autostart
                    {
                        crate::maintenance::execute(
                            Scope::User,
                            crate::maintenance::MaintenanceAction::SetAutostart {
                                enabled: settings.user_autostart,
                            },
                            false,
                        )
                        .await?;
                    }
                    if !self.simulation
                        && settings.gui_autostart != self.snapshot.settings.gui_autostart
                    {
                        crate::maintenance::execute(
                            Scope::User,
                            crate::maintenance::MaintenanceAction::SetGuiAutostart {
                                enabled: settings.gui_autostart,
                            },
                            false,
                        )
                        .await?;
                    }
                } else if settings.user_autostart != self.snapshot.settings.user_autostart
                    || settings.gui_autostart != self.snapshot.settings.gui_autostart
                {
                    return Err(AppError::new(
                        ErrorCode::InvalidInput,
                        "登录启动是个人设置，请通过用户端维护配置；共享后台启动由服务管理控制",
                    ));
                }
                settings.revision = self.snapshot.settings.revision.saturating_add(1);
                persistence::write_settings(&self.paths.config_file, &settings)?;
                self.snapshot.settings = settings;
                self.gateway = gateway;
                self.next_check = 0;
            }
            Command::History { limit } => {
                return Ok(ReplyData::History(
                    self.history
                        .as_ref()
                        .ok_or_else(|| AppError::new(ErrorCode::Unavailable, "历史数据库不可用"))?
                        .recent(limit)?,
                ))
            }
            Command::Statistics => {
                let mut statistics = self
                    .history
                    .as_ref()
                    .ok_or_else(|| AppError::new(ErrorCode::Unavailable, "历史数据库不可用"))?
                    .statistics()?;
                if let Some((start, last)) = self.online_window {
                    let seconds = last.duration_since(start).as_secs();
                    statistics.observed_online_seconds =
                        statistics.observed_online_seconds.saturating_add(seconds);
                    statistics.longest_observed_online_seconds =
                        statistics.longest_observed_online_seconds.max(seconds);
                }
                return Ok(ReplyData::Statistics(statistics));
            }
            Command::Diagnose => return Ok(ReplyData::Diagnostics(self.diagnostics())),
            Command::ExportSettings => {
                return Ok(ReplyData::Settings(Box::new(
                    self.snapshot.settings.clone(),
                )))
            }
            Command::Shutdown => {
                if self.snapshot.scope == Scope::System && !caller.administrator {
                    return Err(AppError::new(
                        ErrorCode::PermissionDenied,
                        "停止系统后台需要管理员授权；暂停认证无需管理员",
                    ));
                }
                self.stopping = true;
                self.snapshot.state = EngineState::Stopping;
            }
        }
        self.emit();
        Ok(ReplyData::Snapshot(Box::new(self.snapshot.clone())))
    }
    async fn logout_gateway(&mut self) -> Result<(), AppError> {
        self.close_online_window();
        if self.simulation {
            self.simulation_session = GatewaySession {
                auth: AuthState::Unauthenticated,
                ip: Some(Ipv4Addr::LOCALHOST),
                ..Default::default()
            };
            self.snapshot.session = self.simulation_session.clone();
            return Ok(());
        }
        let username = self
            .snapshot
            .session
            .username
            .clone()
            .or_else(|| {
                self.selected(self.snapshot.active_binding.as_ref(), None)
                    .ok()
                    .flatten()
                    .map(|a| a.username.clone())
            })
            .unwrap_or_default();
        self.snapshot.session = self
            .gateway
            .logout(&username, self.snapshot.active_binding.as_ref())
            .await
            .map_err(|e| e.into_app_error())?;
        Ok(())
    }
    fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"version":APP_VERSION,"api_version":API_VERSION,"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH,"scope":self.snapshot.scope,"state":self.snapshot.state,"environment":self.snapshot.environment,"network_trusted":self.snapshot.network_trusted,"health":self.snapshot.health,"interface_count":self.snapshot.interfaces.len(),"selected_interface":self.snapshot.active_binding.as_ref().map(|_|"[redacted]"),"network_generation":self.snapshot.network_generation,"retry_count":self.snapshot.retry_count,"next_retry_at":self.snapshot.next_retry_at,"last_error_code":self.snapshot.last_error.as_ref().map(|e|e.code),"credential_store_available":self.vault.is_some(),"history_available":self.history.is_some(),"service":{"installed":self.snapshot.service.installed,"running":self.snapshot.service.running,"enabled":self.snapshot.service.enabled},"update_feed_configured":crate::maintenance::updates::UpdateConfig::configured(),"simulation":self.simulation,"limitations":["报告不包含密码、协议认证材料、用户名或完整IP；网络诊断不会修改系统设置。"]})
    }
    async fn health_check(&mut self) {
        self.next_health = now_secs() + self.snapshot.settings.health_interval_secs;
        if self.snapshot.environment == Environment::NonCampus {
            return;
        }
        if self.simulation {
            self.snapshot.health = HealthStatus {
                internet: Health::Healthy,
                dns: Health::Healthy,
                ipv6: Health::Unknown,
                checked_at: Some(now_secs()),
            };
            return;
        }
        if self.snapshot.session.auth != AuthState::Authenticated {
            return;
        }
        let client = match reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(
                self.snapshot.settings.request_timeout_secs,
            ))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut succeeded = 0;
        let mut checked = 0;
        for target in self.snapshot.settings.health_targets.iter().take(2) {
            let Ok(url) = url::Url::parse(target) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                continue;
            }
            checked += 1;
            if let Ok(response) = client.get(url.clone()).send().await {
                let code = response.status();
                if url.host_str() == Some("connectivitycheck.gstatic.com") {
                    if code.as_u16() == 204 {
                        succeeded += 1;
                    }
                } else if code.is_success() {
                    use futures_util::StreamExt;
                    let mut stream = response.bytes_stream();
                    let mut body = Vec::new();
                    while let Some(Ok(chunk)) = stream.next().await {
                        if body.len() + chunk.len() > 1024 {
                            break;
                        }
                        body.extend_from_slice(&chunk);
                    }
                    if url.host_str() != Some("www.msftconnecttest.com")
                        || body == b"Microsoft Connect Test"
                    {
                        succeeded += 1;
                    }
                }
            }
        }
        self.snapshot.health.internet = if succeeded > 0 {
            Health::Healthy
        } else if checked > 0 {
            Health::Degraded
        } else {
            Health::Unknown
        };
        self.snapshot.health.dns = match tokio::time::timeout(
            Duration::from_secs(3),
            tokio::net::lookup_host(("www.bit.edu.cn", 80)),
        )
        .await
        {
            Ok(Ok(mut addresses)) => {
                if addresses.next().is_some() {
                    Health::Healthy
                } else {
                    Health::Degraded
                }
            }
            _ => Health::Degraded,
        };
        self.snapshot.health.checked_at = Some(now_secs());
        self.snapshot.health.ipv6 = ipv6_health(
            &self.snapshot.settings.health_targets,
            self.snapshot.settings.request_timeout_secs,
        )
        .await;
        if self.snapshot.health.internet == Health::Degraded {
            self.next_health = now_secs() + 30.min(self.snapshot.settings.health_interval_secs);
            self.snapshot.state = EngineState::NetworkDegraded;
            self.event(
                "health_degraded",
                "校园认证仍有效，互联网连接异常；保持现有认证",
                None,
                None,
            );
        }
    }
}

fn log_rank(level: &str) -> u8 {
    match level {
        "error" => 0,
        "warning" => 1,
        "info" => 2,
        _ => 3,
    }
}

fn prune_logs(directory: &std::path::Path, days: u32) {
    use std::io::Read;
    let maximum_age = Duration::from_secs(u64::from(days) * 86400);
    let before = now_secs().saturating_sub(maximum_age.as_secs());
    for name in [
        "engine.jsonl",
        "engine.1.jsonl",
        "engine.2.jsonl",
        "engine.3.jsonl",
        "engine.4.jsonl",
    ] {
        let path = directory.join(name);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
            continue;
        }
        if std::fs::metadata(&path)
            .ok()
            .filter(|m| m.is_file())
            .and_then(|m| m.modified().ok())
            .and_then(|at| at.elapsed().ok())
            .is_some_and(|age| age > maximum_age)
        {
            let _ = std::fs::remove_file(path);
            continue;
        }
        // File mtime alone cannot expire old events in a quiet, continually
        // appended log. Filter each bounded file by the event's own timestamp.
        let Ok(file) = File::open(&path) else {
            continue;
        };
        let mut original = Vec::new();
        if file
            .take(6 * 1024 * 1024 + 1)
            .read_to_end(&mut original)
            .is_err()
            || original.len() > 6 * 1024 * 1024
        {
            continue;
        }
        let mut retained = Vec::with_capacity(original.len());
        let mut changed = false;
        for line in original.split_inclusive(|byte| *byte == b'\n') {
            let expired = line.len() <= 16384
                && serde_json::from_slice::<serde_json::Value>(line)
                    .ok()
                    .and_then(|event| event.get("at")?.as_u64())
                    .is_some_and(|at| at < before);
            if expired {
                changed = true;
            } else {
                retained.extend_from_slice(line);
            }
        }
        if changed {
            let _ = persistence::atomic_write(&path, &retained);
        }
    }
}

fn stable_check_delay(stable_checks: u32, maximum: u64) -> u64 {
    10u64
        .saturating_mul(1u64 << stable_checks.saturating_sub(1).min(16))
        .min(maximum)
}

/// Diagnostic-only IPv6 transport probe. No authentication material is sent.
/// Uses the configured health origins and the OS route, independently of SRUN.
async fn ipv6_health(targets: &[String], timeout_secs: u64) -> Health {
    let check = async {
        let mut attempted = false;
        for target in targets.iter().take(2) {
            let Ok(url) = url::Url::parse(target) else {
                continue;
            };
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                continue;
            }
            let Some(host) = url.host_str() else { continue };
            let host = match host.strip_prefix('[') {
                Some(literal) => literal.strip_suffix(']').unwrap_or(literal),
                None => host,
            };
            let Some(port) = url.port_or_known_default() else {
                continue;
            };
            if let Ok(addresses) = tokio::net::lookup_host((host, port)).await {
                for address in addresses.filter(|a| a.is_ipv6()).take(2) {
                    attempted = true;
                    if let Ok(Ok(stream)) = tokio::time::timeout(
                        Duration::from_secs(2),
                        tokio::net::TcpStream::connect(address),
                    )
                    .await
                    {
                        if stream.local_addr().is_ok_and(|address| address.is_ipv6()) {
                            return Health::Healthy;
                        }
                    }
                }
            };
        }
        if attempted {
            Health::Unavailable
        } else {
            Health::Unknown
        }
    };
    tokio::time::timeout(Duration::from_secs(timeout_secs.clamp(1, 5)), check)
        .await
        .unwrap_or(Health::Degraded)
}

fn active_interface(snapshot: &Snapshot) -> Option<&NetworkInterface> {
    let binding = snapshot.active_binding.as_ref()?;
    snapshot
        .interfaces
        .iter()
        .find(|interface| interface.id == binding.interface_id)
}

fn validate_account_id(id: &str) -> Result<(), AppError> {
    if id.is_empty()
        || id.len() > 64
        || !id
            .bytes()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == b'-' || ch == b'_')
    {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "账号标识、用户名或密码无效",
        ));
    }
    Ok(())
}

fn verification_network_changed() -> AppError {
    AppError::new(ErrorCode::Cancelled, "当前网络已变化，请检查网卡并重新确认")
}

async fn observe_verification<T>(
    operation: impl std::future::Future<Output = Result<T, bitnet_gateway::GatewayError>>,
    changes: &mut Option<watch::Receiver<u64>>,
) -> Result<T, AppError> {
    if let Some(changes) = changes {
        tokio::select! {
            biased;
            _ = changes.changed() => Err(verification_network_changed()),
            result = operation => result.map_err(|error| error.into_app_error()),
        }
    } else {
        operation.await.map_err(|error| error.into_app_error())
    }
}

/// Reports whether the active wireless interface is on BIT-mobile.
pub fn uses_system_authentication(snapshot: &Snapshot) -> bool {
    active_interface(snapshot).is_some_and(|interface| {
        interface.wireless && interface.ssid.as_deref() == Some("BIT-mobile")
    })
}

/// Identifies the connection context displayed for network-trust confirmation.
pub fn network_fingerprint(snapshot: &Snapshot) -> Option<String> {
    use sha2::{Digest, Sha256};
    let binding = snapshot.active_binding.as_ref()?;
    let interface = active_interface(snapshot)?;
    let octets = binding.source_ip.octets();
    let context = serde_json::to_vec(&(
        &snapshot.settings.gateway.base_url,
        &interface.id,
        &interface.ssid,
        interface.gateway,
        &octets[..3],
    ))
    .ok()?;
    let mut hash = Sha256::new();
    hash.update(context);
    let gateway = &snapshot.settings.gateway;
    if gateway.request_ip.is_some() || gateway.dumb_terminal {
        hash.update(serde_json::to_vec(&(gateway.request_ip, gateway.dumb_terminal)).ok()?);
    }
    Some(format!("{:x}", hash.finalize()))
}

pub fn resolve_account(
    settings: &Settings,
    interface: Option<&NetworkInterface>,
) -> Result<Option<String>, AppError> {
    let mut matching = settings
        .rules
        .iter()
        .filter(|rule| {
            rule.interface_id
                .as_ref()
                .is_none_or(|id| interface.is_some_and(|i| &i.id == id))
                && rule
                    .ssid
                    .as_ref()
                    .is_none_or(|ssid| interface.is_some_and(|i| i.ssid.as_ref() == Some(ssid)))
                && rule
                    .gateway
                    .is_none_or(|gw| interface.is_some_and(|i| i.gateway == Some(gw)))
        })
        .collect::<Vec<_>>();
    matching.sort_by_key(|r| std::cmp::Reverse(r.priority));
    if let Some(first) = matching.first() {
        let top = matching
            .iter()
            .take_while(|r| r.priority == first.priority)
            .collect::<Vec<_>>();
        if top.iter().any(|r| r.ignore) {
            return Ok(None);
        }
        if top.iter().any(|r| r.account_id != first.account_id) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "存在相同优先级的冲突网络规则，请修改或手动选择账户",
            ));
        }
        if first.account_id.is_some() {
            return Ok(first.account_id.clone());
        }
    }
    Ok(settings.selected_account.clone())
}

fn ignored_network(settings: &Settings, interface: Option<&NetworkInterface>) -> bool {
    let matching = settings
        .rules
        .iter()
        .filter(|r| {
            r.interface_id
                .as_ref()
                .is_none_or(|id| interface.is_some_and(|i| &i.id == id))
                && r.ssid
                    .as_ref()
                    .is_none_or(|s| interface.is_some_and(|i| i.ssid.as_ref() == Some(s)))
                && r.gateway
                    .is_none_or(|g| interface.is_some_and(|i| i.gateway == Some(g)))
        })
        .collect::<Vec<_>>();
    let Some(priority) = matching.iter().map(|r| r.priority).max() else {
        return false;
    };
    matching.iter().any(|r| r.priority == priority && r.ignore)
}

pub async fn run(scope: Scope, simulation: bool) -> Result<(), AppError> {
    let paths = bitnet_platform::paths(scope)?;
    let _instance = InstanceLock::acquire(&paths.data_dir)?;
    let (tx, mut requests) = mpsc::channel::<Envelope>(24);
    let (publish, state) = watch::channel(Snapshot::new(scope));
    let mut engine = Engine::open(scope, simulation, publish)?;
    let ipc = tokio::spawn(crate::ipc::serve(scope, tx, state));
    let mut changes = bitnet_platform::network::subscribe_changes()?;
    if changes.polling_only {
        engine
            .snapshot
            .capabilities
            .push("network_polling_only".into());
        engine.event(
            "network_monitor_fallback",
            "原生网络通知不可用，已使用有界周期检查",
            None,
            None,
        );
    } else {
        engine.network_changes = Some(changes.receiver.clone());
        engine
            .snapshot
            .capabilities
            .push("native_network_events".into());
    }
    engine.emit();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let _ = bitnet_platform::service::notify_ready();
    let mut network_events_open = true;
    let mut last_tick = now_secs();
    let mut next_prune = now_secs() + 3600;
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        tokio::select! {
            envelope=requests.recv()=>{
                let Some(envelope)=envelope else{break};
                let request_id=envelope.request.request_id;
                let verification=matches!(&envelope.request.command,Command::VerifyAccount{..});
                let fresh_credentials=verification || matches!(&envelope.request.command,Command::LoginOnce{..});
                if fresh_credentials {
                    if envelope.answer.is_closed() {
                        continue;
                    }
                    if envelope.queued_at.elapsed() > Duration::from_secs(3) {
                        let result = Err(AppError::new(ErrorCode::Busy, "认证后台正忙，本次操作尚未开始，请重试"));
                        let _ = envelope.answer.send(Reply { api_version: API_VERSION, request_id, result });
                        continue;
                    }
                }
                let network_command=matches!(&envelope.request.command,Command::Check|Command::Login{..}|Command::LoginOnce{..}|Command::Logout{..}|Command::EnableAutomatic{..});
                let mut event=changes.receiver.clone();event.borrow_and_update();
                let result=if network_command&&network_events_open&&!simulation{
                    tokio::select!{result=engine.execute(envelope.request.command,&envelope.caller)=>result,_=event.changed()=>{engine.next_check=now_secs()+2;Err(AppError::new(ErrorCode::Cancelled,"网络已变化，旧操作已取消；请先查看当前状态"))}}
                }else{engine.execute(envelope.request.command,&envelope.caller).await};
                if let Err(ref e)=result{if !verification && matches!(e.code,ErrorCode::Network|ErrorCode::GatewayUnavailable|ErrorCode::Credentials|ErrorCode::CredentialsUnavailable){engine.set_error(e.clone());engine.emit();}}
                let _=envelope.answer.send(Reply{api_version:API_VERSION,request_id,result});
                if engine.stopping{let _=tokio::time::timeout(Duration::from_secs(2),envelope.response_written).await;}
            },
            changed=changes.receiver.changed(),if network_events_open=>{if changed.is_ok(){engine.next_check=now_secs()+2;engine.next_health=now_secs()+3;}else{network_events_open=false;}},
            _=tick.tick()=>{
                if bitnet_platform::service::is_stopping(){break;}
                let current=now_secs();
                if current.saturating_sub(last_tick)>5{engine.next_check=0;engine.next_health=0;engine.stable_checks=0;engine.event("resume_check","系统恢复或时钟发生变化，重新检查网络",None,None);}
                last_tick=current;
                if engine.next_check<=now_secs(){
                    let mut event=changes.receiver.clone();event.borrow_and_update();
                    let result=if network_events_open&&!simulation{tokio::select!{result=engine.check(true)=>result,_=event.changed()=>{engine.next_check=now_secs()+2;Ok(())}}}else{engine.check(true).await};
                    if let Err(e)=result{engine.set_error(e);engine.emit();}
                }
                if engine.next_health<=now_secs(){engine.health_check().await;engine.emit();}
                let _=bitnet_platform::service::notify_watchdog();
                if current>=next_prune{if let Some(h)=&engine.history{let _=h.prune(engine.snapshot.settings.history_retention_days);}prune_logs(&engine.paths.log_dir,engine.snapshot.settings.log_retention_days);next_prune=current+3600;}
            },
            _=&mut shutdown=>break,
        }
        if engine.stopping {
            break;
        }
        if ipc.is_finished() {
            return ipc
                .await
                .map_err(|_| AppError::new(ErrorCode::Internal, "本地通信任务异常退出"))?;
        }
    }
    ipc.abort();
    engine.close_online_window();
    engine.event("engine_stop", "引擎停止，保持校园认证会话", None, None);
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        if let Ok(mut terminate) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_ = terminate.recv()=>{}, _ = tokio::signal::ctrl_c()=>{}};
            return;
        }
    }
    if tokio::signal::ctrl_c().await.is_err() {
        std::future::pending::<()>().await;
    }
}

struct InstanceLock {
    _file: File,
}
impl InstanceLock {
    fn acquire(data_dir: &std::path::Path) -> Result<Self, AppError> {
        std::fs::create_dir_all(data_dir)
            .map_err(|_| AppError::new(ErrorCode::PermissionDenied, "无法创建状态目录"))?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(data_dir.join("engine.lock"))
            .map_err(|_| AppError::new(ErrorCode::PermissionDenied, "无法打开引擎锁"))?;
        file.try_lock_exclusive()
            .map_err(|_| AppError::new(ErrorCode::Busy, "已有认证引擎运行"))?;
        Ok(Self { _file: file })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn isolated(name: &str) -> (Engine, AppPaths) {
        let data_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/runtime-tests")
            .join(format!("{name}-{}-{}", std::process::id(), now_ms()));
        std::fs::create_dir_all(&data_dir).unwrap();
        let data_dir = std::fs::canonicalize(data_dir).unwrap();
        let paths = AppPaths {
            config_file: data_dir.join("settings.json"),
            credentials_file: data_dir.join("accounts.enc"),
            history_file: data_dir.join("history.sqlite3"),
            log_dir: data_dir.join("logs"),
            data_dir,
        };
        let (tx, _) = watch::channel(Snapshot::new(Scope::User));
        (
            Engine::open_in(Scope::User, true, tx, paths.clone()).unwrap(),
            paths,
        )
    }
    fn owner() -> Caller {
        Caller {
            owner: true,
            administrator: false,
        }
    }

    #[tokio::test]
    async fn one_time_login_keeps_settings_and_cannot_replace_an_online_session() {
        let (mut engine, _) = isolated("one-time-login");
        engine.check(false).await.unwrap();
        let before = engine.snapshot.settings.clone();
        let count = engine.accounts.len();
        let expected_network = network_fingerprint(&engine.snapshot).unwrap();
        let command = |username: &str| Command::LoginOnce {
            username: username.into(),
            password: Secret::new("synthetic"),
            expected_network: expected_network.clone(),
        };
        engine.execute(command("once"), &owner()).await.unwrap();
        assert_eq!(engine.snapshot.session.username.as_deref(), Some("once"));
        let error = engine
            .execute(command("second"), &owner())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfirmationRequired);
        assert_eq!(engine.snapshot.session.username.as_deref(), Some("once"));
        assert_eq!(engine.snapshot.settings, before);
        assert_eq!(engine.accounts.len(), count);
    }

    #[tokio::test]
    async fn account_verification_saves_only_after_a_fresh_login() {
        let (mut engine, paths) = isolated("verify-account");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine.execute(Command::Resume, &owner()).await.unwrap();
        let before = engine.snapshot.settings.clone();
        let fingerprint = network_fingerprint(&engine.snapshot).unwrap();
        let verify = |password: &str, expected_network: String| Command::VerifyAccount {
            id: "new".into(),
            username: "new-user".into(),
            password: Secret::new(password),
            expected_network,
        };

        let stale = engine
            .execute(verify("good", "stale".into()), &owner())
            .await
            .unwrap_err();
        assert_eq!(stale.code, ErrorCode::Cancelled);
        assert_eq!(engine.simulation_session.username.as_deref(), Some("demo"));
        let bad = engine
            .execute(verify("wrong", fingerprint.clone()), &owner())
            .await
            .unwrap_err();
        assert_eq!(bad.code, ErrorCode::Credentials);
        assert!(bad.message.contains("已确认原账户在线"));
        assert_eq!(engine.simulation_session.username.as_deref(), Some("demo"));
        assert_eq!(engine.accounts.len(), 1);
        assert_eq!(engine.snapshot.settings, before);
        assert_eq!(
            persistence::read_settings(&paths.config_file).unwrap(),
            before
        );

        let good = engine
            .execute(verify("good", fingerprint), &owner())
            .await
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(good.settings.selected_account.as_deref(), Some("new"));
        assert_eq!(good.session.username.as_deref(), Some("new-user"));
        assert_eq!(good.accounts.len(), 2);
        assert_eq!(good.settings.revision, before.revision + 1);
        assert_eq!(
            persistence::read_settings(&paths.config_file).unwrap(),
            good.settings
        );
    }

    #[tokio::test]
    async fn failed_account_verification_does_not_guess_a_restore_account() {
        let (mut engine, _) = isolated("verify-unknown-session");
        engine.snapshot.settings.automatic = true;
        engine.snapshot.settings.selected_account = Some("demo".into());
        engine.simulation_session.auth = AuthState::Authenticated;
        engine.simulation_session.username = Some("external".into());
        engine.check(false).await.unwrap();
        let expected_network = network_fingerprint(&engine.snapshot).unwrap();
        let previous_settings = engine.snapshot.settings.clone();
        let result = engine
            .execute(
                Command::VerifyAccount {
                    id: "candidate".into(),
                    username: "candidate".into(),
                    password: Secret::new("wrong"),
                    expected_network,
                },
                &owner(),
            )
            .await
            .unwrap_err();
        assert!(result.message.contains("自动认证已暂停"));
        assert!(engine.snapshot.verification_hold);
        assert_eq!(engine.snapshot.settings, previous_settings);
        engine.check(true).await.unwrap();
        assert_eq!(engine.simulation_session.auth, AuthState::Unauthenticated);
    }
    #[test]
    fn conflicts_never_choose_an_arbitrary_account() {
        let mut s = Settings::default();
        for (id, account) in [("a", "one"), ("b", "two")] {
            s.rules.push(NetworkRule {
                id: id.into(),
                name: id.into(),
                interface_id: None,
                ssid: None,
                gateway: None,
                account_id: Some(account.into()),
                ignore: false,
                priority: 1,
            });
        }
        assert!(resolve_account(&s, None).is_err());
    }
    #[test]
    fn ignore_rule_wins_ties() {
        let mut s = Settings {
            selected_account: Some("one".into()),
            ..Default::default()
        };
        s.rules.push(NetworkRule {
            id: "ignore".into(),
            name: "ignore".into(),
            interface_id: None,
            ssid: None,
            gateway: None,
            account_id: None,
            ignore: true,
            priority: 1,
        });
        assert_eq!(resolve_account(&s, None).unwrap(), None);
    }

    #[tokio::test]
    async fn manual_check_of_ignored_network_does_not_report_an_old_session() {
        let (mut engine, _) = isolated("ignored-check");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine.execute(Command::Resume, &owner()).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
        engine.health_check().await;

        let mut settings = engine.snapshot.settings.clone();
        settings.rules.push(NetworkRule {
            id: "ignore".into(),
            name: "Ignore simulation".into(),
            interface_id: Some("simulation".into()),
            ssid: None,
            gateway: None,
            account_id: None,
            ignore: true,
            priority: 1,
        });
        engine
            .execute(
                Command::SettingsPut {
                    expected_revision: settings.revision,
                    settings,
                },
                &owner(),
            )
            .await
            .unwrap();
        let snapshot = engine
            .execute(Command::Check, &owner())
            .await
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(snapshot.environment, Environment::NonCampus);
        assert!(snapshot.active_binding.is_some());
        assert_eq!(snapshot.session.auth, AuthState::Unknown);
        assert_eq!(snapshot.health, HealthStatus::default());
        assert!(engine.online_window.is_none());
        assert_eq!(engine.simulation_session.auth, AuthState::Authenticated);

        engine.health_check().await;
        assert_eq!(engine.snapshot.health, HealthStatus::default());

        let login = engine
            .execute(
                Command::Login {
                    account_id: None,
                    force: false,
                    confirmed: false,
                },
                &owner(),
            )
            .await
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(login.session.auth, AuthState::Authenticated);
    }

    #[tokio::test]
    async fn manual_logout_pause_survives_restart() {
        let (mut engine, paths) = isolated("logout");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine.execute(Command::Resume, &owner()).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
        assert!(engine
            .execute(Command::Logout { confirmed: false }, &owner())
            .await
            .is_err());
        assert!(!engine.snapshot.settings.paused);
        engine
            .execute(Command::Logout { confirmed: true }, &owner())
            .await
            .unwrap();
        assert!(engine.snapshot.settings.paused);
        drop(engine);
        let (tx, _) = watch::channel(Snapshot::new(Scope::User));
        let mut restarted = Engine::open_in(Scope::User, true, tx, paths).unwrap();
        restarted.check(true).await.unwrap();
        assert_eq!(restarted.snapshot.state, EngineState::Paused);
        assert_eq!(restarted.snapshot.session.auth, AuthState::Unauthenticated);
        assert_eq!(
            restarted
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .login_successes,
            1
        );
    }
    #[tokio::test]
    async fn credential_rejection_blocks_only_until_replaced() {
        let (mut engine, _) = isolated("credentials");
        engine
            .execute(
                Command::AccountPut {
                    id: "campus".into(),
                    username: "test-user".into(),
                    password: Secret::new("wrong"),
                },
                &owner(),
            )
            .await
            .unwrap();
        let mut settings = engine.snapshot.settings.clone();
        settings.automatic = true;
        let revision = settings.revision;
        engine
            .execute(
                Command::SettingsPut {
                    settings,
                    expected_revision: revision,
                },
                &owner(),
            )
            .await
            .unwrap();
        let error = engine.check(true).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::Credentials);
        engine.set_error(error);
        for _ in 0..3 {
            engine.check(true).await.unwrap();
        }
        assert_eq!(
            engine
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .login_failures,
            1
        );
        assert!(persistence::read_settings(&engine.paths.config_file)
            .unwrap()
            .blocked_accounts
            .contains(&"campus".into()));
        engine
            .execute(
                Command::AccountPut {
                    id: "campus".into(),
                    username: "test-user".into(),
                    password: Secret::new("new-synthetic-secret"),
                },
                &owner(),
            )
            .await
            .unwrap();
        engine.check(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
        let snapshot = serde_json::to_string(&engine.snapshot).unwrap();
        assert!(!snapshot.contains("new-synthetic-secret"));
        assert!(
            !std::fs::read_to_string(engine.paths.log_dir.join("engine.jsonl"))
                .unwrap()
                .contains("new-synthetic-secret")
        );
    }
    #[tokio::test]
    async fn stale_settings_cannot_overwrite_manual_pause() {
        let (mut engine, _) = isolated("revision");
        let stale = engine.snapshot.settings.clone();
        engine.execute(Command::Pause, &owner()).await.unwrap();
        let error = engine
            .execute(
                Command::SettingsPut {
                    expected_revision: stale.revision,
                    settings: stale,
                },
                &owner(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Busy);
        assert!(engine.snapshot.settings.paused);
    }
    #[tokio::test]
    async fn degraded_internet_preserves_authenticated_session() {
        let (mut engine, _) = isolated("degraded");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine.execute(Command::Resume, &owner()).await.unwrap();
        engine.snapshot.health.internet = Health::Degraded;
        engine.check(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
        assert_eq!(engine.snapshot.state, EngineState::NetworkDegraded);
        assert_eq!(
            engine
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .login_successes,
            1
        );
    }
    #[tokio::test]
    async fn unknown_gateway_state_never_triggers_login() {
        let (mut engine, _) = isolated("unknown");
        engine.snapshot.settings.automatic = true;
        engine.snapshot.settings.selected_account = Some("demo".into());
        engine.simulation_session.auth = AuthState::Unknown;
        engine.check(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Unknown);
        assert_eq!(
            engine
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .login_successes,
            0
        );
    }
    #[tokio::test]
    async fn an_unapproved_network_never_receives_automatic_credentials() {
        let (mut engine, _) = isolated("unapproved");
        engine.snapshot.settings.automatic = true;
        engine.snapshot.settings.selected_account = Some("demo".into());
        engine.snapshot.session.auth = AuthState::Unauthenticated;
        engine.snapshot.network_trusted = false;
        engine.reconcile_auth(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Unauthenticated);
        assert_eq!(
            engine.snapshot.last_error.as_ref().unwrap().code,
            ErrorCode::ConfirmationRequired
        );
        assert_eq!(
            engine
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .login_successes,
            0
        );
        engine.snapshot.network_trusted = true;
        engine.reconcile_auth(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
    }
    #[tokio::test]
    async fn imported_settings_cannot_grant_network_trust() {
        let (mut engine, _) = isolated("trust-import");
        let mut settings = engine.snapshot.settings.clone();
        settings.trusted_networks.push("a".repeat(64));
        let revision = settings.revision;
        engine
            .execute(
                Command::SettingsPut {
                    settings,
                    expected_revision: revision,
                },
                &owner(),
            )
            .await
            .unwrap();
        assert!(engine.snapshot.settings.trusted_networks.is_empty());
        engine
            .execute(Command::TrustNetwork { trusted: true }, &owner())
            .await
            .unwrap();
        assert_eq!(
            persistence::read_settings(&engine.paths.config_file)
                .unwrap()
                .trusted_networks
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn enabling_automatic_requires_the_displayed_network_and_saves_without_logging_in() {
        let (mut engine, paths) = isolated("enable-automatic");
        engine.check(false).await.unwrap();
        let fingerprint = network_fingerprint(&engine.snapshot).unwrap();

        let error = engine
            .execute(
                Command::EnableAutomatic {
                    expected_network: fingerprint.clone(),
                },
                &owner(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Credentials);
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        let before = engine.snapshot.settings.clone();
        let error = engine
            .execute(
                Command::EnableAutomatic {
                    expected_network: "stale".into(),
                },
                &owner(),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::ConfirmationRequired);
        assert_eq!(engine.snapshot.settings, before);
        assert_eq!(
            persistence::read_settings(&paths.config_file).unwrap(),
            before
        );

        let snapshot = engine
            .execute(
                Command::EnableAutomatic {
                    expected_network: fingerprint.clone(),
                },
                &owner(),
            )
            .await
            .unwrap()
            .snapshot()
            .unwrap();
        assert_eq!(snapshot.settings.revision, before.revision + 1);
        assert!(snapshot.settings.automatic);
        assert_eq!(snapshot.settings.trusted_networks, vec![fingerprint]);
        assert_eq!(
            persistence::read_settings(&paths.config_file).unwrap(),
            snapshot.settings
        );
        assert_eq!(snapshot.session.auth, AuthState::Unauthenticated);
        assert_eq!(engine.simulation_session.auth, AuthState::Unauthenticated);

        engine.check(true).await.unwrap();
        assert_eq!(engine.snapshot.session.auth, AuthState::Authenticated);
    }

    #[test]
    fn bit_mobile_requires_exact_wireless_ssid_on_the_active_binding() {
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.interfaces.push(NetworkInterface {
            id: "wlan".into(),
            name: "Wi-Fi".into(),
            index: 1,
            ipv4: vec![Ipv4Addr::new(10, 0, 1, 2)],
            gateway: None,
            up: true,
            wireless: true,
            ssid: Some("BIT-mobile".into()),
            virtual_adapter: false,
            alternate_adapter: false,
        });
        assert!(!uses_system_authentication(&snapshot));
        snapshot.active_binding = snapshot.interfaces[0].binding();
        assert!(uses_system_authentication(&snapshot));
        snapshot.interfaces[0].ssid = Some("BIT-mobile-guest".into());
        assert!(!uses_system_authentication(&snapshot));
        snapshot.interfaces[0].ssid = Some("BIT-mobile".into());
        snapshot.interfaces[0].wireless = false;
        assert!(!uses_system_authentication(&snapshot));
    }

    #[tokio::test]
    async fn profiles_restore_their_own_target_and_account_without_unpausing() {
        let (mut engine, _) = isolated("profiles");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine
            .execute(
                Command::ProfileSave {
                    id: "campus".into(),
                    name: "Campus".into(),
                },
                &owner(),
            )
            .await
            .unwrap();
        let mut settings = engine.snapshot.settings.clone();
        settings.gateway.base_url = "http://10.0.0.99".into();
        settings.selected_account = None;
        let revision = settings.revision;
        engine
            .execute(
                Command::SettingsPut {
                    settings,
                    expected_revision: revision,
                },
                &owner(),
            )
            .await
            .unwrap();
        engine.execute(Command::Pause, &owner()).await.unwrap();
        engine
            .execute(
                Command::ProfileSelect {
                    id: "campus".into(),
                },
                &owner(),
            )
            .await
            .unwrap();
        assert_eq!(
            engine.snapshot.settings.gateway.base_url,
            "http://10.0.0.55"
        );
        assert_eq!(
            engine.snapshot.settings.selected_account.as_deref(),
            Some("demo")
        );
        assert!(engine.snapshot.settings.paused);
        engine.snapshot.scope = Scope::System;
        engine.snapshot.settings.gateway.base_url = "http://10.0.0.99".into();
        assert_eq!(
            engine
                .execute(
                    Command::ProfileSelect {
                        id: "campus".into()
                    },
                    &owner()
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
    }
    #[test]
    fn trust_fingerprint_changes_with_network_context() {
        let (mut engine, _) = isolated("fingerprint");
        // Populate the same non-secret context that the OS collector supplies.
        engine.snapshot.active_binding = Some(NetworkBinding {
            interface_id: "test".into(),
            interface_index: 1,
            source_ip: "10.2.3.4".parse().unwrap(),
        });
        engine.snapshot.interfaces.push(NetworkInterface {
            id: "test".into(),
            name: "Test".into(),
            index: 1,
            ipv4: vec![],
            gateway: Some("10.2.3.1".parse().unwrap()),
            up: true,
            wireless: true,
            ssid: Some("Campus".into()),
            virtual_adapter: false,
            alternate_adapter: false,
        });
        let first = network_fingerprint(&engine.snapshot).unwrap();
        engine.snapshot.interfaces[0].ssid = Some("Home".into());
        assert_ne!(first, network_fingerprint(&engine.snapshot).unwrap());
    }
    #[test]
    fn stable_networks_gradually_reduce_checks_within_the_configured_limit() {
        assert_eq!(
            (1..=5)
                .map(|n| stable_check_delay(n, 60))
                .collect::<Vec<_>>(),
            [10, 20, 40, 60, 60]
        );
        assert_eq!(stable_check_delay(u32::MAX, 5), 5);
    }
    #[test]
    fn log_retention_expires_old_events_inside_a_recently_written_file() {
        let (engine, paths) = isolated("retention");
        drop(engine);
        std::fs::create_dir_all(&paths.log_dir).unwrap();
        let path = paths.log_dir.join("engine.jsonl");
        std::fs::write(
            &path,
            format!(
                "{{\"at\":1,\"kind\":\"old\"}}\n{{\"at\":{},\"kind\":\"recent\"}}\n",
                now_secs()
            ),
        )
        .unwrap();
        prune_logs(&paths.log_dir, 14);
        let text = std::fs::read_to_string(path).unwrap();
        assert!(!text.contains("old"));
        assert!(text.contains("recent"));
    }
    #[tokio::test]
    async fn ipv6_diagnostic_proves_transport_on_a_loopback_fixture() {
        let Ok(listener) = tokio::net::TcpListener::bind("[::1]:0").await else {
            return;
        };
        let address = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move {
            listener.accept().await.unwrap();
        });
        assert_eq!(
            ipv6_health(&[format!("http://{address}")], 3).await,
            Health::Healthy
        );
        accepted.await.unwrap();
    }
    #[tokio::test]
    async fn a_confirmed_lost_session_records_one_reconnect_and_online_intervals() {
        let (mut engine, _) = isolated("reconnect");
        engine
            .execute(Command::AccountSelect { id: "demo".into() }, &owner())
            .await
            .unwrap();
        engine.execute(Command::Resume, &owner()).await.unwrap();
        let started = std::time::Instant::now() - Duration::from_secs(42);
        engine.online_window = Some((started, started + Duration::from_secs(40)));
        engine.simulation_session.auth = AuthState::Unauthenticated;
        engine.check(true).await.unwrap();
        let stats = engine.history.as_ref().unwrap().statistics().unwrap();
        assert_eq!(stats.reconnects, 1);
        assert_eq!(stats.session_interruptions, 1);
        assert_eq!(stats.observed_online_seconds, 40);
        engine.check(true).await.unwrap();
        assert_eq!(
            engine
                .history
                .as_ref()
                .unwrap()
                .statistics()
                .unwrap()
                .reconnects,
            1
        );
        assert!(stats.average_recovery_ms.is_some());
    }
}
