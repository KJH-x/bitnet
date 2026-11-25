//! The only installation/startup/update entry point shared by GUI and CLI.
#[path = "ipv6_maintenance.rs"]
mod ipv6;
#[path = "updates.rs"]
pub mod updates;
use bitnet_core::{AppError, ErrorCode, NetworkInterface, Scope};
use fs2::FileExt;
pub use ipv6::{
    acknowledge_ipv6_stack_reset, ipv6_stack_reset_status, is_reset_id, reset_id_from_backup,
    Ipv6StackResetStatus,
};
use serde_json::json;
use std::io::Read;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, File},
    io::Write,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Debug, Clone)]
pub enum MaintenanceAction {
    ServiceInstall,
    ServiceStart,
    ServiceStop,
    ServiceRemove { purge: bool },
    Repair,
    CheckUpdate,
    InstallUpdate,
    InstallCheckedUpdate { manifest_sha256: UpdateManifestId },
    RollbackUpdate,
    SetAutostart { enabled: bool },
    SetGuiAutostart { enabled: bool },
    EnableIpv6 { interface: InterfaceTarget },
    DisableIpv6 { interface: InterfaceTarget },
    ResetIpv6 { interface: InterfaceTarget },
    RenewIpv6 { interface: InterfaceTarget },
    RenewDhcpv6 { interface: InterfaceTarget },
    RestartIpv6Adapter { interface: InterfaceTarget },
    FlushDnsCache,
    ResetIpv6Stack,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateManifestId(String);

impl UpdateManifestId {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            Ok(Self(value.to_ascii_lowercase()))
        } else {
            Err(err(
                ErrorCode::InvalidInput,
                "Expected a SHA-256 update manifest identifier",
            ))
        }
    }

    fn for_payload(payload: &[u8]) -> Self {
        use sha2::{Digest as _, Sha256};
        Self(format!("{:x}", Sha256::digest(payload)))
    }

    fn verify(&self, payload: &[u8]) -> Result<(), AppError> {
        if *self != Self::for_payload(payload) {
            return Err(err(
                ErrorCode::Busy,
                "The verified update changed. Check the new release before installing.",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct CheckedUpdate {
    pub version: String,
    pub scope: Scope,
    pub manifest_sha256: UpdateManifestId,
}

/// Fetch and authenticate the current release without downloading any installers.
pub async fn check_installable_update(scope: Scope) -> Result<Option<CheckedUpdate>, AppError> {
    let config = updates::UpdateConfig::embedded()?;
    let Some(release) = config.check_available_release().await? else {
        return Ok(None);
    };
    let _ = release.manifest.assets_for_install(scope)?;
    Ok(Some(CheckedUpdate {
        version: release.manifest.version,
        scope,
        manifest_sha256: UpdateManifestId::for_payload(&release.raw_manifest),
    }))
}

/// Native Windows adapter identity. Alias, address and display name are never
/// used as a privileged operation's selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InterfaceTarget {
    guid: String,
    index: u32,
}

impl InterfaceTarget {
    pub fn from_interface(interface: &NetworkInterface) -> Result<Self, AppError> {
        Self::new(&interface.id, interface.index)
    }

    pub fn parse(selector: &str) -> Result<Self, AppError> {
        let (guid, index) = selector
            .split_once('@')
            .ok_or_else(|| err(ErrorCode::InvalidInput, "Expected adapter GUID@index"))?;
        let index = index
            .parse()
            .map_err(|_| err(ErrorCode::InvalidInput, "Invalid Windows interface index"))?;
        Self::new(guid, index)
    }

    fn new(guid: &str, index: u32) -> Result<Self, AppError> {
        let guid = guid
            .strip_prefix('{')
            .and_then(|guid| guid.strip_suffix('}'))
            .unwrap_or(guid);
        if index == 0
            || guid.len() != 36
            || !guid.bytes().enumerate().all(|(position, byte)| {
                if matches!(position, 8 | 13 | 18 | 23) {
                    byte == b'-'
                } else {
                    byte.is_ascii_hexdigit()
                }
            })
            || guid == "00000000-0000-0000-0000-000000000000"
        {
            return Err(err(
                ErrorCode::InvalidInput,
                "A current Windows adapter GUID and nonzero index are required",
            ));
        }
        Ok(Self {
            guid: guid.to_ascii_lowercase(),
            index,
        })
    }
}

impl MaintenanceAction {
    pub fn ipv6_target(&self) -> Option<&InterfaceTarget> {
        match self {
            Self::EnableIpv6 { interface }
            | Self::DisableIpv6 { interface }
            | Self::ResetIpv6 { interface }
            | Self::RenewIpv6 { interface }
            | Self::RenewDhcpv6 { interface }
            | Self::RestartIpv6Adapter { interface } => Some(interface),
            _ => None,
        }
    }

    fn is_ipv6_maintenance(&self) -> bool {
        self.ipv6_target().is_some() || matches!(self, Self::FlushDnsCache | Self::ResetIpv6Stack)
    }
}
#[derive(Debug, Clone)]
pub struct MaintenanceResult {
    pub message: String,
    pub details: serde_json::Value,
}
fn err(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message)
}
fn result(message: &str) -> MaintenanceResult {
    MaintenanceResult {
        message: message.into(),
        details: json!({}),
    }
}

pub fn action_name(action: &MaintenanceAction) -> &'static str {
    match action {
        MaintenanceAction::ServiceInstall => "service-install",
        MaintenanceAction::ServiceStart => "service-start",
        MaintenanceAction::ServiceStop => "service-stop",
        MaintenanceAction::ServiceRemove { .. } => "service-remove",
        MaintenanceAction::Repair => "repair",
        MaintenanceAction::CheckUpdate => "check-update",
        MaintenanceAction::InstallUpdate | MaintenanceAction::InstallCheckedUpdate { .. } => {
            "install-update"
        }
        MaintenanceAction::RollbackUpdate => "rollback-update",
        MaintenanceAction::SetAutostart { enabled: true } => "autostart-on",
        MaintenanceAction::SetAutostart { enabled: false } => "autostart-off",
        MaintenanceAction::SetGuiAutostart { enabled: true } => "gui-autostart-on",
        MaintenanceAction::SetGuiAutostart { enabled: false } => "gui-autostart-off",
        MaintenanceAction::EnableIpv6 { .. } => "ipv6-enable",
        MaintenanceAction::DisableIpv6 { .. } => "ipv6-disable",
        MaintenanceAction::ResetIpv6 { .. } => "ipv6-reset",
        MaintenanceAction::RenewIpv6 { .. } => "ipv6-renew",
        MaintenanceAction::RenewDhcpv6 { .. } => "ipv6-renew-dhcp",
        MaintenanceAction::RestartIpv6Adapter { .. } => "ipv6-restart-adapter",
        MaintenanceAction::FlushDnsCache => "ipv6-flush-dns",
        MaintenanceAction::ResetIpv6Stack => "ipv6-stack-reset",
    }
}

pub fn parse_action(name: &str, purge: bool) -> Result<MaintenanceAction, AppError> {
    parse_action_with_interface(name, purge, None)
}

pub fn parse_action_with_interface(
    name: &str,
    purge: bool,
    interface: Option<InterfaceTarget>,
) -> Result<MaintenanceAction, AppError> {
    if purge && name != "service-remove" {
        return Err(err(
            ErrorCode::InvalidInput,
            "Purge is valid only for service removal",
        ));
    }
    if matches!(
        name,
        "ipv6-enable"
            | "ipv6-disable"
            | "ipv6-reset"
            | "ipv6-renew"
            | "ipv6-renew-dhcp"
            | "ipv6-restart-adapter"
    ) {
        let interface = interface.ok_or_else(|| {
            err(
                ErrorCode::InvalidInput,
                "IPv6 maintenance requires --interface GUID@index",
            )
        })?;
        return Ok(match name {
            "ipv6-enable" => MaintenanceAction::EnableIpv6 { interface },
            "ipv6-disable" => MaintenanceAction::DisableIpv6 { interface },
            "ipv6-renew" => MaintenanceAction::RenewIpv6 { interface },
            "ipv6-renew-dhcp" => MaintenanceAction::RenewDhcpv6 { interface },
            "ipv6-restart-adapter" => MaintenanceAction::RestartIpv6Adapter { interface },
            _ => MaintenanceAction::ResetIpv6 { interface },
        });
    }
    if interface.is_some() {
        return Err(err(
            ErrorCode::InvalidInput,
            "--interface is valid only for IPv6 maintenance",
        ));
    }
    Ok(match name {
        "service-install" => MaintenanceAction::ServiceInstall,
        "service-start" => MaintenanceAction::ServiceStart,
        "service-stop" => MaintenanceAction::ServiceStop,
        "service-remove" => MaintenanceAction::ServiceRemove { purge },
        "repair" => MaintenanceAction::Repair,
        "check-update" => MaintenanceAction::CheckUpdate,
        "install-update" => MaintenanceAction::InstallUpdate,
        "rollback-update" => MaintenanceAction::RollbackUpdate,
        "autostart-on" => MaintenanceAction::SetAutostart { enabled: true },
        "autostart-off" => MaintenanceAction::SetAutostart { enabled: false },
        "gui-autostart-on" => MaintenanceAction::SetGuiAutostart { enabled: true },
        "gui-autostart-off" => MaintenanceAction::SetGuiAutostart { enabled: false },
        "ipv6-flush-dns" => MaintenanceAction::FlushDnsCache,
        "ipv6-stack-reset" => MaintenanceAction::ResetIpv6Stack,
        _ => return Err(err(ErrorCode::InvalidInput, "Unknown maintenance action")),
    })
}

pub async fn execute(
    scope: Scope,
    action: MaintenanceAction,
    interactive: bool,
) -> Result<MaintenanceResult, AppError> {
    #[cfg(windows)]
    if scope == Scope::System
        && matches!(
            action,
            MaintenanceAction::ServiceInstall | MaintenanceAction::ServiceStart
        )
    {
        // Reject absent trust material or a corrupt cached MSI before UAC and
        // before disrupting the user's functioning authentication controller.
        let installing = matches!(action, MaintenanceAction::ServiceInstall);
        if installing {
            updates::verify_offline_package(&sibling("packages/Service.msi")?)?;
        }
        let _handoff = HandoffLock::acquire()?;
        let stopped = stop_personal_controller().await?;
        let outcome = execute_inner(scope, action, interactive).await;
        let service = bitnet_platform::service::status();
        if outcome.is_err()
            && (if installing {
                restore_personal_controller(stopped, &service)
            } else {
                stopped && !service.running && service.detail.is_none()
            })
            && restart_personal_controller().await.is_err()
        {
            return Err(err(
                ErrorCode::Unavailable,
                "Service installation failed and the previous personal controller could not be restored; restart the personal client",
            ));
        }
        return outcome;
    }
    execute_inner(scope, action, interactive).await
}

/// Entry point used only by the newly authorized helper. The helper receives
/// closed action names and independently verifies every installation payload.
pub async fn execute_elevated(
    scope: Scope,
    action: MaintenanceAction,
) -> Result<MaintenanceResult, AppError> {
    if !bitnet_platform::service::is_admin() {
        return Err(err(
            ErrorCode::PermissionDenied,
            "A real administrator token is required",
        ));
    }
    execute_inner(scope, action, false).await
}

/// Whether another process holds the current user's live controller-handoff
/// lock. A stale unlocked file never prevents normal startup.
pub fn maintenance_in_progress() -> bool {
    let Ok(paths) = bitnet_platform::paths(Scope::User) else {
        return true;
    };
    lock_in_progress(&paths.data_dir.join("maintenance.handoff.lock"))
}

fn lock_in_progress(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    let Ok(file) = open_handoff_lock(path, false) else {
        return true;
    };
    match file.try_lock_exclusive() {
        Ok(()) => FileExt::unlock(&file).is_err(),
        Err(_) => true,
    }
}

fn open_handoff_lock(path: &Path, create: bool) -> Result<File, AppError> {
    reject_links(path)?;
    let mut options = fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(create)
        .truncate(false);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(7).custom_flags(0x0020_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options.open(path).map_err(|_| {
        err(
            ErrorCode::PermissionDenied,
            "Could not open the maintenance handoff lock",
        )
    })
}

#[cfg(windows)]
struct HandoffLock(File);
#[cfg(windows)]
impl HandoffLock {
    fn acquire() -> Result<Self, AppError> {
        let path = bitnet_platform::paths(Scope::User)?.data_dir;
        reject_links(&path)?;
        fs::create_dir_all(&path).map_err(|_| {
            err(
                ErrorCode::PermissionDenied,
                "Could not create the maintenance state directory",
            )
        })?;
        let file = open_handoff_lock(&path.join("maintenance.handoff.lock"), true)?;
        file.try_lock_exclusive()
            .map_err(|_| err(ErrorCode::Busy, "Another maintenance handoff is active"))?;
        Ok(Self(file))
    }
}
#[cfg(windows)]
impl Drop for HandoffLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

async fn execute_inner(
    scope: Scope,
    action: MaintenanceAction,
    interactive: bool,
) -> Result<MaintenanceResult, AppError> {
    if action.is_ipv6_maintenance() {
        if !cfg!(windows) {
            return Err(err(
                ErrorCode::Unavailable,
                "IPv6 maintenance is currently supported on Windows",
            ));
        }
        if scope != Scope::System {
            return Err(err(
                ErrorCode::InvalidInput,
                "IPv6 maintenance requires system scope",
            ));
        }
    }
    if matches!(
        action,
        MaintenanceAction::InstallUpdate | MaintenanceAction::InstallCheckedUpdate { .. }
    ) {
        // A build without an authenticated feed never prompts for privileges.
        let _ = updates::UpdateConfig::embedded()?;
    }
    if matches!(action, MaintenanceAction::CheckUpdate) {
        let config = updates::UpdateConfig::embedded()?;
        let Some(manifest) = config.check_available().await? else {
            return Ok(MaintenanceResult {
                message: "No newer stable update is available".into(),
                details: json!({"available":false,"signature_verified":true}),
            });
        };
        let assets = manifest.assets_for_install(scope)?;
        return Ok(MaintenanceResult {
            message: format!("Verified update {} is available", manifest.version),
            details: json!({"available":true,"version":manifest.version,"assets":assets,"signature_verified":true}),
        });
    }
    if requires_admin(scope, &action) && !bitnet_platform::service::is_admin() {
        if !interactive {
            return Err(err(ErrorCode::PermissionDenied,"Administrator authorization is required; noninteractive maintenance cannot elevate"));
        }
        return elevate(scope, action).await;
    }
    #[cfg(windows)]
    if action.is_ipv6_maintenance() {
        return ipv6::execute(&action).await;
    }
    match action {
        MaintenanceAction::SetAutostart { enabled } if scope == Scope::User => {
            return set_autostart(false, enabled)
        }
        MaintenanceAction::SetGuiAutostart { enabled } => {
            if scope != Scope::User {
                return Err(err(
                    ErrorCode::InvalidInput,
                    "GUI startup belongs to a user session",
                ));
            }
            return set_autostart(true, enabled);
        }
        _ => {}
    }
    if matches!(
        action,
        MaintenanceAction::InstallUpdate | MaintenanceAction::InstallCheckedUpdate { .. }
    ) {
        let config = updates::UpdateConfig::embedded()?;
        let release = config.check_release().await?;
        if let MaintenanceAction::InstallCheckedUpdate { manifest_sha256 } = &action {
            manifest_sha256.verify(&release.raw_manifest)?;
        }
        let assets = release.manifest.assets_for_install(scope)?;
        // Native Debian/PKG installers modify the machine even when invoked by
        // the personal client. Their payload must stay in root-owned staging.
        #[cfg(windows)]
        let stage = Staging::new(scope)?;
        #[cfg(not(windows))]
        let stage = Staging::new(Scope::System)?;
        let mut packages = Vec::new();
        for asset in assets {
            let package = config.download(asset, &stage.directory).await?;
            updates::verify_asset(&package, asset)?;
            packages.push(package);
        }
        let reboot = run(packages_plan(&packages, scope)?).await?;
        #[cfg(windows)]
        if scope == Scope::System {
            configure_windows_recovery()?;
        }
        let retention = updates::cache::Cache::open(
            &stage.root,
            scope,
            scope == Scope::System || cfg!(not(windows)),
        )
        .and_then(|cache| {
            cache.record_update(&config, &release, &packages, bitnet_core::APP_VERSION)
        });
        let (cache_details, message) = match retention {
            Ok(status) if status.skipped_limit => (json!(status), "Verified update installer completed; the new release was not cached because the shared 1 GiB cache limit would be exceeded"),
            Ok(status) => (json!(status), "Verified update installer completed"),
            Err(_) => (json!({"active_cached":false,"cache_warning":true}), "Verified update installer completed; previous-version cache could not be saved, so rollback may be unavailable"),
        };
        return Ok(MaintenanceResult {
            message: message.into(),
            details: json!({"version":release.manifest.version,"reboot_required":reboot,"cache":cache_details}),
        });
    }
    if matches!(action, MaintenanceAction::RollbackUpdate) {
        let config = updates::UpdateConfig::embedded().map_err(|_| err(ErrorCode::Unavailable, "Rollback is unavailable without the compiled release verification configuration and a compatible previous cache"))?;
        #[cfg(windows)]
        let stage = Staging::new(scope)?;
        #[cfg(not(windows))]
        let stage = Staging::new(Scope::System)?;
        let cache = updates::cache::Cache::open(
            &stage.root,
            scope,
            scope == Scope::System || cfg!(not(windows)),
        )?;
        let previous = cache.previous(&config, bitnet_core::APP_VERSION)?;
        let version = semver::Version::parse(&previous.release.manifest.version)
            .map_err(|_| err(ErrorCode::Incompatible, "Invalid cached version"))?;
        if version < semver::Version::new(0, 1, 2) {
            let paths = bitnet_platform::paths(scope)?;
            let settings = crate::persistence::read_settings(&paths.config_file)?;
            let has_extended_gateway_settings = |gateway: &bitnet_core::GatewayConfig| {
                gateway.request_ip.is_some() || gateway.dumb_terminal
            };
            if has_extended_gateway_settings(&settings.gateway)
                || settings
                    .profiles
                    .iter()
                    .any(|profile| has_extended_gateway_settings(&profile.gateway))
            {
                return Err(err(ErrorCode::Incompatible, "Clear the request IP and terminal logout options in the active gateway and saved profiles before rolling back below 0.1.2"));
            }
        }
        let assets = previous.release.manifest.assets_for_install(scope)?;
        let mut packages = Vec::new();
        for (asset, source) in assets.iter().zip(&previous.packages) {
            let destination = stage.directory.join(&asset.filename);
            copy_bounded(source, &destination)?;
            updates::verify_asset(&destination, asset)?;
            packages.push(destination);
        }
        // Recheck the exact signed metadata after copying. Historical versions
        // are accepted only by this explicit, already-authorized rollback path.
        config
            .verify_cached_release(&previous.release.raw_manifest, &previous.release.signature)?;
        let reboot = run(rollback_packages_plan(&packages, scope)?).await?;
        #[cfg(windows)]
        if scope == Scope::System {
            configure_windows_recovery()?;
        }
        let index_updated = cache
            .record_rollback(&previous, bitnet_core::APP_VERSION)
            .is_ok();
        return Ok(MaintenanceResult {
            message: if index_updated {
                "Explicit rollback installer completed"
            } else {
                "Rollback installer completed; local cache state could not be updated"
            }
            .into(),
            details: json!({"version":previous.release.manifest.version,"reboot_required":reboot,"cache_index_updated":index_updated}),
        });
    }
    if scope != Scope::System {
        return Err(err(
            ErrorCode::InvalidInput,
            "Service maintenance requires system scope",
        ));
    }
    service_action(action).await
}

#[cfg(windows)]
async fn stop_personal_controller() -> Result<bool, AppError> {
    match crate::ipc::request(Scope::User, bitnet_core::Command::Status).await {
        Err(error) if error.code == ErrorCode::Unavailable => return Ok(false),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    // Call the personal endpoint directly: Client routes to the installed
    // system service, which is intentionally not the controller being handed off.
    crate::ipc::request(Scope::User, bitnet_core::Command::Shutdown).await?;
    let path = bitnet_platform::paths(Scope::User)?
        .data_dir
        .join("engine.lock");
    // No installer is started until the controller's OS lock is released.
    wait_controller_exit(&path).await?;
    Ok(true)
}

#[cfg(any(windows, test))]
fn restore_personal_controller(stopped: bool, service: &bitnet_core::ServiceStatus) -> bool {
    // A partially registered system service owns subsequent recovery. Starting
    // a competing personal engine would race the SCM recovery configuration.
    stopped && !service.installed && !service.running
}

#[cfg(any(windows, test))]
async fn wait_controller_exit(path: &Path) -> Result<(), AppError> {
    let file = open_handoff_lock(path, false)?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => {
                FileExt::unlock(&file)
                    .map_err(|_| err(ErrorCode::Internal, "Could not release the handoff probe"))?;
                return Ok(());
            }
            Err(error) if error.raw_os_error() != fs2::lock_contended_error().raw_os_error() => {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Could not verify controller handoff",
                ));
            }
            Err(_) if Instant::now() >= deadline => {
                return Err(err(
                    ErrorCode::Busy,
                    "The personal controller did not stop; installation was not started",
                ));
            }
            Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
}

#[cfg(windows)]
async fn restart_personal_controller() -> Result<(), AppError> {
    use std::os::windows::process::CommandExt;
    let mut process = Command::new(sibling("bitengine.exe")?);
    process
        .arg("--user")
        .creation_flags(0x0800_0000 | 0x0000_0008)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not restore the personal controller",
            )
        })?;
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if crate::ipc::request(Scope::User, bitnet_core::Command::Status)
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(err(
        ErrorCode::Unavailable,
        "The restored personal controller did not become ready",
    ))
}
fn requires_admin(scope: Scope, action: &MaintenanceAction) -> bool {
    if matches!(
        action,
        MaintenanceAction::CheckUpdate | MaintenanceAction::SetGuiAutostart { .. }
    ) {
        return false;
    }
    #[cfg(not(windows))]
    if matches!(
        action,
        MaintenanceAction::InstallUpdate
            | MaintenanceAction::InstallCheckedUpdate { .. }
            | MaintenanceAction::RollbackUpdate
    ) {
        return true;
    }
    scope == Scope::System
}

