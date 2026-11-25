use crate::{
    diagnostics, maintenance, preview_only, App, AppError, Client, Command, Confirmation,
    ErrorCode, InterfaceTarget, MaintenanceAction, MaintenanceResult, Message, Scope, Settings,
    Snapshot, Task,
};
use bitnet_core::{HealthStatus, MAX_IPV6_TARGETS};
use diagnostics::{Ipv6RepairKind, Report};
use maintenance::Ipv6StackResetStatus;
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::time::{sleep, timeout_at, Instant};

const RECHECK_WINDOW: Duration = Duration::from_secs(30);
const REPORT_LIFETIME: Duration = Duration::from_secs(60);

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct ProbeSettings {
    ipv6_targets: Vec<String>,
    timeout_secs: u64,
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

    fn backup_path() -> Result<PathBuf, AppError> {
        let paths = bitnet_platform::paths(Scope::User)?;
        Ok(paths.data_dir.join("ipv6-recheck.json"))
    }

    fn save_for_reboot(&self) -> Result<(), AppError> {
        let path = Self::backup_path()?;
        let data = serde_json::to_vec(self)
            .map_err(|_| AppError::new(ErrorCode::Internal, "无法记录重启后的检测目标"))?;
        bitnet_platform::write_private_file(&path, &data)
    }

    fn for_reboot() -> Option<Self> {
        let path = Self::backup_path().ok()?;
        let bytes = bitnet_platform::read_private_file(&path).ok()??;
        if bytes.len() > MAX_IPV6_TARGETS * 2048 + 512 {
            return None;
        }
        let settings: Self = serde_json::from_slice(&bytes).ok()?;
        settings.settings().validate().ok()?;
        Some(settings)
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
        self.show_evidence = false;
    }

    pub(crate) fn before(&self) -> Option<&str> {
        self.before.as_deref()
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
        if request.requires_advanced() {
            if self.repair_action(true).is_none() {
                return Task::none();
            }
            if let Err(error) = context.settings.save_for_reboot() {
                self.error(error);
                return Task::none();
            }
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
                self.ipv6_repair.result = Some(RepairResult::RebootRequired(
                    result.details["ipv6_backup_dir"]
                        .as_str()
                        .map(PathBuf::from)
                        .unwrap_or_default(),
                    false,
                ));
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
                    let _ = maintenance::acknowledge_ipv6_stack_reset(&status.id);
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
            self.ipv6_repair.boot_status = None;
        }
        self.ipv6_repair.result = Some(result);
        self.accept_diagnostics(run, report)
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
        if let Some(settings) = ProbeSettings::for_reboot() {
            context.settings = settings;
        }
        self.recheck_ipv6(context, None, Some(status))
    }
}
