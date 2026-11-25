use crate::{
    diagnostics, maintenance, preview_only, App, AppError, Client, Command, Confirmation,
    ErrorCode, InterfaceTarget, MaintenanceAction, MaintenanceResult, Message, Scope, Settings,
    Snapshot, Task,
};
use bitnet_core::{HealthStatus, MAX_IPV6_TARGETS};
use diagnostics::{Ipv6RepairKind, Report};
use maintenance::Ipv6StackResetStatus;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::time::{sleep, timeout_at, Instant};

const RECHECK_WINDOW: Duration = Duration::from_secs(30);
const REPORT_LIFETIME: Duration = Duration::from_secs(60);
const RECHECK_FILE: &str = "ipv6-recheck.json";
const RECHECK_SCHEMA: u32 = 1;
/// A restart fix that nobody rechecked for a week no longer describes this boot.
const RECHECK_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const RECHECK_MAX_BYTES: usize = MAX_IPV6_TARGETS * 2048 + 512;

fn now_ms() -> u64 {
    bitnet_core::now_ms() as u64
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProbeSettings {
    ipv6_targets: Vec<String>,
    timeout_secs: u64,
}

/// Targets that a guided stack reset saved for the recheck after Windows restarts.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct RecheckRecord {
    schema: u32,
    /// Identifier of the protected reset transaction that saved these targets.
    reset_id: String,
    written_ms: u64,
    settings: ProbeSettings,
}

/// Why a saved recheck target must not be reused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecheckRejection {
    OtherReset,
    UnsupportedSchema,
    Expired,
    Damaged,
    InvalidSettings,
}

enum StoredSettings {
    Restored(ProbeSettings),
    Absent,
    Rejected(RecheckRejection),
}

impl From<&Settings> for ProbeSettings {
    fn from(settings: &Settings) -> Self {
        Self {
            ipv6_targets: settings.ipv6_targets.clone(),
            timeout_secs: settings.request_timeout_secs,
        }
    }
}

impl ProbeSettings {
    fn settings(&self) -> Settings {
        let mut settings = Settings {
            request_timeout_secs: self.timeout_secs,
            ..Settings::default()
        };
        settings.ipv6_targets.clone_from(&self.ipv6_targets);
        settings
    }

    fn recheck_path(data_dir: &Path) -> PathBuf {
        data_dir.join(RECHECK_FILE)
    }

    fn save_for_reboot(&self, reset_id: &str) -> Result<(), AppError> {
        let paths = bitnet_platform::paths(Scope::User)?;
        self.save_at(&paths.data_dir, reset_id)
    }