async fn service_action(action: MaintenanceAction) -> Result<MaintenanceResult, AppError> {
    #[cfg(windows)]
    let mut reboot_required = false;
    #[cfg(not(windows))]
    let reboot_required = false;
    #[cfg(windows)]
    match action {
        MaintenanceAction::ServiceInstall => {
            let source = sibling("packages/Service.msi")?;
            updates::verify_offline_package(&source)?;
            let stage = Staging::new(Scope::System)?;
            let target = stage.directory.join("Service.msi");
            copy_bounded(&source, &target)?;
            let signature = updates::read_small_file(&source.with_extension("msi.minisig"), 8192)?;
            let mut output = updates::create_file(&target.with_extension("msi.minisig"))?;
            output
                .write_all(&signature)
                .and_then(|_| output.sync_all())
                .map_err(|_| {
                    err(
                        ErrorCode::Unavailable,
                        "Could not stage the installer signature",
                    )
                })?;
            drop(output);
            updates::verify_offline_package(&target)?;
            let reboot = run(package_plan(&target, Scope::System)?).await?;
            configure_windows_recovery()?;
            if !reboot {
                wait_windows_service(true).await?;
            }
            return Ok(MaintenanceResult {
                message: "Verified service installation completed".into(),
                details: json!({"reboot_required":reboot}),
            });
        }
        MaintenanceAction::ServiceStart => {
            // Older BitNet versions disabled the service when stopping it.
            if windows_service_disabled()? {
                run(system_plan(
                    "sc.exe",
                    &["config", "BitNet", "start=", "demand"],
                    45,
                )?)
                .await?;
            }
            change_windows_service_state(true).await?;
        }
        MaintenanceAction::ServiceStop => {
            change_windows_service_state(false).await?;
        }
        MaintenanceAction::ServiceRemove { purge } => {
            reboot_required = run(system_plan(
                "msiexec.exe",
                &["/x", &service_product()?, "/passive", "/norestart"],
                900,
            )?)
            .await?;
            if purge {
                purge_system_data()?;
            }
        }
        MaintenanceAction::Repair => {
            reboot_required = run(system_plan(
                "msiexec.exe",
                &["/fomus", &service_product()?, "/passive", "/norestart"],
                900,
            )?)
            .await?;
            configure_windows_recovery()?;
        }
        MaintenanceAction::SetAutostart { enabled } => {
            run(system_plan(
                "sc.exe",
                &[
                    "config",
                    "BitNet",
                    "start=",
                    if enabled { "auto" } else { "demand" },
                ],
                45,
            )?)
            .await?;
        }
        _ => return Err(err(ErrorCode::InvalidInput, "Unsupported system action")),
    }
    #[cfg(target_os = "linux")]
    match action {
        MaintenanceAction::ServiceInstall => {
            installed_linux_unit()?;
            run(plan("/usr/bin/systemctl", &["daemon-reload"], 30)).await?;
            run(plan(
                "/usr/bin/systemctl",
                &["enable", "--now", "bitnet.service"],
                45,
            ))
            .await?;
        }
        MaintenanceAction::ServiceStart => {
            run(plan("/usr/bin/systemctl", &["start", "bitnet.service"], 45)).await?;
        }
        MaintenanceAction::ServiceStop => {
            run(plan("/usr/bin/systemctl", &["stop", "bitnet.service"], 45)).await?;
        }
        MaintenanceAction::SetAutostart { enabled } => {
            run(plan(
                "/usr/bin/systemctl",
                &[if enabled { "enable" } else { "disable" }, "bitnet.service"],
                30,
            ))
            .await?;
        }
        MaintenanceAction::Repair => {
            installed_linux_unit()?;
            for args in [
                vec!["daemon-reload"],
                vec!["reset-failed", "bitnet.service"],
                vec!["restart", "bitnet.service"],
            ] {
                run(plan("/usr/bin/systemctl", &args, 45)).await?;
            }
        }
        MaintenanceAction::ServiceRemove { purge } => {
            run(plan(
                "/usr/bin/systemctl",
                &["disable", "--now", "bitnet.service"],
                45,
            ))
            .await?;
            for path in [
                "/etc/systemd/system/bitnet.service",
                "/usr/lib/systemd/system/bitnet.service",
                "/lib/systemd/system/bitnet.service",
            ] {
                remove_file(Path::new(path))?;
            }
            run(plan("/usr/bin/systemctl", &["daemon-reload"], 30)).await?;
            if purge {
                purge_system_data()?;
            }
        }
        _ => return Err(err(ErrorCode::InvalidInput, "Unsupported system action")),
    }
    #[cfg(target_os = "macos")]
    match action {
        MaintenanceAction::ServiceInstall => {
            provision_mac_plists()?;
            let path = Path::new("/Library/LaunchDaemons/top.nsapi.bitnet.plist");
            root_helper(path)?;
            run(plan(
                "/bin/launchctl",
                &["enable", "system/top.nsapi.bitnet"],
                30,
            ))
            .await?;
            mac_start(false).await?;
        }
        MaintenanceAction::ServiceStart => {
            mac_start(false).await?;
        }
        MaintenanceAction::ServiceStop => {
            mac_stop().await?;
        }
        MaintenanceAction::SetAutostart { enabled } => {
            run(plan(
                "/bin/launchctl",
                &[
                    if enabled { "enable" } else { "disable" },
                    "system/top.nsapi.bitnet",
                ],
                30,
            ))
            .await?;
        }
        MaintenanceAction::Repair => {
            provision_mac_plists()?;
            run(plan(
                "/bin/launchctl",
                &["enable", "system/top.nsapi.bitnet"],
                30,
            ))
            .await?;
            mac_start(true).await?;
        }
        MaintenanceAction::ServiceRemove { purge } => {
            mac_stop().await?;
            mac_stop_job("system/top.nsapi.bitnet.keychain").await?;
            remove_file(Path::new("/Library/LaunchDaemons/top.nsapi.bitnet.plist"))?;
            remove_file(Path::new(
                "/Library/LaunchDaemons/top.nsapi.bitnet.keychain.plist",
            ))?;
            if purge {
                purge_system_data()?;
            }
        }
        _ => return Err(err(ErrorCode::InvalidInput, "Unsupported system action")),
    }
    Ok(MaintenanceResult {
        message: "System maintenance completed".into(),
        details: json!({"service":bitnet_platform::service::status(),"reboot_required":reboot_required}),
    })
}

