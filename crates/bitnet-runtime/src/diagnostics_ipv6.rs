use super::{t, Rows, PROBE_TIMEOUT};
use crate::{health_probe, maintenance::InterfaceTarget};
use bitnet_core::{is_public_ipv6, DnsProbeResult, HealthStatus, Settings, DNS_FREE_IPV6_TARGET};
use serde::Deserialize;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::Duration,
};

pub(super) const REFERENCE: Ipv6Addr = Ipv6Addr::new(0x2606, 0x4700, 0x4700, 0, 0, 0, 0, 0x1111);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipv6Outcome {
    Normal,
    Failed,
    Inconclusive,
    NotRun,
}

impl Ipv6Outcome {
    pub fn label(self, english: bool) -> &'static str {
        match self {
            Self::Normal => t(english, "正常", "Healthy"),
            Self::Failed => t(english, "失败", "Failed"),
            Self::Inconclusive => t(english, "未能判断", "Inconclusive"),
            Self::NotRun => t(english, "未执行", "Not run"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ipv6RepairKind {
    Enable,
    RenewDhcp,
    ResetBinding,
    RestartAdapter,
    FlushDns,
}

#[derive(Clone, Debug)]
pub struct Ipv6RepairAdapter {
    pub name: String,
    pub target: InterfaceTarget,
}

#[derive(Clone, Debug)]
pub struct Ipv6Assessment {
    pub checked_at: Option<u64>,
    pub recovered: bool,
    pub outcome: Ipv6Outcome,
    pub summary: String,
    pub adapters: Vec<Ipv6RepairAdapter>,
    pub target: Option<InterfaceTarget>,
    pub recommendation: Option<Ipv6RepairKind>,
    pub local_stack_suspected: bool,
    pub health: HealthStatus,
    /// Compact display evidence without IP addresses, for the desktop summary.
    pub evidence: Vec<(String, String)>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NativeIpv6 {
    pub adapters: Vec<NativeAdapter>,
    pub addresses: Vec<NativeAddress>,
    pub routes: Vec<NativeRoute>,
    pub disabled_components: Option<u32>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NativeAdapter {
    index: u32,
    guid: String,
    name: String,
    link_up: bool,
    repair_eligible: bool,
    binding: Option<bool>,
    dhcp: String,
    managed_address: String,
    router_discovery: String,
}

impl NativeAdapter {
    fn target(&self) -> Option<InterfaceTarget> {
        if !self.link_up || !self.repair_eligible || self.binding.is_none() {
            return None;
        }
        InterfaceTarget::parse(&format!("{}@{}", self.guid, self.index)).ok()
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NativeAddress {
    index: u32,
    address: Option<Ipv6Addr>,
    state: String,
    prefix_origin: String,
    suffix_origin: String,
    skip_as_source: bool,
    preferred_lifetime: String,
    valid_lifetime: String,
}

impl NativeAddress {
    fn can_be_source(&self) -> bool {
        self.state.eq_ignore_ascii_case("Preferred")
            && !self.skip_as_source
            && self.address.is_some_and(is_public_ipv6)
    }

    fn is_dhcp(&self) -> bool {
        self.prefix_origin.eq_ignore_ascii_case("Dhcp")
            || self.suffix_origin.eq_ignore_ascii_case("Dhcp")
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(super) struct NativeRoute {
    destination: Option<IpAddr>,
    index: Option<u32>,
    prefix: String,
    source: Option<IpAddr>,
    next_hop: Option<IpAddr>,
    neighbor: String,
}

#[derive(Debug, Default)]
pub(super) struct ReferenceProbe {
    pub connected: bool,
    pub error: Option<i32>,
    pub source: Option<IpAddr>,
    pub https: bool,
    pub elapsed_ms: u128,
}

pub(super) async fn reference_probe(health: &HealthStatus, settings: &Settings) -> ReferenceProbe {
    let started = std::time::Instant::now();
    let prior_https = health
        .ipv6_probes
        .iter()
        .any(|probe| probe.literal_https_reachable);
    let https = async {
        if settings
            .ipv6_targets
            .iter()
            .any(|target| target == DNS_FREE_IPV6_TARGET)
        {
            return prior_https;
        }
        let settings = Settings {
            ipv6_targets: vec![DNS_FREE_IPV6_TARGET.to_owned()],
            request_timeout_secs: 3,
            ..Settings::default()
        };
        let mut result = HealthStatus::default();
        health_probe::run_ipv6(&settings, None, &[], &mut result).await;
        result
            .ipv6_probes
            .iter()
            .any(|probe| probe.literal_https_reachable)
    };
    let tcp = tokio::time::timeout(
        PROBE_TIMEOUT,
        tokio::net::TcpStream::connect(SocketAddr::new(REFERENCE.into(), 443)),
    );
    let (tcp, https) = tokio::join!(tcp, https);
    let (connected, source, error) = match tcp {
        Ok(Ok(socket)) => (
            true,
            socket.local_addr().ok().map(|address| address.ip()),
            None,
        ),
        Ok(Err(error)) => (false, None, error.raw_os_error()),
        Err(_) => (false, None, None),
    };
    ReferenceProbe {
        connected,
        error,
        source,
        https,
        elapsed_ms: started.elapsed().as_millis(),
    }
}

pub(super) fn route_targets(health: &HealthStatus) -> Vec<IpAddr> {
    let mut destinations = vec![IpAddr::V6(REFERENCE)];
    for address in health
        .ipv6_probes
        .iter()
        .filter_map(|probe| probe.peer_address)
        .chain(
            health
                .ipv6_probes
                .iter()
                .flat_map(|probe| probe.resolved_addresses.iter().copied()),
        )
    {
        if destinations.len() >= 16 {
            break;
        }
        if !destinations.contains(&address) && !address.is_loopback() {
            destinations.push(address);
        }
    }
    destinations
}

fn is_dns_failure(result: Option<DnsProbeResult>) -> bool {
    matches!(
        result,
        Some(DnsProbeResult::Timeout | DnsProbeResult::Empty | DnsProbeResult::LookupFailed)
    )
}

pub(super) async fn confirm_dns_failure(health: &HealthStatus, settings: &Settings) -> bool {
    let failed = {
        let mut domains = health
            .ipv6_probes
            .iter()
            .filter_map(|probe| probe.dns_result)
            .peekable();
        domains.peek().is_some() && domains.all(|result| is_dns_failure(Some(result)))
    };
    if !failed {
        return false;
    }
    let hosts: Vec<_> = settings
        .ipv6_targets
        .iter()
        .filter_map(|target| bitnet_core::parse_ipv6_target(target).ok())
        .filter_map(|url| match url.host()? {
            url::Host::Domain(name) => Some(name.to_owned()),
            _ => None,
        })
        .collect();
    let results = futures_util::future::join_all(hosts.into_iter().map(|host| async move {
        match tokio::time::timeout(Duration::from_secs(2), tokio::net::lookup_host((host, 0))).await
        {
            Ok(Ok(mut addresses)) => addresses.next().is_none(),
            _ => true,
        }
    }))
    .await;
    !results.is_empty() && results.into_iter().all(|failed| failed)
}

fn selected_adapter<'a>(
    native: &'a NativeIpv6,
    health: &HealthStatus,
    selected: Option<&InterfaceTarget>,
) -> Option<&'a NativeAdapter> {
    if let Some(selected) = selected {
        return native
            .adapters
            .iter()
            .find(|adapter| adapter.target().as_ref() == Some(selected));
    }
    let observed: Vec<_> = health
        .ipv6_probes
        .iter()
        .filter_map(|probe| probe.peer_address)
        .filter(IpAddr::is_ipv6)
        .collect();
    let mut routes = native
        .routes
        .iter()
        .filter(|route| route.destination.is_some_and(|address| address.is_ipv6()))
        .filter(|route| {
            observed.is_empty()
                || route
                    .destination
                    .is_some_and(|peer| observed.contains(&peer))
        })
        .filter_map(|route| route.index);
    let index = routes.next()?;
    if routes.any(|other| other != index) {
        return None;
    }
    native
        .adapters
        .iter()
        .find(|adapter| adapter.index == index && adapter.target().is_some())
}

pub(super) fn pending_address_check(
    native: &NativeIpv6,
    health: &HealthStatus,
    selected: Option<&InterfaceTarget>,
) -> Option<u32> {
    let adapter = selected_adapter(native, health, selected)?;
    let tentative = native.addresses.iter().any(|address| {
        address.index == adapter.index && address.state.eq_ignore_ascii_case("Tentative")
    });
    let usable = native
        .addresses
        .iter()
        .any(|address| address.index == adapter.index && address.can_be_source());
    (tentative && !usable && health.confirmed_ipv6().is_none()).then_some(adapter.index)
}

pub(super) fn assess(
    native: &NativeIpv6,
    health: HealthStatus,
    selected: Option<&InterfaceTarget>,
    resampled: Option<&NativeIpv6>,
    reference: &ReferenceProbe,
    repeated_dns_failure: bool,
    english: bool,
) -> Ipv6Assessment {
    let label = |zh, en| t(english, zh, en);
    let adapter = selected_adapter(native, &health, selected);
    let target = adapter.and_then(NativeAdapter::target);
    let adapters = native
        .adapters
        .iter()
        .filter_map(|adapter| {
            Some(Ipv6RepairAdapter {
                name: adapter.name.clone(),
                target: adapter.target()?,
            })
        })
        .collect::<Vec<_>>();
    let recovered = health.confirmed_ipv6().is_some();
    let mut result = Ipv6Assessment {
        checked_at: health.ipv6_checked_at,
        recovered,
        outcome: Ipv6Outcome::Inconclusive,
        summary: String::new(),
        adapters,
        target,
        recommendation: None,
        local_stack_suspected: false,
        health,
        evidence: Vec::new(),
    };
    let tentative = |address: &&NativeAddress| address.state.eq_ignore_ascii_case("Tentative");
    let relay_route = native
        .routes
        .iter()
        .filter_map(|route| route.index)
        .any(|index| {
            native.adapters.iter().any(|adapter| {
                adapter.index == index && adapter.link_up && !adapter.repair_eligible
            })
        });
    let (outcome, summary, recommendation) = if recovered {
        (
            Ipv6Outcome::Normal,
            label(
                "检测站点确认了公网 IPv6",
                "A probe site confirmed public IPv6.",
            ),
            None,
        )
    } else if result.checked_at.is_none() || result.health.ipv6_probes.is_empty() {
        (
            Ipv6Outcome::NotRun,
            label("尚无 IPv6 探测结果", "No IPv6 probe has run."),
            None,
        )
    } else if (reference.connected || reference.https) && repeated_dns_failure {
        (
            Ipv6Outcome::Failed,
            label(
                "固定 IPv6 地址可达，但域名连续解析失败",
                "A fixed IPv6 address works, but DNS lookups repeatedly failed.",
            ),
            Some(Ipv6RepairKind::FlushDns),
        )
    } else if reference.https {
        (
            Ipv6Outcome::Inconclusive,
            label(
                "固定 IPv6 地址可达，检测站点未返回公网 IPv6",
                "A fixed IPv6 address works. The probe sites did not return public IPv6.",
            ),
            None,
        )
    } else if native
        .disabled_components
        .is_some_and(|value| value & 0x10 != 0)
    {
        (Ipv6Outcome::Failed, label("Windows 的 IPv6 配置禁止了常规网卡的 IPv6，请核对设备策略", "Windows IPv6 configuration disables IPv6 on regular adapters. Review the device policy."), None)
    } else if relay_route {
        (Ipv6Outcome::Inconclusive, label("部分 IPv6 目标经过隧道或其他虚拟接口，请检查其选路和代理配置", "Some targets use a tunnel or another virtual interface. Review its route and proxy configuration."), None)
    } else if let Some(adapter) = adapter {
        let addresses: Vec<_> = native
            .addresses
            .iter()
            .filter(|address| address.index == adapter.index)
            .collect();
        let usable = addresses.iter().any(|address| address.can_be_source());
        let duplicate = !usable
            && addresses
                .iter()
                .any(|address| address.state.eq_ignore_ascii_case("Duplicate"));
        let pending = addresses.iter().any(tentative);
        if adapter.binding == Some(false) {
            (
                Ipv6Outcome::Failed,
                label(
                    "所选网卡的 IPv6 已关闭",
                    "IPv6 is disabled on the selected adapter.",
                ),
                Some(Ipv6RepairKind::Enable),
            )
        } else if duplicate {
            (Ipv6Outcome::Failed, label("所选网卡发现 IPv6 地址冲突，请检查同一网络中的重复地址", "The selected adapter has a duplicate IPv6 address. Check for another device using it."), None)
        } else if !usable && pending {
            let still_pending = resampled.is_some_and(|next| {
                next.adapters.iter().any(|current| {
                    current.index == adapter.index
                        && current.target().as_ref() == result.target.as_ref()
                }) && !next
                    .addresses
                    .iter()
                    .any(|address| address.index == adapter.index && address.can_be_source())
                    && addresses
                        .iter()
                        .filter(|address| tentative(address))
                        .any(|old| {
                            next.addresses.iter().any(|address| {
                                address.index == adapter.index
                                    && address.address == old.address
                                    && tentative(&address)
                            })
                        })
            });
            if still_pending {
                result.local_stack_suspected = true;
                (
                    Ipv6Outcome::Failed,
                    label(
                        "网卡的 IPv6 地址持续处于初始化状态",
                        "The adapter's IPv6 address stays in the initializing state.",
                    ),
                    Some(Ipv6RepairKind::RestartAdapter),
                )
            } else {
                (
                    Ipv6Outcome::Inconclusive,
                    label(
                        "网卡正在确认地址可用性，请稍后复测",
                        "The adapter is checking its IPv6 address. Try another check shortly.",
                    ),
                    None,
                )
            }
        } else if !usable
            && adapter.dhcp.eq_ignore_ascii_case("Enabled")
            && (adapter.managed_address.eq_ignore_ascii_case("Enabled")
                || addresses.iter().any(|address| address.is_dhcp()))
        {
            (
                Ipv6Outcome::Failed,
                label(
                    "未找到可用的 DHCPv6 地址",
                    "No usable DHCPv6 address was found.",
                ),
                Some(Ipv6RepairKind::RenewDhcp),
            )
        } else if !usable && !adapter.router_discovery.eq_ignore_ascii_case("Disabled") {
            (Ipv6Outcome::Failed, label("网卡未获得可用于公网的 IPv6 地址，请重新获取地址和路由", "The adapter has no suitable public IPv6 address. Refresh its addresses and routes."), Some(Ipv6RepairKind::ResetBinding))
        } else {
            result.local_stack_suspected = usable
                && matches!(reference.error, Some(10047 | 10049))
                && native.routes.iter().any(|route| {
                    route.index == Some(adapter.index)
                        && route.destination == Some(REFERENCE.into())
                });
            let summary = if result.local_stack_suspected {
                label(
                    "Windows 无法创建或选择 IPv6 本地套接字",
                    "Windows could not create or select a local IPv6 socket.",
                )
            } else {
                label("站点未确认 IPv6，请核对目标路由、下一跳和网络上游", "The sites did not confirm IPv6. Review the route, next hop and upstream network.")
            };
            (
                Ipv6Outcome::Inconclusive,
                summary,
                result
                    .local_stack_suspected
                    .then_some(Ipv6RepairKind::ResetBinding),
            )
        }
    } else if result.adapters.is_empty() {
        (
            Ipv6Outcome::Inconclusive,
            label(
                "无法确认可管理的 IPv6 网卡，请检查链路和网卡状态",
                "No manageable IPv6 adapter was identified. Check the link and adapters.",
            ),
            None,
        )
    } else {
        (
            Ipv6Outcome::Inconclusive,
            label(
                "请选择要检查的 IPv6 网卡",
                "Choose the IPv6 adapter to inspect.",
            ),
            None,
        )
    };
    result.outcome = outcome;
    result.summary = summary.into();
    result.recommendation = recommendation;
    if let Some(adapter) = adapter {
        let binding = match adapter.binding {
            Some(true) => label("IPv6 已启用", "IPv6 enabled"),
            Some(false) => label("IPv6 已关闭", "IPv6 disabled"),
            None => label("无法确认绑定", "Binding unknown"),
        };
        result.evidence.push((
            label("所选网卡", "Selected adapter").into(),
            format!("{} · {binding}", adapter.name),
        ));
    }
    if result.checked_at.is_some() {
        let route_count = native
            .routes
            .iter()
            .filter(|route| route.index.is_some())
            .count();
        result.evidence.push((
            label("已找到目标路由", "Destination routes found").into(),
            format!("{route_count}/{}", native.routes.len()),
        ));
        let confirmed = if reference.https {
            label("TCP / HTTPS 已验证", "TCP / HTTPS verified")
        } else if reference.connected {
            label(
                "TCP 可达 · HTTPS 未确认",
                "TCP reachable · HTTPS unconfirmed",
            )
        } else {
            label("不可达或未能确认", "Unreachable or unconfirmed")
        };
        result.evidence.push((
            label("固定 IPv6 地址", "Fixed IPv6 address").into(),
            confirmed.into(),
        ));
        if repeated_dns_failure {
            result.evidence.push((
                label("系统 DNS", "System DNS").into(),
                label("连续解析失败", "Repeated lookup failures").into(),
            ));
        }
    }
    result
}

pub(super) fn evidence_rows(
    native: &NativeIpv6,
    health: &HealthStatus,
    reference: &ReferenceProbe,
) -> Rows {
    let show = |value: Option<IpAddr>| value.map_or_else(|| "—".into(), |value| value.to_string());
    let mut rows: Rows = native
        .adapters
        .iter()
        .map(|adapter| {
            vec![
                "IPv6 binding / adapter".into(),
                format!("{} ({})", adapter.name, adapter.index),
                format!(
                    "link={} · binding={} · DHCP={} · router discovery={}",
                    if adapter.link_up { "up" } else { "down" },
                    adapter.binding.map_or("unknown", |value| if value {
                        "enabled"
                    } else {
                        "disabled"
                    }),
                    adapter.dhcp,
                    adapter.router_discovery
                ),
            ]
        })
        .collect();
    rows.extend(native.addresses.iter().map(|address| {
        vec![
            "IPv6 address".into(),
            format!(
                "{} · ifIndex {}",
                show(address.address.map(IpAddr::V6)),
                address.index
            ),
            format!(
                "{} · origin {}/{} · skip as source {} · preferred {} · valid {}",
                address.state,
                address.prefix_origin,
                address.suffix_origin,
                address.skip_as_source,
                address.preferred_lifetime,
                address.valid_lifetime
            ),
        ]
    }));
    rows.extend(native.routes.iter().map(|route| {
        vec![
            "System route / next hop".into(),
            show(route.destination),
            format!(
                "{} · ifIndex {} · source {} · next hop {} · NDP {}",
                route.prefix,
                route
                    .index
                    .map_or_else(|| "—".into(), |value| value.to_string()),
                show(route.source),
                show(route.next_hop),
                route.neighbor
            ),
        ]
    }));
    rows.extend(health.ipv6_probes.iter().map(|probe| {
        let family = |v6| {
            probe
                .resolved_addresses
                .iter()
                .filter(|ip| ip.is_ipv6() == v6)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        };
        vec![
            "DNS A / AAAA".into(),
            format!("IPv6 {:?} #{}", probe.kind, probe.target),
            format!(
                "{:?} · A [{}] · AAAA [{}] · socket {} → {}",
                probe.dns_result,
                family(false),
                family(true),
                show(probe.local_address),
                show(probe.peer_address)
            ),
        ]
    }));
    if health.ipv6_checked_at.is_some() {
        rows.push(vec![
            "Fixed IPv6 TCP / HTTPS".into(),
            format!("[{REFERENCE}]:443"),
            format!(
                "TCP {} · HTTPS verified {} · source {} · {} ms{}",
                if reference.connected {
                    "connected"
                } else {
                    "unconfirmed"
                },
                reference.https,
                show(reference.source),
                reference.elapsed_ms,
                reference
                    .error
                    .map_or_else(String::new, |code| format!(" · OS error {code}"))
            ),
        ]);
    }
    rows
}

pub(super) fn resample_rows(native: &NativeIpv6) -> Rows {
    native
        .addresses
        .iter()
        .map(|address| {
            vec![
                "IPv6 address after 5-second delay".into(),
                format!(
                    "{} · ifIndex {}",
                    address
                        .address
                        .map_or_else(|| "—".into(), |ip| ip.to_string()),
                    address.index
                ),
                address.state.clone(),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence(binding: bool, index: Option<u32>, routed_through_tunnel: bool) -> NativeIpv6 {
        serde_json::from_value(serde_json::json!({
            "adapters": [
                { "index": 15, "name": "vEthernet", "guid": "{11111111-2222-3333-4444-555555555555}",
                  "link_up": true, "binding": binding, "repair_eligible": true, "dhcp": "Enabled" },
                { "index": 16, "name": "Mihomo", "link_up": true, "binding": true, "repair_eligible": false }
            ],
            "routes": [{
                "destination": if routed_through_tunnel { "fdfe:dcba::1" } else { "2606:4700:4700::1111" },
                "index": index
            }]
        }))
        .unwrap()
    }

    fn observed(reported: Option<&str>) -> HealthStatus {
        serde_json::from_value(serde_json::json!({
            "internet": "unknown", "dns": "unknown", "ipv6": "unknown",
            "ipv6_checked_at": 100,
            "ipv6_probes": [{
                "target": 1, "kind": "test_ipv6", "attempted": 1, "succeeded": usize::from(reported.is_some()),
                "reported_address": reported, "peer_address": "fdfe:dcba::1", "timed_out": reported.is_none(), "elapsed_ms": 40
            }]
        }))
        .unwrap()
    }

    #[test]
    fn fake_ip_echo_keeps_working_even_with_no_default_or_literal_path() {
        let native = evidence(true, Some(16), true);
        let report = assess(
            &native,
            observed(Some("2001:4860::82")),
            None,
            None,
            &ReferenceProbe::default(),
            false,
            true,
        );
        assert!(report.recovered);
        assert_eq!(report.outcome, Ipv6Outcome::Normal);
        assert_eq!(report.recommendation, None);
    }

    #[test]
    fn failed_tunnel_is_reported_without_a_host_adapter_repair() {
        let native = evidence(true, Some(16), true);
        let report = assess(
            &native,
            observed(None),
            None,
            None,
            &ReferenceProbe::default(),
            false,
            true,
        );
        assert_eq!(report.outcome, Ipv6Outcome::Inconclusive);
        assert_eq!(report.recommendation, None);
        assert!(!report.local_stack_suspected);
    }

    #[test]
    fn ipv6_only_adapter_can_be_repaired_and_requires_choice_without_route() {
        let native = evidence(false, None, false);
        let reference = ReferenceProbe::default();
        let report = assess(&native, observed(None), None, None, &reference, false, true);
        assert_eq!(report.adapters.len(), 1);
        assert!(report.target.is_none());
        let report = assess(
            &native,
            observed(None),
            Some(&report.adapters[0].target),
            None,
            &reference,
            false,
            true,
        );
        assert_eq!(report.recommendation, Some(Ipv6RepairKind::Enable));
    }

    #[test]
    fn dns_failure_needs_fixed_ip_and_repeated_failed_lookup() {
        let native = evidence(true, None, false);
        let reference = ReferenceProbe {
            connected: true,
            ..ReferenceProbe::default()
        };
        let report = assess(&native, observed(None), None, None, &reference, false, true);
        assert_eq!(report.recommendation, None);
        let report = assess(&native, observed(None), None, None, &reference, true, true);
        assert_eq!(report.recommendation, Some(Ipv6RepairKind::FlushDns));
    }

    #[test]
    fn only_persisting_tentative_address_offers_adapter_restart() {
        let mut native = evidence(true, None, false);
        native.addresses.push(NativeAddress {
            index: 15,
            address: "2001:4860::82".parse().ok(),
            state: "Tentative".into(),
            ..NativeAddress::default()
        });
        let selected = native.adapters[0].target();
        let reference = ReferenceProbe::default();
        let report = assess(
            &native,
            observed(None),
            selected.as_ref(),
            None,
            &reference,
            false,
            true,
        );
        assert_eq!(report.recommendation, None);
        let report = assess(
            &native,
            observed(None),
            selected.as_ref(),
            Some(&native),
            &reference,
            false,
            true,
        );
        assert_eq!(report.recommendation, Some(Ipv6RepairKind::RestartAdapter));
        let mut passing = evidence(true, None, false);
        passing.addresses.push(NativeAddress {
            index: 15,
            address: "2001:4860::82".parse().ok(),
            state: "Preferred".into(),
            ..NativeAddress::default()
        });
        let report = assess(
            &native,
            observed(None),
            selected.as_ref(),
            Some(&passing),
            &reference,
            false,
            true,
        );
        assert_eq!(report.recommendation, None);
    }
}