    fn save_at(&self, data_dir: &Path, reset_id: &str) -> Result<(), AppError> {
        // Only the protected reset may name the transaction a recheck belongs to.
        if !maintenance::is_reset_id(reset_id) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "无法把复测目标绑定到本次 IPv6 重置",
            ));
        }
        let record = RecheckRecord {
            schema: RECHECK_SCHEMA,
            reset_id: reset_id.to_owned(),
            written_ms: now_ms(),
            settings: self.clone(),
        };
        let data = serde_json::to_vec(&record)
            .map_err(|_| AppError::new(ErrorCode::Internal, "无法记录重启后的检测目标"))?;
        if data.len() > RECHECK_MAX_BYTES {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "重启后的检测目标过大",
            ));
        }
        // A brand-new profile has no data directory yet, and the atomic writer
        // refuses to create one implicitly.
        bitnet_platform::prepare_private_directory(data_dir)?;
        bitnet_platform::write_private_file(&Self::recheck_path(data_dir), &data)
    }

    fn for_reboot(reset_id: &str) -> StoredSettings {
        match bitnet_platform::paths(Scope::User) {
            Ok(paths) => Self::restore_at(&paths.data_dir, reset_id),
            Err(_) => StoredSettings::Absent,
        }
    }

    fn restore_at(data_dir: &Path, reset_id: &str) -> StoredSettings {
        let path = Self::recheck_path(data_dir);
        let bytes = match bitnet_platform::read_private_file(&path) {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return StoredSettings::Absent,
            Err(_) => return Self::reject(&path, RecheckRejection::Damaged),
        };
        let record = if bytes.len() <= RECHECK_MAX_BYTES {
            serde_json::from_slice::<RecheckRecord>(&bytes).ok()
        } else {
            None
        };
        let Some(record) = record else {
            return Self::reject(&path, RecheckRejection::Damaged);
        };
        if record.schema != RECHECK_SCHEMA {
            return Self::reject(&path, RecheckRejection::UnsupportedSchema);
        }
        if record.reset_id != reset_id {
            return Self::reject(&path, RecheckRejection::OtherReset);
        }
        if now_ms().saturating_sub(record.written_ms) > RECHECK_MAX_AGE.as_millis() as u64 {
            return Self::reject(&path, RecheckRejection::Expired);
        }
        if record.settings.settings().validate().is_err() {
            return Self::reject(&path, RecheckRejection::InvalidSettings);
        }
        StoredSettings::Restored(record.settings)
    }

    /// A record that cannot be tied to the current reset is discarded instead of
    /// silently probing the targets of an older transaction.
    fn reject(path: &Path, reason: RecheckRejection) -> StoredSettings {
        let _ = std::fs::remove_file(path);
        StoredSettings::Rejected(reason)
    }

    fn discard_saved() {
        if let Ok(paths) = bitnet_platform::paths(Scope::User) {
            let _ = std::fs::remove_file(Self::recheck_path(&paths.data_dir));
        }
    }
}

#[derive(Clone)]
struct ProbeContext {
    settings: ProbeSettings,
    snapshot: Option<Snapshot>,
    english: bool,
    simulation: bool,
}

impl ProbeContext {
    async fn probe(&self) -> HealthStatus {
        if let Some(before) = self.snapshot.as_ref().filter(|snapshot| {
            snapshot.capabilities.iter().any(|cap| cap == "probe_ipv6")
                && ProbeSettings::from(&snapshot.settings) == self.settings
        }) {
            let client = Client::exact(before.scope);
            let result = client.command(Command::ProbeIpv6).await;
            if let Ok(after) = result.and_then(|reply| reply.snapshot()) {
                if after.scope == before.scope
                    && ProbeSettings::from(&after.settings) == self.settings
                    && after.health.ipv6_checked_at.is_some()
                {
                    return after.health;
                }
            }
        }
        diagnostics::probe_ipv6_locally(&self.settings.settings(), self.simulation).await
    }

    async fn report(self, selected: Option<InterfaceTarget>, retry: bool) -> Report {
        let health = if retry {
            let deadline = Instant::now() + RECHECK_WINDOW;
            let mut health = HealthStatus::default();
            while let Ok(check) = timeout_at(deadline, self.probe()).await {
                let recovered = check.confirmed_ipv6().is_some();
                health = check;
                if recovered || Instant::now() + Duration::from_secs(2) >= deadline {
                    break;
                }
                sleep(Duration::from_secs(2)).await;
            }
            health
        } else {
            self.probe().await
        };
        let snapshot = match self.snapshot {
            Some(mut snapshot) => {
                snapshot.settings.ipv6_targets = self.settings.ipv6_targets.clone();
                snapshot.settings.request_timeout_secs = self.settings.timeout_secs;
                Some(snapshot)
            }
            None if self.settings != ProbeSettings::from(&Settings::default()) => {
                let mut snapshot = Snapshot::new(Scope::User);
                snapshot.settings = self.settings.settings();
                Some(snapshot)
            }
            None => None,
        };
        diagnostics::collect_with_ipv6(
            snapshot,
            self.english,
            self.simulation,
            Some(health),
            selected,
        )
        .await
    }
}