#[cfg(target_os = "linux")]
fn installed_linux_unit() -> Result<(), AppError> {
    for path in [
        "/etc/systemd/system/bitnet.service",
        "/usr/lib/systemd/system/bitnet.service",
    ] {
        let path = Path::new(path);
        if path.try_exists().map_err(|_| {
            err(
                ErrorCode::PermissionDenied,
                "Could not inspect the installed system unit",
            )
        })? {
            // /lib is a symlink to /usr/lib on supported Debian/Ubuntu systems;
            // use the canonical package path and verify all root-controlled parents.
            return root_helper(path);
        }
    }
    let target = Path::new("/etc/systemd/system/bitnet.service");
    restore_root_template(
        Path::new("/usr/lib/bitnet/templates/bitnet.service"),
        target,
    )?;
    root_helper(target)
}

#[cfg(windows)]
async fn wait_windows_service(running: bool) -> Result<(), AppError> {
    let deadline = Instant::now() + Duration::from_secs(45);
    loop {
        if windows_target_reached(windows_service_state()?, running) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(err(
                ErrorCode::Unavailable,
                "The requested service state was not confirmed",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(any(windows, test))]
fn windows_target_reached(state: u32, running: bool) -> bool {
    // SERVICE_STOPPED=1 and SERVICE_RUNNING=4. Pending or paused states are
    // neither target, rather than being reduced to a misleading false boolean.
    state == if running { 4 } else { 1 }
}

#[cfg(windows)]
fn windows_service_state() -> Result<u32, AppError> {
    use windows_sys::Win32::System::Services::*;
    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not query the service manager",
        ));
    }
    let name = wide("BitNet");
    let service = unsafe { OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_STATUS) };
    unsafe { CloseServiceHandle(manager) };
    if service.is_null() {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not query the installed service",
        ));
    }
    let mut state: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
    let mut needed = 0;
    let queried = unsafe {
        QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            (&mut state as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of_val(&state) as u32,
            &mut needed,
        )
    };
    unsafe { CloseServiceHandle(service) };
    if queried == 0 {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not confirm the service state",
        ));
    }
    Ok(state.dwCurrentState)
}

