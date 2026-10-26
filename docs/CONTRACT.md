# Shared contract for parallel implementation

Authoritative models: crates/bitnet-core/src/lib.rs. Do not edit that file without coordination.

## Gateway crate

Export GatewayClient::new(GatewayConfig) -> Result<Self, GatewayError>.
Async methods: status(&self, Option<&NetworkBinding>) -> Result<GatewaySession, GatewayError>; login(&self, username: &str, password: &str, Option<&NetworkBinding>) -> Result<GatewaySession, GatewayError>; logout(&self, username: &str, Option<&NetworkBinding>) -> Result<GatewaySession, GatewayError>.
GatewayError must map to AppError via into_app_error(). Never expose request URLs/material. Allow unbound connections ONLY to literal loopback fixtures. Authentication otherwise requires binding. Endpoint custom configuration is administrator controlled in service mode.

## Platform crate

Export paths(scope: Scope) -> Result<AppPaths, AppError>, AppPaths { data_dir: PathBuf, config_file: PathBuf, credentials_file: PathBuf, history_file: PathBuf, log_dir: PathBuf }.
credentials::CredentialStore::open(scope, data_dir: &Path) -> Result<Self, AppError>; load() -> Result<Vec<AccountSecret>, AppError>; save(&[AccountSecret]) -> Result<(), AppError>.
network::interfaces() -> Result<Vec<NetworkInterface>, AppError>; network change notification ideally supports a channel or monotonic generation, polling fallback explicit.
service::status() -> ServiceStatus; service::is_admin() -> bool; service::run(callback: impl FnOnce() -> Result<(), AppError> + Send + 'static) -> Result<(), AppError>. Platform daemon callbacks must not log or persist plaintext credentials. Never install/start host services during tests.

## Runtime SDK for GUI/CLI

bitnet_runtime::client::Client::new(scope: Scope) -> Self.
async command(&self, Command) -> Result<ReplyData, AppError>.
async status(&self) -> Result<Snapshot, AppError>.
async ensure_running(&self) -> Result<(), AppError>: may start user engine; NEVER falls back to user engine when system service is installed but unavailable.
Client is Clone + Send + Sync. UI can use iced::Task::perform. The SDK validates server identity before sending secrets.

All command mutation is serialized by the engine. GUI/CLI use command outcomes and then refresh snapshots. Maintain API v1 compatibility and explicit unsupported operations.