#[derive(Clone)]
pub(crate) struct Ipv6RepairRequest {
    run: u64,
    pub(crate) action: MaintenanceAction,
    pub(crate) adapter_name: Option<String>,
}

impl Ipv6RepairRequest {
    pub(crate) fn requires_advanced(&self) -> bool {
        matches!(self.action, MaintenanceAction::ResetIpv6Stack)
    }
}

#[derive(Clone)]
pub(crate) enum RepairResult {
    Applying,
    Rechecking,
    Recovered,
    StillUnavailable,
    Cancelled,
    Failed(AppError),
    RebootRequired(PathBuf, bool),
    AfterReboot {
        recovered: bool,
        interrupted: bool,
        backup_dir: PathBuf,
    },
}

#[derive(Default)]
pub(crate) struct Ipv6Repair {
    run: u64,
    context: Option<ProbeContext>,
    finished_at: Option<Instant>,
    active: Option<Ipv6RepairRequest>,
    attempted: Option<MaintenanceAction>,
    last_target: Option<InterfaceTarget>,
    before: Option<String>,
    boot_status: Option<Ipv6StackResetStatus>,
    acknowledged: Option<bool>,
    pub(crate) show_evidence: bool,
    pub(crate) result: Option<RepairResult>,
}

impl Ipv6Repair {
    pub(crate) fn invalidate(&mut self) {
        self.run = self.run.wrapping_add(1);
        self.context = None;
        self.finished_at = None;
        self.last_target = None;
        self.attempted = None;
        self.acknowledged = None;
        self.show_evidence = false;
    }

    pub(crate) fn before(&self) -> Option<&str> {
        self.before.as_deref()
    }

    /// Whether the protected reset record was acknowledged for the last
    /// recheck after a restart. `None` until such a recheck finishes. The
    /// repair view renders it once the frontend merge lands.
    #[allow(dead_code)]
    pub(crate) fn acknowledged(&self) -> Option<bool> {
        self.acknowledged
    }
}

impl App {
    fn new_probe_context(&self) -> ProbeContext {
        let snapshot = self.snapshot.clone().filter(|_| self.snapshot_current);
        let settings = snapshot.as_ref().map_or_else(
            || ProbeSettings::from(&Settings::default()),
            |snapshot| ProbeSettings::from(&snapshot.settings),
        );
        ProbeContext {
            settings,
            snapshot,
            english: self.english(),
            simulation: preview_only(),
        }
    }

    pub(crate) fn can_run_diagnostics(&self) -> bool {
        !self.busy && !self.diagnostics_running && (cfg!(windows) || self.snapshot_current)
    }

    pub(crate) fn ipv6_health_for_view<'a>(&'a self, snapshot: &'a Snapshot) -> &'a HealthStatus {
        let report = self.diagnostics.as_ref().filter(|report| {
            report.ipv6.checked_at > snapshot.health.ipv6_checked_at
                && self.ipv6_repair.context.as_ref().is_some_and(|context| {
                    context.snapshot.as_ref().is_some_and(|source| {
                        source.scope == snapshot.scope
                            && source.network_generation == snapshot.network_generation
                    }) && context.settings == ProbeSettings::from(&snapshot.settings)
                })
        });
        report.map_or(&snapshot.health, |report| &report.ipv6.health)
    }

    pub(crate) fn run_diagnostics(&mut self, selected: Option<InterfaceTarget>) -> Task<Message> {
        if !self.can_run_diagnostics() {
            return Task::none();
        }
        let context = self.new_probe_context();
        let selected = selected.or_else(|| self.ipv6_repair.last_target.clone());
        self.ipv6_repair.run = self.ipv6_repair.run.wrapping_add(1);
        self.ipv6_repair.finished_at = None;
        self.ipv6_repair.context = Some(context.clone());
        self.ipv6_repair.last_target.clone_from(&selected);
        self.diagnostics = None;
        self.diagnostics_running = true;
        let run = self.ipv6_repair.run;
        Task::perform(context.report(selected, false), move |report| {
            Message::DiagnosticsReady(run, Arc::new(report))
        })
    }