#[cfg(windows)]
fn windows_service_disabled() -> Result<bool, AppError> {
    use windows_sys::Win32::System::Services::*;
    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not query the service manager",
        ));
    }
    let name = wide("BitNet");
    let service = unsafe { OpenServiceW(manager, name.as_ptr(), SERVICE_QUERY_CONFIG) };
    unsafe { CloseServiceHandle(manager) };
    if service.is_null() {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not open the system service",
        ));
    }
    let mut needed = 0;
    unsafe { QueryServiceConfigW(service, std::ptr::null_mut(), 0, &mut needed) };
    let mut buffer = vec![0u64; (needed as usize).div_ceil(8)];
    let queried = needed > 0
        && unsafe { QueryServiceConfigW(service, buffer.as_mut_ptr().cast(), needed, &mut needed) }
            != 0;
    unsafe { CloseServiceHandle(service) };
    if !queried {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not read the service startup configuration",
        ));
    }
    Ok(unsafe {
        (*buffer.as_ptr().cast::<QUERY_SERVICE_CONFIGW>()).dwStartType == SERVICE_DISABLED
    })
}

#[cfg(windows)]
async fn change_windows_service_state(running: bool) -> Result<(), AppError> {
    use windows_sys::Win32::System::Services::*;
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut requested = false;
    loop {
        let state = windows_service_state()?;
        if windows_target_reached(state, running) {
            return Ok(());
        }
        if !requested {
            let command = match (running, state) {
                (true, SERVICE_STOPPED) => Some("start"),
                (true, SERVICE_PAUSED) => Some("continue"),
                (false, SERVICE_RUNNING | SERVICE_PAUSED) => Some("stop"),
                _ => None,
            };
            if let Some(command) = command {
                run(system_plan("sc.exe", &[command, "BitNet"], 45)?).await?;
                requested = true;
            }
        }
        if Instant::now() >= deadline {
            return Err(err(
                ErrorCode::Unavailable,
                "The requested service transition was not confirmed",
            ));
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(windows)]
fn configure_windows_recovery() -> Result<(), AppError> {
    use windows_sys::Win32::System::Services::*;
    let manager = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), SC_MANAGER_CONNECT) };
    if manager.is_null() {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Could not configure service recovery",
        ));
    }
    let name = wide("BitNet");
    // SC_ACTION_RESTART also requires SERVICE_START on this handle.
    let service = unsafe {
        OpenServiceW(
            manager,
            name.as_ptr(),
            SERVICE_CHANGE_CONFIG | SERVICE_START,
        )
    };
    unsafe { CloseServiceHandle(manager) };
    if service.is_null() {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Could not open the installed service configuration",
        ));
    }
    let mut actions = [5000, 10000, 30000].map(|delay| SC_ACTION {
        Type: SC_ACTION_RESTART,
        Delay: delay,
    });
    let failure = SERVICE_FAILURE_ACTIONSW {
        dwResetPeriod: 86400,
        lpRebootMsg: std::ptr::null_mut(),
        lpCommand: std::ptr::null_mut(),
        cActions: actions.len() as u32,
        lpsaActions: actions.as_mut_ptr(),
    };
    let flag = SERVICE_FAILURE_ACTIONS_FLAG {
        fFailureActionsOnNonCrashFailures: 1,
    };
    let actions_ok = unsafe {
        ChangeServiceConfig2W(
            service,
            SERVICE_CONFIG_FAILURE_ACTIONS,
            (&failure as *const SERVICE_FAILURE_ACTIONSW).cast(),
        )
    };
    let flag_ok = actions_ok != 0
        && unsafe {
            ChangeServiceConfig2W(
                service,
                SERVICE_CONFIG_FAILURE_ACTIONS_FLAG,
                (&flag as *const SERVICE_FAILURE_ACTIONS_FLAG).cast(),
            )
        } != 0;
    unsafe { CloseServiceHandle(service) };
    if !flag_ok {
        return Err(err(
            ErrorCode::Unavailable,
            "The installer completed but service recovery could not be configured; run Repair",
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
async fn mac_start(restart: bool) -> Result<(), AppError> {
    root_helper(Path::new(
        "/Library/LaunchDaemons/top.nsapi.bitnet.keychain.plist",
    ))?;
    root_helper(Path::new("/Library/LaunchDaemons/top.nsapi.bitnet.plist"))?;
    // The root key broker is a demand job. Bootstrap its socket before the
    // restricted worker starts; launchd launches it only when the worker asks.
    if !mac_job_loaded("system/top.nsapi.bitnet.keychain").await? {
        run(plan(
            "/bin/launchctl",
            &[
                "bootstrap",
                "system",
                "/Library/LaunchDaemons/top.nsapi.bitnet.keychain.plist",
            ],
            30,
        ))
        .await?;
    }
    let loaded = mac_job_loaded("system/top.nsapi.bitnet").await?;
    for command in mac_start_plans(loaded, restart) {
        run(command).await?;
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn mac_start_plans(loaded: bool, restart: bool) -> Vec<Plan> {
    let mut commands = Vec::new();
    if !loaded {
        commands.push(plan(
            "/bin/launchctl",
            &[
                "bootstrap",
                "system",
                "/Library/LaunchDaemons/top.nsapi.bitnet.plist",
            ],
            30,
        ));
    }
    commands.push(plan(
        "/bin/launchctl",
        if restart {
            &["kickstart", "-k", "system/top.nsapi.bitnet"]
        } else {
            &["kickstart", "system/top.nsapi.bitnet"]
        },
        30,
    ));
    commands
}

#[cfg(target_os = "macos")]
async fn mac_stop() -> Result<(), AppError> {
    // kill restarts a KeepAlive job immediately; bootout unloads it until
    // the next explicit start or normal machine startup. A failed query is
    // never accepted as proof that the job stopped.
    mac_stop_job("system/top.nsapi.bitnet").await
}

#[cfg(target_os = "macos")]
async fn mac_job_loaded(label: &'static str) -> Result<bool, AppError> {
    mac_probe_result(run_exit_code(plan("/bin/launchctl", &["print", label], 30)).await?)
}

#[cfg(any(target_os = "macos", test))]
fn mac_probe_result(code: i32) -> Result<bool, AppError> {
    // launchd reports the precise service-not-found errors ESRCH(3) or
    // launchd's service lookup error 113. Permissions and unknown failures remain
    // errors rather than becoming a fabricated stopped/absent state.
    match code {
        0 => Ok(true),
        3 | 113 => Ok(false),
        _ => Err(err(
            ErrorCode::Unavailable,
            "Could not confirm the launchd job state",
        )),
    }
}

#[cfg(target_os = "macos")]
async fn mac_stop_job(label: &'static str) -> Result<(), AppError> {
    let code = run_exit_code(plan("/bin/launchctl", &["bootout", label], 30)).await?;
    if code == 0 || (matches!(code, 3 | 113) && !mac_job_loaded(label).await?) {
        return Ok(());
    }
    Err(err(
        ErrorCode::Unavailable,
        "Could not confirm launchd job removal",
    ))
}

#[cfg(target_os = "macos")]
fn provision_mac_plists() -> Result<(), AppError> {
    for filename in ["top.nsapi.bitnet.plist", "top.nsapi.bitnet.keychain.plist"] {
        let source = Path::new("/Library/PrivilegedHelperTools/BitNetTemplates").join(filename);
        let target = Path::new("/Library/LaunchDaemons").join(filename);
        restore_root_template(&source, &target)?;
    }
    Ok(())
}

#[cfg(unix)]
fn restore_root_template(source: &Path, target: &Path) -> Result<(), AppError> {
    root_helper(source)?;
    reject_links(target)?;
    let parent = target.parent().ok_or_else(|| {
        err(
            ErrorCode::PermissionDenied,
            "Service template parent is invalid",
        )
    })?;
    root_directory(parent)?;
    if target.exists() {
        root_helper(target)?;
    }
    let bytes = updates::read_small_file(source, 32 * 1024)?;
    let temporary = parent.join(format!(
        ".bitnet-template-{}-{}.new",
        std::process::id(),
        bitnet_core::now_ms()
    ));
    let mut file = updates::create_file(&temporary)?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not stage the installed service template",
            )
        })?;
    drop(file);
    if fs::rename(&temporary, target).is_err() {
        let _ = fs::remove_file(&temporary);
        return Err(err(
            ErrorCode::Unavailable,
            "Could not restore the installed service template",
        ));
    }
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not synchronize the service template directory",
            )
        })
}

struct Plan {
    program: PathBuf,
    args: Vec<OsString>,
    timeout: Duration,
}
fn plan(program: &str, args: &[&str], seconds: u64) -> Plan {
    Plan {
        program: program.into(),
        args: args.iter().map(OsString::from).collect(),
        timeout: Duration::from_secs(seconds),
    }
}
async fn run(plan: Plan) -> Result<bool, AppError> {
    match run_exit_code(plan).await? {
        0 => Ok(false),
        3010 | 1641 => Ok(true),
        _ => Err(err(
            ErrorCode::Unavailable,
            "OS maintenance failed; no successful installation is claimed",
        )),
    }
}

async fn run_exit_code(plan: Plan) -> Result<i32, AppError> {
    run_os_tool(plan, false).await.map(|(code, _)| code)
}

async fn run_os_tool(plan: Plan, capture: bool) -> Result<(i32, Vec<u8>), AppError> {
    tokio::task::spawn_blocking(move || {
        let mut command = Command::new(&plan.program);
        command
            .args(plan.args)
            .stdin(Stdio::null())
            .stdout(if capture {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
        }
        #[cfg(unix)]
        {
            command
                .env_clear()
                .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
        }
        let mut child = command.spawn().map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not start the fixed OS maintenance tool",
            )
        })?;
        let start = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|_| {
                err(
                    ErrorCode::Unavailable,
                    "Could not inspect maintenance process",
                )
            })? {
                let code = status.code().ok_or_else(|| {
                    err(
                        ErrorCode::Unavailable,
                        "OS maintenance terminated unexpectedly",
                    )
                })?;
                let mut output = Vec::new();
                if let Some(stdout) = child.stdout.take() {
                    stdout.take(129).read_to_end(&mut output).map_err(|_| {
                        err(
                            ErrorCode::Unavailable,
                            "Could not read OS maintenance result",
                        )
                    })?;
                    if output.len() > 128 {
                        return Err(err(
                            ErrorCode::Unavailable,
                            "Unexpected OS maintenance output",
                        ));
                    }
                }
                return Ok((code, output));
            }
            if start.elapsed() > plan.timeout {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err(ErrorCode::Unavailable, "OS maintenance timed out"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    })
    .await
    .map_err(|_| err(ErrorCode::Internal, "Maintenance worker failed"))?
}
fn package_plan(package: &Path, scope: Scope) -> Result<Plan, AppError> {
    reject_links(package)?;
    #[cfg(windows)]
    {
        if package.extension().and_then(OsStr::to_str) != Some("msi") {
            return Err(err(
                ErrorCode::InvalidInput,
                "Windows requires an MSI installer",
            ));
        }
        let mut p = system_plan("msiexec.exe", &["/i"], 900)?;
        p.args.push(package.as_os_str().into());
        p.args
            .extend(["/passive", "/norestart"].map(OsString::from));
        if scope == Scope::User {
            p.args.push("MSIINSTALLPERUSER=1".into());
        }
        Ok(p)
    }
    #[cfg(target_os = "linux")]
    {
        let _ = scope;
        if package.extension().and_then(OsStr::to_str) != Some("deb") {
            return Err(err(
                ErrorCode::InvalidInput,
                "Linux requires a Debian installer",
            ));
        }
        Ok(Plan {
            program: "/usr/bin/dpkg".into(),
            args: vec!["--install".into(), package.as_os_str().into()],
            timeout: Duration::from_secs(900),
        })
    }
    #[cfg(target_os = "macos")]
    {
        let _ = scope;
        if package.extension().and_then(OsStr::to_str) != Some("pkg") {
            return Err(err(
                ErrorCode::InvalidInput,
                "macOS requires a PKG installer",
            ));
        }
        Ok(Plan {
            program: "/usr/sbin/installer".into(),
            args: vec![
                "-pkg".into(),
                package.as_os_str().into(),
                "-target".into(),
                "/".into(),
            ],
            timeout: Duration::from_secs(900),
        })
    }
}

fn packages_plan(packages: &[PathBuf], scope: Scope) -> Result<Plan, AppError> {
    let first = packages
        .first()
        .ok_or_else(|| err(ErrorCode::InvalidInput, "No verified installer packages"))?;
    let base = package_plan(first, scope)?;
    #[cfg(target_os = "linux")]
    {
        if packages.len() > 2 {
            return Err(err(ErrorCode::InvalidInput, "Too many installer packages"));
        }
        let mut combined = base;
        for package in &packages[1..] {
            reject_links(package)?;
            if package.extension().and_then(OsStr::to_str) != Some("deb") {
                return Err(err(
                    ErrorCode::InvalidInput,
                    "Linux requires Debian installers",
                ));
            }
            combined.args.push(package.as_os_str().into());
        }
        Ok(combined)
    }
    #[cfg(not(target_os = "linux"))]
    {
        if packages.len() != 1 {
            return Err(err(
                ErrorCode::InvalidInput,
                "This platform requires one installer package",
            ));
        }
        Ok(base)
    }
}

fn rollback_packages_plan(packages: &[PathBuf], scope: Scope) -> Result<Plan, AppError> {
    let base = packages_plan(packages, scope)?;
    #[cfg(windows)]
    {
        let mut explicit = base;
        explicit.args.push("BITNET_ALLOW_DOWNGRADE=1".into());
        Ok(explicit)
    }
    #[cfg(not(windows))]
    {
        Ok(base)
    }
}
fn sibling(relative: &str) -> Result<PathBuf, AppError> {
    let exe = std::env::current_exe().map_err(|_| {
        err(
            ErrorCode::Unavailable,
            "Could not locate the installed executable",
        )
    })?;
    let path = exe
        .parent()
        .ok_or_else(|| {
            err(
                ErrorCode::Unavailable,
                "Installed executable has no directory",
            )
        })?
        .join(relative);
    reject_links(&path)?;
    Ok(path)
}
pub(super) fn reject_links(path: &Path) -> Result<(), AppError> {
    if !path.is_absolute() || path.components().any(|v| matches!(v, Component::ParentDir)) {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Maintenance paths must be absolute without parent traversal",
        ));
    }
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err(err(
                        ErrorCode::PermissionDenied,
                        "Maintenance refuses symlinks",
                    ));
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err(err(
                            ErrorCode::PermissionDenied,
                            "Maintenance refuses reparse points",
                        ));
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Could not inspect maintenance path",
                ))
            }
        }
    }
    Ok(())
}
fn copy_bounded(source: &Path, target: &Path) -> Result<(), AppError> {
    let mut input = updates::open_read(source)?;
    let mut output = updates::create_file(target)?;
    let mut count = 0;
    let mut buffer = [0; 64 * 1024];
    loop {
        let n = input
            .read(&mut buffer)
            .map_err(|_| err(ErrorCode::Unavailable, "Could not read offline installer"))?;
        if n == 0 {
            break;
        }
        count += n as u64;
        if count > updates::MAX_PACKAGE_BYTES {
            return Err(err(
                ErrorCode::InvalidInput,
                "Offline installer exceeds size limit",
            ));
        }
        output
            .write_all(&buffer[..n])
            .map_err(|_| err(ErrorCode::Unavailable, "Could not stage offline installer"))?;
    }
    output.sync_all().map_err(|_| {
        err(
            ErrorCode::Unavailable,
            "Could not synchronize staged installer",
        )
    })
}