    pub(crate) fn select_repair_adapter(&mut self, target: InterfaceTarget) -> Task<Message> {
        if !self.diagnostics.as_ref().is_some_and(|report| {
            report
                .ipv6
                .adapters
                .iter()
                .any(|adapter| adapter.target == target)
        }) {
            return Task::none();
        }
        self.ipv6_repair.attempted = None;
        self.ipv6_repair.before = None;
        self.run_diagnostics(Some(target))
    }

    pub(crate) fn accept_diagnostics(&mut self, run: u64, report: Arc<Report>) -> Task<Message> {
        if self.ipv6_repair.run != run {
            return Task::none();
        }
        self.diagnostics_running = false;
        self.ipv6_repair.finished_at = Some(Instant::now());
        self.diagnostics = Some(report);
        self.refresh()
    }

    fn repair_action(&self, stack_reset: bool) -> Option<MaintenanceAction> {
        if !cfg!(windows) || preview_only() || !self.can_run_diagnostics() {
            return None;
        }
        let report = &self.diagnostics.as_ref()?.ipv6;
        if report.recovered || self.ipv6_repair.boot_status.is_some() {
            return None;
        }
        if stack_reset {
            let prior = self.ipv6_repair.attempted.as_ref()?.ipv6_target()?;
            return (self.show_advanced
                && report.local_stack_suspected
                && report.target.as_ref() == Some(prior)
                && matches!(
                    self.ipv6_repair.result,
                    Some(RepairResult::StillUnavailable)
                ))
            .then_some(MaintenanceAction::ResetIpv6Stack);
        }
        let kind = report.recommendation?;
        if kind == Ipv6RepairKind::FlushDns {
            return Some(MaintenanceAction::FlushDnsCache);
        }
        let interface = report.target.clone()?;
        Some(match kind {
            Ipv6RepairKind::Enable => MaintenanceAction::EnableIpv6 { interface },
            Ipv6RepairKind::RenewDhcp => MaintenanceAction::RenewDhcpv6 { interface },
            Ipv6RepairKind::RestartAdapter => MaintenanceAction::RestartIpv6Adapter { interface },
            Ipv6RepairKind::ResetBinding
                if matches!(
                    &self.ipv6_repair.attempted,
                    Some(MaintenanceAction::ResetIpv6 { interface: prior }) if *prior == interface
                ) && matches!(
                    self.ipv6_repair.result,
                    Some(RepairResult::StillUnavailable)
                ) =>
            {
                MaintenanceAction::RestartIpv6Adapter { interface }
            }
            Ipv6RepairKind::ResetBinding => MaintenanceAction::ResetIpv6 { interface },
            Ipv6RepairKind::FlushDns => unreachable!("handled before selecting the adapter"),
        })
    }