struct Staging {
    directory: PathBuf,
    root: PathBuf,
    _lock: File,
}
impl Staging {
    fn new(scope: Scope) -> Result<Self, AppError> {
        let system = scope == Scope::System || cfg!(not(windows));
        let root = if system {
            #[cfg(windows)]
            {
                program_data()?.join("BitNetMaintenance")
            }
            #[cfg(target_os = "linux")]
            {
                PathBuf::from("/var/lib/bitnet-maintenance")
            }
            #[cfg(target_os = "macos")]
            {
                PathBuf::from("/Library/Application Support/BitNetMaintenance")
            }
        } else {
            bitnet_platform::paths(Scope::User)?
                .data_dir
                .join("maintenance")
        };
        secure_directory(&root, system)?;
        let lock_path = root.join("maintenance.lock");
        reject_links(&lock_path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x0020_0000);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        }
        let lock = options
            .open(lock_path)
            .map_err(|_| err(ErrorCode::Unavailable, "Could not open maintenance lock"))?;
        lock.try_lock_exclusive()
            .map_err(|_| err(ErrorCode::Busy, "Another maintenance operation is running"))?;
        for counter in 0..128 {
            let directory = root.join(format!(
                "stage-{}-{}-{counter}",
                std::process::id(),
                bitnet_core::now_ms()
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    return Ok(Self {
                        directory,
                        root,
                        _lock: lock,
                    })
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => {
                    return Err(err(
                        ErrorCode::Unavailable,
                        "Could not reserve protected staging",
                    ))
                }
            }
        }
        Err(err(
            ErrorCode::Busy,
            "Could not reserve a unique staging path",
        ))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        if let (Ok(root), Ok(directory)) = (self.root.canonicalize(), self.directory.canonicalize())
        {
            if directory.parent() == Some(root.as_path())
                && directory.starts_with(&root)
                && reject_links(&directory).is_ok()
            {
                let _ = fs::remove_dir_all(directory);
            }
        }
    }
}
fn secure_directory(path: &Path, system: bool) -> Result<(), AppError> {
    reject_links(path)?;
    if let Some(parent) = path.parent() {
        if !parent.is_dir() {
            if system {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Protected staging parent is missing",
                ));
            }
            fs::create_dir_all(parent).map_err(|_| {
                err(
                    ErrorCode::Unavailable,
                    "Could not create user maintenance directory",
                )
            })?;
        }
    }
    #[cfg(windows)]
    secure_windows_directory(path, system, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if !path.exists() {
            fs::create_dir(path).map_err(|_| {
                err(
                    ErrorCode::Unavailable,
                    "Could not create maintenance directory",
                )
            })?;
        }
        let m = fs::symlink_metadata(path).map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not inspect maintenance owner",
            )
        })?;
        if !m.is_dir()
            || m.uid()
                != if system {
                    0
                } else {
                    unsafe { libc::geteuid() }
                }
        {
            return Err(err(
                ErrorCode::PermissionDenied,
                "Maintenance directory has an unexpected owner",
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
            err(
                ErrorCode::PermissionDenied,
                "Could not restrict maintenance directory",
            )
        })?;
    }
    reject_links(path)
}

fn set_autostart(gui: bool, enabled: bool) -> Result<MaintenanceResult, AppError> {
    let label = if gui {
        "top.nsapi.bitnet.gui"
    } else {
        "top.nsapi.bitnet.engine"
    };
    let executable = sibling(match (gui, cfg!(windows)) {
        (true, true) => "bitnet-gui.exe",
        (false, true) => "bitengine.exe",
        (true, false) => "bitnet-gui",
        (false, false) => "bitengine",
    })?;
    if enabled && !executable.is_file() {
        return Err(err(
            ErrorCode::Unavailable,
            "The installed startup executable is missing",
        ));
    }
    #[cfg(windows)]
    {
        let _ = label;
        windows_autostart(
            if gui { "BitNetGUI" } else { "BitNetEngine" },
            &executable,
            gui,
            enabled,
        )?;
    }
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| err(ErrorCode::Unavailable, "User home is unavailable"))?;
        let directory = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("autostart");
        reject_links(&directory)?;
        fs::create_dir_all(&directory).map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not create XDG autostart directory",
            )
        })?;
        let path = directory.join(format!("{label}.desktop"));
        if enabled {
            let value = executable
                .to_string_lossy()
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('`', "\\`")
                .replace('$', "\\$")
                .replace('%', "%%");
            let args = if gui { " --autostart" } else { "" };
            write_startup(&path,format!("[Desktop Entry]\nType=Application\nName=BitNet\nExec=\"{value}\"{args}\nTerminal=false\n").as_bytes())?;
        } else {
            remove_file(&path)?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| err(ErrorCode::Unavailable, "User home is unavailable"))?;
        let directory = home.join("Library/LaunchAgents");
        reject_links(&directory)?;
        fs::create_dir_all(&directory).map_err(|_| {
            err(
                ErrorCode::Unavailable,
                "Could not create LaunchAgents directory",
            )
        })?;
        let path = directory.join(format!("{label}.plist"));
        if enabled {
            let value = executable
                .to_string_lossy()
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;")
                .replace('"', "&quot;")
                .replace('\'', "&apos;");
            let args = if gui {
                "<string>--autostart</string>"
            } else {
                ""
            };
            write_startup(&path,format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array><string>{value}</string>{args}</array><key>RunAtLoad</key><true/></dict></plist>").as_bytes())?;
        } else {
            remove_file(&path)?;
        }
    }
    Ok(result(if enabled {
        "User login startup enabled"
    } else {
        "User login startup disabled"
    }))
}
#[cfg(unix)]
fn write_startup(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    reject_links(path)?;
    let temp = path.with_extension(format!("pending-{}", bitnet_core::now_ms()));
    let mut file = updates::create_file(&temp)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| err(ErrorCode::Unavailable, "Could not write startup entry"))?;
    drop(file);
    fs::rename(temp, path)
        .map_err(|_| err(ErrorCode::Unavailable, "Could not commit startup entry"))
}
#[cfg(unix)]
fn remove_file(path: &Path) -> Result<(), AppError> {
    if !path.exists() {
        return Ok(());
    }
    reject_links(path)?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(err(
            ErrorCode::Unavailable,
            "Could not remove fixed startup/service file",
        )),
    }
}
fn purge_system_data() -> Result<(), AppError> {
    #[cfg(windows)]
    let path = program_data()?.join("BitNet");
    #[cfg(target_os = "linux")]
    let path = PathBuf::from("/var/lib/bitnet");
    #[cfg(target_os = "macos")]
    let path = PathBuf::from("/Library/Application Support/BitNet");
    reject_links(&path)?;
    if !path.exists() {
        return Ok(());
    }
    let resolved = path
        .canonicalize()
        .map_err(|_| err(ErrorCode::Unavailable, "Could not resolve service data"))?;
    let parent = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| {
            err(
                ErrorCode::PermissionDenied,
                "Service data parent is invalid",
            )
        })?;
    if resolved.parent() != Some(parent.as_path()) || resolved == parent {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Refusing purge outside the fixed service data directory",
        ));
    }
    fs::remove_dir_all(resolved)
        .map_err(|_| err(ErrorCode::Unavailable, "Could not purge service data"))
}

#[cfg(windows)]
fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
#[cfg(windows)]
fn system_plan(program: &str, args: &[&str], seconds: u64) -> Result<Plan, AppError> {
    use windows_sys::Win32::System::SystemInformation::GetSystemDirectoryW;
    let mut value = [0; 32768];
    let n = unsafe { GetSystemDirectoryW(value.as_mut_ptr(), value.len() as u32) } as usize;
    if n == 0 || n >= value.len() {
        return Err(err(
            ErrorCode::Unavailable,
            "Windows system directory is unavailable",
        ));
    }
    use std::os::windows::ffi::OsStringExt;
    let mut p = plan("", args, seconds);
    p.program = PathBuf::from(OsString::from_wide(&value[..n])).join(program);
    Ok(p)
}

#[cfg(windows)]
fn program_data() -> Result<PathBuf, AppError> {
    use windows_sys::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_ProgramData, SHGetKnownFolderPath},
    };
    let mut pointer = std::ptr::null_mut();
    if unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramData, 0, std::ptr::null_mut(), &mut pointer) }
        < 0
        || pointer.is_null()
    {
        return Err(err(
            ErrorCode::Unavailable,
            "Windows ProgramData is unavailable",
        ));
    }
    let mut n = 0;
    while n < 32768 && unsafe { *pointer.add(n) } != 0 {
        n += 1;
    }
    use std::os::windows::ffi::OsStringExt;
    let p = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(pointer, n)
    }));
    unsafe { CoTaskMemFree(pointer.cast()) };
    reject_links(&p)?;
    Ok(p)
}
#[cfg(windows)]
fn secure_windows_directory(
    path: &Path,
    system: bool,
    public_metadata: bool,
) -> Result<(), AppError> {
    use windows_sys::Win32::{
        Foundation::{GetLastError, LocalFree, ERROR_ALREADY_EXISTS},
        Security::{Authorization::*, *},
        Storage::FileSystem::CreateDirectoryW,
    };
    let p = wide(path);
    let mut descriptor = std::ptr::null_mut();
    let sddl = if public_metadata {
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FRFX;;;BU)"
    } else if system {
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"
    } else {
        "D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;OW)"
    };
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            wide(sddl).as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Could not construct staging permissions",
        ));
    }
    let attributes = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let made = unsafe { CreateDirectoryW(p.as_ptr(), &attributes) };
    let last = unsafe { GetLastError() };
    if made == 0 && last != ERROR_ALREADY_EXISTS {
        unsafe { LocalFree(descriptor) };
        return Err(err(
            ErrorCode::Unavailable,
            "Could not create staging directory",
        ));
    }
    reject_links(path)?;
    let mut owner = std::ptr::null_mut();
    let mut existing = std::ptr::null_mut();
    let status = unsafe {
        GetNamedSecurityInfoW(
            p.as_ptr(),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut existing,
        )
    };
    let trusted = status == 0
        && (!system
            || unsafe {
                IsWellKnownSid(owner, WinLocalSystemSid) != 0
                    || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
            });
    if !existing.is_null() {
        unsafe { LocalFree(existing) };
    }
    if !trusted {
        unsafe { LocalFree(descriptor) };
        return Err(err(
            ErrorCode::PermissionDenied,
            "Staging directory has an untrusted owner",
        ));
    }
    let mut present = 0;
    let mut acl = std::ptr::null_mut();
    let mut defaulted = 0;
    let ok =
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) };
    let status = if ok != 0 && present != 0 {
        unsafe {
            SetNamedSecurityInfoW(
                p.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                acl,
                std::ptr::null_mut(),
            )
        }
    } else {
        1
    };
    unsafe { LocalFree(descriptor) };
    if status != 0 {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Could not protect staging directory",
        ));
    }
    Ok(())
}
#[cfg(windows)]
fn service_product() -> Result<String, AppError> {
    use windows_sys::Win32::System::ApplicationInstallationAndServicing::MsiEnumRelatedProductsW;
    let mut guid = [0; 39];
    if unsafe {
        MsiEnumRelatedProductsW(
            wide("{7FBA8135-7341-4F91-A39E-615190032EA2}").as_ptr(),
            0,
            0,
            guid.as_mut_ptr(),
        )
    } != 0
    {
        return Err(err(
            ErrorCode::Unavailable,
            "The machine service MSI is not installed or cannot be queried",
        ));
    }
    Ok(String::from_utf16_lossy(&guid[..38]))
}
#[cfg(windows)]
fn windows_autostart(
    name: &str,
    executable: &Path,
    gui: bool,
    enabled: bool,
) -> Result<(), AppError> {
    use windows_sys::Win32::System::Registry::*;
    let mut key = std::ptr::null_mut();
    if unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run").as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_SET_VALUE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    } != 0
    {
        return Err(err(
            ErrorCode::PermissionDenied,
            "Could not open current-user startup settings",
        ));
    }
    let status = if enabled {
        let args = if gui { " --autostart" } else { "" };
        let value = wide(format!("\"{}\"{args}", executable.to_string_lossy()));
        unsafe {
            RegSetValueExW(
                key,
                wide(name).as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        }
    } else {
        unsafe { RegDeleteValueW(key, wide(name).as_ptr()) }
    };
    unsafe { RegCloseKey(key) };
    if status != 0 && (status != 2 || enabled) {
        return Err(err(
            ErrorCode::Unavailable,
            "Could not update current-user startup settings",
        ));
    }
    Ok(())
}

async fn elevate(scope: Scope, action: MaintenanceAction) -> Result<MaintenanceResult, AppError> {
    #[cfg(windows)]
    {
        let helper = sibling("bitmaint.exe")?;
        if !helper.is_file() {
            return Err(err(
                ErrorCode::Unavailable,
                "The installed maintenance helper is missing",
            ));
        }
        let mut arguments = format!(
            "--scope {} --action {} --elevated{}",
            if scope == Scope::System {
                "system"
            } else {
                "user"
            },
            action_name(&action),
            if matches!(action, MaintenanceAction::ServiceRemove { purge: true }) {
                " --purge"
            } else {
                ""
            }
        );
        if let Some(interface) = action.ipv6_target() {
            use std::fmt::Write as _;
            let _ = write!(
                arguments,
                " --interface {}@{}",
                interface.guid, interface.index
            );
        }
        if let MaintenanceAction::InstallCheckedUpdate { manifest_sha256 } = &action {
            use std::fmt::Write as _;
            let _ = write!(arguments, " --manifest-sha256 {}", manifest_sha256.0);
        }
        tokio::task::spawn_blocking(move || {
            use windows_sys::Win32::{
                Foundation::*,
                System::Threading::*,
                UI::{Shell::*, WindowsAndMessaging::SW_HIDE},
            };
            let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
            info.cbSize = std::mem::size_of_val(&info) as u32;
            info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
            let verb = wide("runas");
            let program = wide(helper);
            let args = wide(arguments);
            info.lpVerb = verb.as_ptr();
            info.lpFile = program.as_ptr();
            info.lpParameters = args.as_ptr();
            info.nShow = SW_HIDE;
            if unsafe { ShellExecuteExW(&mut info) } == 0 {
                return Err(err(
                    if unsafe { GetLastError() } == ERROR_CANCELLED {
                        ErrorCode::Cancelled
                    } else {
                        ErrorCode::PermissionDenied
                    },
                    "Administrator authorization was cancelled or unavailable",
                ));
            }
            if info.hProcess.is_null() {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Elevated maintenance has no process handle",
                ));
            }
            let waited = unsafe { WaitForSingleObject(info.hProcess, 900000) };
            let mut code = 1;
            let ok = unsafe { GetExitCodeProcess(info.hProcess, &mut code) };
            unsafe { CloseHandle(info.hProcess) };
            if waited != WAIT_OBJECT_0 || ok == 0 {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Elevated maintenance did not finish in time",
                ));
            }
            if action.is_ipv6_maintenance() {
                return ipv6::elevated_result(&action, code);
            }
            if code != 0 && code != 3010 {
                return Err(err(ErrorCode::Unavailable, "Elevated maintenance failed"));
            }
            Ok(MaintenanceResult {
                message: "Authorized maintenance completed".into(),
                details: json!({"reboot_required":code==3010}),
            })
        })
        .await
        .map_err(|_| err(ErrorCode::Internal, "Authorization worker failed"))?
    }
    #[cfg(target_os = "linux")]
    {
        let helper = Path::new("/usr/libexec/bitnet/bitmaint");
        root_helper(helper)?;
        let mut p = plan(
            "/usr/bin/pkexec",
            &[
                "/usr/libexec/bitnet/bitmaint",
                "--scope",
                if scope == Scope::System {
                    "system"
                } else {
                    "user"
                },
                "--action",
                action_name(&action),
                "--elevated",
            ],
            900,
        );
        if matches!(action, MaintenanceAction::ServiceRemove { purge: true }) {
            p.args.push("--purge".into());
        }
        if let MaintenanceAction::InstallCheckedUpdate { manifest_sha256 } = &action {
            p.args
                .extend(["--manifest-sha256".into(), manifest_sha256.0.clone().into()]);
        }
        run(p).await?;
        Ok(result("Authorized maintenance completed"))
    }
    #[cfg(target_os = "macos")]
    {
        const HELPER: &str = "/Library/PrivilegedHelperTools/top.nsapi.bitnet.maint";
        if !Path::new(HELPER).try_exists().map_err(|_| {
            err(
                ErrorCode::PermissionDenied,
                "Could not inspect the installed maintenance helper",
            )
        })? {
            if !matches!(action, MaintenanceAction::ServiceInstall) {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Install the verified macOS service package before system maintenance",
                ));
            }
            let package = mac_bundled_package()?;
            updates::verify_offline_package(&package)?;
            run(Plan {
                program: "/usr/bin/open".into(),
                args: vec![
                    "-a".into(),
                    "/System/Library/CoreServices/Installer.app".into(),
                    package.as_os_str().into(),
                ],
                timeout: Duration::from_secs(30),
            })
            .await?;
            return Ok(MaintenanceResult {
                message: "Verified service package opened in Installer; complete installation and refresh service status".into(),
                details: json!({"installer_opened":true,"installation_confirmed":false}),
            });
        }
        root_helper(Path::new(HELPER))?;
        let extra = match &action {
            MaintenanceAction::InstallCheckedUpdate { manifest_sha256 } => {
                format!(" --manifest-sha256 {}", manifest_sha256.0)
            }
            MaintenanceAction::ServiceRemove { purge: true } => " --purge".to_owned(),
            _ => String::new(),
        };
        let script = format!("do shell script \"{HELPER} --scope {} --action {} --elevated{extra}\" with administrator privileges", if scope == Scope::System {"system"} else {"user"}, action_name(&action));
        run(Plan {
            program: "/usr/bin/osascript".into(),
            args: vec!["-e".into(), script.into()],
            timeout: Duration::from_secs(900),
        })
        .await?;
        Ok(result("Authorized maintenance completed"))
    }
}