    pub(crate) fn ipv6_repair_label(&self, action: &MaintenanceAction) -> &'static str {
        match action {
            MaintenanceAction::EnableIpv6 { .. } => self.t("启用 IPv6", "Enable IPv6"),
            MaintenanceAction::RenewDhcpv6 { .. } => self.t("续租 DHCPv6", "Renew DHCPv6"),
            MaintenanceAction::ResetIpv6 { .. } => self.t("重新获取 IPv6", "Reinitialize IPv6"),
            MaintenanceAction::RestartIpv6Adapter { .. } => {
                self.t("重启这块网卡", "Restart this adapter")
            }
            MaintenanceAction::FlushDnsCache => self.t("清除 DNS 缓存", "Clear DNS cache"),
            MaintenanceAction::ResetIpv6Stack => {
                self.t("重置 IPv6 协议栈…", "Reset the IPv6 stack…")
            }
            _ => self.t("修复 IPv6", "Repair IPv6"),
        }
    }

    pub(crate) fn ipv6_repair_confirmation(&self, action: &MaintenanceAction) -> &'static str {
        match action {
            MaintenanceAction::ResetIpv6Stack => self.t(
                "BitNet 会先备份本机 IPv6 配置。重置会影响所有网卡，可能移除静态地址、路由和 DNS 设置，远程桌面可能断开。请在方便重启 Windows 时继续。",
                "BitNet backs up the IPv6 configuration first. Resetting affects every adapter and may remove static addresses, routes and DNS settings. Remote Desktop may disconnect. Continue when you can restart Windows.",
            ),
            MaintenanceAction::RestartIpv6Adapter { .. } => self.t(
                "重启这块网卡会中断其 IPv4 和 IPv6 连接。远程桌面可能断开。需要管理员确认。",
                "Restarting this adapter interrupts its IPv4 and IPv6 connections. Remote Desktop may disconnect. Windows will ask for administrator approval.",
            ),
            MaintenanceAction::FlushDnsCache => self.t(
                "Windows 会清除本机保存的域名查询结果。需要管理员确认。",
                "Windows will clear cached DNS answers on this computer and ask for administrator approval.",
            ),
            _ => self.t(
                "此操作只针对所选网卡。正在使用的连接可能短暂中断，Windows 会请求管理员确认。",
                "This operation targets the selected adapter. Active connections may briefly drop. Windows will ask for administrator approval.",
            ),
        }
    }

    pub(crate) fn guided_repair_label(&self, stack_reset: bool) -> Option<&'static str> {
        self.repair_action(stack_reset)
            .map(|action| self.ipv6_repair_label(&action))
    }

    pub(crate) fn ask_ipv6_repair(&mut self, stack_reset: bool) -> Task<Message> {
        let Some(action) = self.repair_action(stack_reset) else {
            return Task::none();
        };
        let adapter_name = action.ipv6_target().and_then(|target| {
            self.diagnostics
                .as_ref()?
                .ipv6
                .adapters
                .iter()
                .find(|adapter| &adapter.target == target)
                .map(|adapter| adapter.name.clone())
        });
        self.confirmation = Some(Confirmation::Ipv6Repair(Ipv6RepairRequest {
            run: self.ipv6_repair.run,
            adapter_name,
            action,
        }));
        Task::none()
    }

    pub(crate) fn start_ipv6_repair(&mut self, request: Ipv6RepairRequest) -> Task<Message> {
        if self.busy || self.connecting || self.diagnostics_running || preview_only() {
            return Task::none();
        }
        let Some(context) = self.ipv6_repair.context.as_ref() else {
            return Task::none();
        };
        if request.run != self.ipv6_repair.run
            || self
                .ipv6_repair
                .finished_at
                .is_none_or(|end| end.elapsed() > REPORT_LIFETIME)
            || self.new_probe_context().settings != context.settings
        {
            self.show_notice(
                true,
                self.t(
                    "诊断结果已过期，请重新检测。",
                    "Run diagnostics again before repairing.",
                )
                .into(),
            );
            return Task::none();
        }
        if request.requires_advanced() && self.repair_action(true).is_none() {
            return Task::none();
        }
        self.busy = true;
        self.request_epoch = self.request_epoch.wrapping_add(1);
        self.ipv6_repair.before = self
            .diagnostics
            .as_ref()
            .map(|report| report.ipv6.summary.clone());
        self.ipv6_repair.result = Some(RepairResult::Applying);
        let action = request.action.clone();
        self.ipv6_repair.active = Some(request);
        Task::perform(
            maintenance::execute(Scope::System, action, true),
            Message::GuidedRepairCompleted,
        )
    }

    pub(crate) fn ipv6_repair_completed(
        &mut self,
        result: Result<MaintenanceResult, AppError>,
    ) -> Task<Message> {
        let Some(request) = self.ipv6_repair.active.take() else {
            return Task::none();
        };
        match result {
            Ok(result) if request.requires_advanced() => {
                self.busy = false;
                let backup_dir = result.details["ipv6_backup_dir"]
                    .as_str()
                    .map(PathBuf::from)
                    .unwrap_or_default();
                self.remember_recheck_targets(
                    &backup_dir,
                    result.details["ipv6_reset_id"].as_str(),
                );
                self.ipv6_repair.result = Some(RepairResult::RebootRequired(backup_dir, false));
                Self::observe_ipv6_stack_reset()
            }
            Ok(_) => {
                let selected = request
                    .action
                    .ipv6_target()
                    .cloned()
                    .or_else(|| self.ipv6_repair.last_target.clone());
                self.ipv6_repair.attempted = Some(request.action);
                let context = self
                    .ipv6_repair
                    .context
                    .clone()
                    .unwrap_or_else(|| self.new_probe_context());
                self.recheck_ipv6(context, selected, None)
            }
            Err(error) => {
                self.busy = false;
                let cancelled = error.code == ErrorCode::Cancelled;
                self.ipv6_repair.result = Some(if cancelled {
                    RepairResult::Cancelled
                } else {
                    RepairResult::Failed(error)
                });
                if request.requires_advanced() && !cancelled {
                    Self::observe_ipv6_stack_reset()
                } else {
                    Task::none()
                }
            }
        }
    }

    fn recheck_ipv6(
        &mut self,
        context: ProbeContext,
        selected: Option<InterfaceTarget>,
        after_boot: Option<Ipv6StackResetStatus>,
    ) -> Task<Message> {
        self.busy = true;
        self.diagnostics_running = true;
        self.ipv6_repair.run = self.ipv6_repair.run.wrapping_add(1);
        self.ipv6_repair.context = Some(context.clone());
        self.ipv6_repair.result = Some(RepairResult::Rechecking);
        let run = self.ipv6_repair.run;
        Task::perform(
            async move {
                let report = context.report(selected, true).await;
                let recovered = report.ipv6.recovered;
                let result = if let Some(status) = after_boot {
                    RepairResult::AfterReboot {
                        recovered,
                        interrupted: status.interrupted,
                        backup_dir: status.backup_dir,
                    }
                } else if recovered {
                    RepairResult::Recovered
                } else {
                    RepairResult::StillUnavailable
                };
                (Arc::new(report), result)
            },
            move |(report, result)| Message::GuidedRecheckReady(run, report, result),
        )
    }

    pub(crate) fn ipv6_recheck_ready(
        &mut self,
        run: u64,
        report: Arc<Report>,
        result: RepairResult,
    ) -> Task<Message> {
        if run != self.ipv6_repair.run {
            return Task::none();
        }
        self.busy = false;
        if matches!(result, RepairResult::AfterReboot { .. }) {
            self.acknowledge_reset();
        }
        self.ipv6_repair.result = Some(result);
        self.accept_diagnostics(run, report)
    }

    fn remember_recheck_targets(&mut self, backup_dir: &Path, reset_id: Option<&str>) {
        let reset_id = reset_id
            .filter(|id| maintenance::is_reset_id(id))
            .or_else(|| maintenance::reset_id_from_backup(backup_dir));
        let Some(reset_id) = reset_id else {
            let message = self.t(
                "无法确认本次 IPv6 重置的标识，重启后将使用当前的检测目标。",
                "The IPv6 reset could not be identified, so the recheck will use the current targets.",
            );
            self.show_notice(true, message.into());
            return;
        };
        let settings = self.ipv6_repair.context.as_ref().map_or_else(
            || self.new_probe_context().settings,
            |context| context.settings.clone(),
        );
        if let Err(error) = settings.save_for_reboot(reset_id) {
            self.error(error);
        }
    }

    /// The reset record is acknowledged only after its recheck finished, so a
    /// failed acknowledgment leaves the pending state for the next start.
    fn acknowledge_reset(&mut self) {
        let Some(id) = self
            .ipv6_repair
            .boot_status
            .as_ref()
            .map(|status| status.id.clone())
        else {
            self.ipv6_repair.acknowledged = None;
            return;
        };
        self.ipv6_repair.boot_status = None;
        match maintenance::acknowledge_ipv6_stack_reset(&id) {
            Ok(()) => {
                self.ipv6_repair.acknowledged = Some(true);
                ProbeSettings::discard_saved();
            }
            Err(error) => {
                self.ipv6_repair.acknowledged = Some(false);
                self.error(error);
            }
        }
    }

    fn recheck_rejection_notice(&self, rejection: RecheckRejection) -> &'static str {
        match rejection {
            RecheckRejection::OtherReset => self.t(
                "保存的复测目标属于另一次重置，已丢弃并使用当前设置。",
                "The saved recheck target belongs to another reset and was discarded.",
            ),
            RecheckRejection::UnsupportedSchema => self.t(
                "保存的复测目标来自其他版本，已丢弃并使用当前设置。",
                "The saved recheck target is from another version and was discarded.",
            ),
            RecheckRejection::Expired => self.t(
                "保存的复测目标已过期，已丢弃并使用当前设置。",
                "The saved recheck target expired and was discarded.",
            ),
            RecheckRejection::Damaged => self.t(
                "保存的复测目标无法读取，已丢弃并使用当前设置。",
                "The saved recheck target could not be read and was discarded.",
            ),
            RecheckRejection::InvalidSettings => self.t(
                "保存的复测目标无效，已丢弃并使用当前设置。",
                "The saved recheck target was invalid and was discarded.",
            ),
        }
    }

    pub(crate) fn observe_ipv6_stack_reset() -> Task<Message> {
        Task::perform(
            maintenance::ipv6_stack_reset_status(),
            Message::Ipv6StackResetObserved,
        )
    }

    pub(crate) fn ipv6_stack_reset_observed(
        &mut self,
        result: Result<Option<Ipv6StackResetStatus>, AppError>,
    ) -> Task<Message> {
        match result {
            Ok(Some(status)) if status.reboot_required => {
                self.ipv6_repair.result = Some(RepairResult::RebootRequired(
                    status.backup_dir.clone(),
                    status.interrupted,
                ));
                self.ipv6_repair.boot_status = Some(status);
            }
            Ok(status) => {
                self.ipv6_repair.boot_status = status;
                return self.recheck_ipv6_after_boot();
            }
            Err(error) => self.error(error),
        }
        Task::none()
    }

    pub(crate) fn recheck_ipv6_after_boot(&mut self) -> Task<Message> {
        if self.busy || self.connecting || self.diagnostics_running {
            return Task::none();
        }
        let Some(status) = self
            .ipv6_repair
            .boot_status
            .clone()
            .filter(|status| !status.reboot_required)
        else {
            return Task::none();
        };
        let mut context = self.new_probe_context();
        match ProbeSettings::for_reboot(&status.id) {
            StoredSettings::Restored(settings) => context.settings = settings,
            StoredSettings::Absent => (),
            StoredSettings::Rejected(rejection) => {
                let message = self.recheck_rejection_notice(rejection);
                self.show_notice(true, message.into());
            }
        }
        self.recheck_ipv6(context, None, Some(status))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
    };

    const RESET: &str = "reset-2468-1700000000000-0";
    const OTHER_RESET: &str = "reset-2468-1700000000001-0";

    /// Fixture directories live under the ignored target tree so tests never
    /// write into a real user profile.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root =
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/gui-recheck-tests");
            fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            let path = root.join(format!(
                "fixture-{}-{}-{}",
                std::process::id(),
                bitnet_core::now_ms(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        /// A profile directory that does not exist yet, like a first launch.
        fn data_dir(&self) -> PathBuf {
            self.0.join("BitNet")
        }

        fn record(&self) -> PathBuf {
            ProbeSettings::recheck_path(&self.data_dir())
        }

        fn write(&self, bytes: &[u8]) {
            fs::create_dir_all(self.data_dir()).unwrap();
            fs::write(self.record(), bytes).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/gui-recheck-tests")
                .canonicalize()
                .unwrap();
            if let Ok(resolved) = self.0.canonicalize() {
                if resolved.parent() == Some(allowed.as_path()) && resolved.starts_with(&allowed) {
                    let _ = fs::remove_dir_all(resolved);
                }
            }
        }
    }

    fn targets() -> ProbeSettings {
        ProbeSettings::from(&Settings::default())
    }

    fn record_bytes(reset_id: &str, written_ms: u64, settings: &ProbeSettings) -> Vec<u8> {
        serde_json::to_vec(&RecheckRecord {
            schema: RECHECK_SCHEMA,
            reset_id: reset_id.into(),
            written_ms,
            settings: settings.clone(),
        })
        .unwrap()
    }

    #[test]
    fn a_new_profile_can_save_the_recheck_targets_for_a_reset() {
        let scratch = Scratch::new();
        assert!(!scratch.data_dir().exists());
        targets().save_at(&scratch.data_dir(), RESET).unwrap();
        assert!(scratch.record().is_file());
        match ProbeSettings::restore_at(&scratch.data_dir(), RESET) {
            StoredSettings::Restored(restored) => assert_eq!(restored, targets()),
            _ => panic!("a saved target must be restored for the same reset"),
        }
    }

    #[test]
    fn targets_saved_for_another_reset_are_discarded() {
        let scratch = Scratch::new();
        targets().save_at(&scratch.data_dir(), OTHER_RESET).unwrap();
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::OtherReset)
        ));
        assert!(!scratch.record().exists());
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Absent
        ));
    }

    #[test]
    fn a_missing_profile_directory_reports_no_saved_targets() {
        let scratch = Scratch::new();
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Absent
        ));
    }

    #[test]
    fn damaged_or_oversized_records_are_rejected() {
        let scratch = Scratch::new();
        scratch.write(b"{\"schema\":1,\"reset_id\":");
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::Damaged)
        ));
        assert!(!scratch.record().exists());

        let mut oversized = record_bytes(RESET, now_ms(), &targets());
        oversized.resize(RECHECK_MAX_BYTES + 1, b' ');
        scratch.write(&oversized);
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::Damaged)
        ));
        assert!(!scratch.record().exists());
    }

    #[test]
    fn records_from_another_schema_or_past_their_lifetime_are_rejected() {
        let scratch = Scratch::new();
        let mut unknown = record_bytes(RESET, now_ms(), &targets());
        unknown = String::from_utf8(unknown)
            .unwrap()
            .replace("\"schema\":1", "\"schema\":99")
            .into_bytes();
        scratch.write(&unknown);
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::UnsupportedSchema)
        ));

        let expired = now_ms() - RECHECK_MAX_AGE.as_millis() as u64 - 1;
        scratch.write(&record_bytes(RESET, expired, &targets()));
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::Expired)
        ));
        assert!(!scratch.record().exists());
    }

    #[test]
    fn saved_targets_must_still_validate_and_name_a_reset() {
        let scratch = Scratch::new();
        let invalid = ProbeSettings {
            ipv6_targets: vec!["file:///tmp/ipv6".into()],
            timeout_secs: 5,
        };
        scratch.write(&record_bytes(RESET, now_ms(), &invalid));
        assert!(matches!(
            ProbeSettings::restore_at(&scratch.data_dir(), RESET),
            StoredSettings::Rejected(RecheckRejection::InvalidSettings)
        ));

        let unbound = targets().save_at(&scratch.data_dir(), "reset-2468");
        assert_eq!(unbound.unwrap_err().code, ErrorCode::InvalidInput);
        assert!(!scratch.record().exists());
    }
}