#[cfg(target_os = "macos")]
fn mac_bundled_package() -> Result<PathBuf, AppError> {
    let current = std::env::current_exe()
        .map_err(|_| err(ErrorCode::Unavailable, "Could not locate the installed app"))?;
    mac_package_from_executable(&current)
}

#[cfg(any(target_os = "macos", test))]
fn mac_package_from_executable(current: &Path) -> Result<PathBuf, AppError> {
    let macos = current
        .parent()
        .filter(|p| p.file_name() == Some(OsStr::new("MacOS")));
    let contents = macos
        .and_then(Path::parent)
        .filter(|p| p.file_name() == Some(OsStr::new("Contents")))
        .ok_or_else(|| {
            err(
                ErrorCode::Unavailable,
                "Install the macOS service package from the verified distribution",
            )
        })?;
    let package = contents.join("Resources/packages/Service.pkg");
    reject_links(&package)?;
    Ok(package)
}
#[cfg(unix)]
fn root_helper(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::MetadataExt;
    reject_links(path)?;
    let m = fs::metadata(path).map_err(|_| {
        err(
            ErrorCode::Unavailable,
            "Install the platform package to provision the authorized helper",
        )
    })?;
    if !m.is_file() || m.uid() != 0 || m.mode() & 0o022 != 0 {
        return Err(err(
            ErrorCode::PermissionDenied,
            "The maintenance helper is not root-controlled",
        ));
    }
    root_directory(
        path.parent()
            .ok_or_else(|| err(ErrorCode::PermissionDenied, "Missing helper parent"))?,
    )
}

#[cfg(unix)]
fn root_directory(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::MetadataExt;
    reject_links(path)?;
    for ancestor in path.ancestors() {
        let metadata = fs::metadata(ancestor).map_err(|_| {
            err(
                ErrorCode::PermissionDenied,
                "Could not verify helper parents",
            )
        })?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
            return Err(err(
                ErrorCode::PermissionDenied,
                "The maintenance helper parent is not root-controlled",
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    pub(super) struct Directory(pub PathBuf);
    pub(super) fn directory() -> Directory {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/maintenance-tests");
        fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join(format!(
            "fixture-{}-{}-{}",
            std::process::id(),
            bitnet_core::now_ms(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Directory(path)
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    impl Drop for Directory {
        fn drop(&mut self) {
            let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/maintenance-tests")
                .canonicalize()
                .unwrap();
            if let Ok(resolved) = self.0.canonicalize() {
                if resolved.parent() == Some(allowed.as_path()) && resolved.starts_with(&allowed) {
                    let _ = fs::remove_dir_all(resolved);
                }
            }
        }
    }
    #[test]
    fn confirmed_update_requires_the_same_signed_manifest_bytes() {
        let payload = br#"{"version":"1.0.0","assets":[]}"#;
        let digest = UpdateManifestId::for_payload(payload);
        let elevated = UpdateManifestId::parse(&digest.0.to_uppercase()).unwrap();
        assert_eq!(digest, elevated);
        assert!(elevated.verify(payload).is_ok());
        assert_eq!(
            elevated
                .verify(br#"{"version":"1.0.1","assets":[]}"#)
                .unwrap_err()
                .code,
            ErrorCode::Busy
        );
    }

    #[test]
    fn closed_actions_reject_command_injection() {
        assert!(parse_action("service-start;whoami", false).is_err());
        assert!(parse_action("service-start", true).is_err());
        for name in [
            "service-install",
            "service-start",
            "service-stop",
            "repair",
            "check-update",
            "rollback-update",
            "autostart-on",
            "gui-autostart-off",
        ] {
            assert_eq!(action_name(&parse_action(name, false).unwrap()), name);
        }
    }
    #[test]
    fn startup_modes_do_not_require_system_privileges() {
        assert!(!requires_admin(
            Scope::User,
            &MaintenanceAction::SetAutostart { enabled: true }
        ));
        assert!(!requires_admin(
            Scope::User,
            &MaintenanceAction::SetGuiAutostart { enabled: true }
        ));
        assert!(requires_admin(
            Scope::System,
            &MaintenanceAction::ServiceInstall
        ));
        assert!(requires_admin(
            Scope::System,
            &MaintenanceAction::RollbackUpdate
        ));
        assert_eq!(
            requires_admin(Scope::User, &MaintenanceAction::RollbackUpdate),
            !cfg!(windows)
        );
    }
    #[test]
    fn service_pending_states_are_never_mistaken_for_a_confirmed_stop() {
        assert!(windows_target_reached(1, false));
        assert!(windows_target_reached(4, true));
        for pending_or_paused in [2, 3, 5, 6, 7] {
            assert!(!windows_target_reached(pending_or_paused, false));
            assert!(!windows_target_reached(pending_or_paused, true));
        }
    }
    #[cfg(windows)]
    #[test]
    fn only_explicit_rollback_adds_the_installer_downgrade_capability() {
        let directory = directory();
        let package = directory.0.join("Client.msi");
        fs::write(&package, b"synthetic package, never executed").unwrap();
        let normal = packages_plan(std::slice::from_ref(&package), Scope::User).unwrap();
        assert!(!normal.args.iter().any(|a| a == "BITNET_ALLOW_DOWNGRADE=1"));
        let rollback = rollback_packages_plan(&[package], Scope::User).unwrap();
        assert!(rollback
            .args
            .iter()
            .any(|a| a == "BITNET_ALLOW_DOWNGRADE=1"));
    }
    #[test]
    fn mac_restart_bootstraps_unloaded_jobs_before_kickstart() {
        assert!(mac_probe_result(0).unwrap());
        assert!(!mac_probe_result(3).unwrap());
        assert!(!mac_probe_result(113).unwrap());
        for failure in [1, 2, 5, 13, 127] {
            assert!(mac_probe_result(failure).is_err());
        }
        let commands = mac_start_plans(false, true);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0].program, PathBuf::from("/bin/launchctl"));
        assert_eq!(commands[0].args[0], "bootstrap");
        assert_eq!(
            commands[1].args,
            ["kickstart", "-k", "system/top.nsapi.bitnet"].map(OsString::from)
        );
        let commands = mac_start_plans(true, false);
        assert_eq!(commands.len(), 1);
        assert_eq!(
            commands[0].args,
            ["kickstart", "system/top.nsapi.bitnet"].map(OsString::from)
        );
    }
    #[test]
    fn mac_initial_package_comes_only_from_the_fixed_bundle_resource() {
        let directory = directory();
        let binary = directory.0.join("BitNet.app/Contents/MacOS/bitnet-gui");
        assert_eq!(
            mac_package_from_executable(&binary).unwrap(),
            directory
                .0
                .join("BitNet.app/Contents/Resources/packages/Service.pkg")
        );
        assert!(mac_package_from_executable(&directory.0.join("bitmaint")).is_err());
    }
    #[tokio::test]
    async fn handoff_uses_live_locks_and_restores_only_without_a_system_service() {
        let directory = directory();
        let path = directory.0.join("maintenance.handoff.lock");
        assert!(!lock_in_progress(&path));
        let lock = open_handoff_lock(&path, true).unwrap();
        lock.try_lock_exclusive().unwrap();
        assert!(lock_in_progress(&path));
        let waiter = tokio::spawn({
            let path = path.clone();
            async move { wait_controller_exit(&path).await }
        });
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(!waiter.is_finished());
        FileExt::unlock(&lock).unwrap();
        waiter.await.unwrap().unwrap();
        assert!(!lock_in_progress(&path));
        let mut status = bitnet_core::ServiceStatus {
            installed: false,
            running: false,
            enabled: false,
            detail: None,
        };
        assert!(restore_personal_controller(true, &status));
        assert!(!restore_personal_controller(false, &status));
        status.installed = true;
        assert!(!restore_personal_controller(true, &status));
        status.installed = false;
        status.running = true;
        assert!(!restore_personal_controller(true, &status));
    }
    #[test]
    fn unsafe_paths_fail_before_mutation() {
        assert!(reject_links(Path::new("relative/Service.msi")).is_err());
        assert!(reject_links(&std::env::current_dir().unwrap().join("../Service.msi")).is_err());
    }
    #[tokio::test]
    async fn unconfigured_updates_do_not_claim_success() {
        if option_env!("BITNET_UPDATE_FEED").is_none()
            || option_env!("BITNET_RELEASE_PUBLIC_KEY").is_none()
        {
            assert!(execute(Scope::User, MaintenanceAction::CheckUpdate, false)
                .await
                .is_err());
        }
    }
    #[test]
    fn staging_copy_is_bounded_and_rejects_existing_destinations() {
        let directory = directory();
        let source = directory.0.join("source");
        let target = directory.0.join("target");
        fs::write(&source, b"synthetic fixture").unwrap();
        copy_bounded(&source, &target).unwrap();
        assert_eq!(
            updates::hash_file(&source).unwrap(),
            updates::hash_file(&target).unwrap()
        );
        assert!(copy_bounded(&source, &target).is_err());
    }
    #[test]
    fn corrupt_installer_metadata_never_runs_an_installer() {
        let directory = directory();
        let source = directory.0.join("Service.msi");
        fs::write(&source, b"not an installer").unwrap();
        assert!(updates::verify_offline_package(&source).is_err());
    }
}
