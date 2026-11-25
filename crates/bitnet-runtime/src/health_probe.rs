use bitnet_core::{
    is_cernet_ipv6, is_public_ipv6, now_secs, parse_ipv6_target, DnsProbeObservation,
    DnsProbeResult, Health, HealthExpect, HealthProbeExpectation, HealthProbeObservation,
    HealthStatus, HealthTarget, Ipv6EchoKind, Ipv6ProbeKind, Ipv6ProbeObservation, Ipv6Status,
    NetworkInterface, ProbeOutcome, Settings, DNS_FREE_IPV6_TARGET,
};
use futures_util::future::join_all;
use hyper_util::client::legacy::connect::HttpInfo;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant},
};
use tokio::{net, time};

const DNS_HOST: &str = "www.bit.edu.cn";
const BODY_LIMIT: usize = 16 * 1024;

pub(crate) async fn run(
    settings: &Settings,
    interface: Option<&NetworkInterface>,
    interfaces: &[NetworkInterface],
) -> HealthStatus {
    let targets = settings.configured_health_targets();
    let timeout = Duration::from_secs(settings.request_timeout_secs);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .ok();
    let health_probes = join_all(
        targets
            .iter()
            .enumerate()
            .map(|(index, target)| http_probe(client.as_ref(), index + 1, target, interface)),
    );
    let (health_probes, dns_probe, ipv6_probes) =
        tokio::join!(health_probes, dns_probe(), ipv6_observations(settings));
    let detail = |zh: &str, en: &str| localized(&settings.language, zh, en);
    let (internet, internet_detail) = if health_probes
        .iter()
        .any(|p| p.outcome == ProbeOutcome::Match)
    {
        (Health::Healthy, None)
    } else if health_probes.is_empty() {
        (
            Health::Unknown,
            detail("未配置探测地址", "No probe targets configured"),
        )
    } else if health_probes
        .iter()
        .all(|p| p.outcome == ProbeOutcome::InvalidTarget)
    {
        (
            Health::Unknown,
            detail("没有有效的探测地址", "No valid probe targets"),
        )
    } else {
        (
            Health::Degraded,
            detail("探测地址均未返回预期结果", "No probe met its expectation"),
        )
    };
    let (dns, dns_detail) = match dns_probe.result {
        DnsProbeResult::Ok => (Health::Healthy, None),
        DnsProbeResult::Timeout => (
            Health::Degraded,
            detail("DNS 查询超时", "DNS lookup timed out"),
        ),
        DnsProbeResult::Empty => (
            Health::Degraded,
            detail("DNS 未返回地址", "DNS returned no addresses"),
        ),
        DnsProbeResult::LookupFailed => (
            Health::Degraded,
            detail("DNS 查询失败", "DNS lookup failed"),
        ),
    };
    let mut health = HealthStatus {
        internet,
        dns,
        checked_at: Some(now_secs()),
        ipv6_checked_at: Some(now_secs()),
        health_probes,
        dns_probe: Some(dns_probe),
        ipv6_probes,
        internet_detail,
        dns_detail,
        ..HealthStatus::default()
    };
    update_ipv6_status(&mut health, interface, interfaces, &settings.language);
    health
}

pub(crate) async fn run_ipv6(
    settings: &Settings,
    interface: Option<&NetworkInterface>,
    interfaces: &[NetworkInterface],
    health: &mut HealthStatus,
) {
    health.ipv6_probes = ipv6_observations(settings).await;
    health.ipv6_checked_at = Some(now_secs());
    update_ipv6_status(health, interface, interfaces, &settings.language);
}

pub(crate) fn update_ipv6_status(
    health: &mut HealthStatus,
    interface: Option<&NetworkInterface>,
    interfaces: &[NetworkInterface],
    language: &str,
) {
    let addresses = interface
        .into_iter()
        .chain(interfaces.iter().filter(|_| interface.is_none()))
        .filter(|adapter| adapter.up && adapter.ipv6_enabled != Some(false))
        .flat_map(|adapter| adapter.ipv6.iter().copied());
    health.local_ipv6 = addresses
        .clone()
        .find(|address| is_cernet_ipv6(*address))
        .or_else(|| addresses.clone().find(|address| is_public_ipv6(*address)))
        .or_else(|| addresses.into_iter().find(Ipv6Addr::is_unique_local));
    health.public_ipv6 = health.confirmed_ipv6();
    health.ipv6_status = match interface {
        Some(i) if i.ipv6_enabled == Some(false) => Ipv6Status::Disabled,
        Some(i) if !i.ipv6.is_empty() && i.ipv6.iter().all(Ipv6Addr::is_unicast_link_local) => {
            Ipv6Status::LinkLocalOnly
        }
        _ => Ipv6Status::Unknown,
    };
    health.ipv6_status = health.observed_ipv6_status();
    health.ipv6 = match health.ipv6_status {
        Ipv6Status::Normal | Ipv6Status::GlobalNotPreferred => Health::Healthy,
        Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable => Health::Unavailable,
        Ipv6Status::Unknown | Ipv6Status::Disabled => Health::Unknown,
    };
    let (zh, en) = match health.ipv6_status {
        Ipv6Status::Normal if health.public_ipv6.is_some() => (
            "检测站点返回了公网 IPv6 地址",
            "A probe returned a public IPv6 address",
        ),
        Ipv6Status::Normal => (
            "已通过 IPv6 地址直连检测站点",
            "Reached the probe site directly over IPv6",
        ),
        Ipv6Status::GlobalNotPreferred => (
            "IPv6 可用，双栈站点回显了 IPv4",
            "IPv6 works. The dual-stack site reported IPv4.",
        ),
        Ipv6Status::GlobalUnreachable => (
            "未能连接到公网 IPv6 检测站点",
            "Could not reach a public IPv6 probe target",
        ),
        Ipv6Status::Disabled => (
            "所选网卡已关闭 IPv6",
            "IPv6 is disabled on the selected adapter",
        ),
        Ipv6Status::LinkLocalOnly => (
            "所选网卡只有链路本地 IPv6 地址",
            "The selected adapter has only link-local IPv6 addresses",
        ),
        Ipv6Status::Unknown if health.connectivity_ipv6_probes().next().is_none() => (
            "尚未测试公网 IPv6 连接",
            "Public IPv6 connectivity has not been tested",
        ),
        Ipv6Status::Unknown
            if health
                .connectivity_ipv6_probes()
                .any(|p| p.non_public_target) =>
        {
            (
                "有检测地址不可用，详情见各站点结果",
                "A probe address was unusable. See the site results.",
            )
        }
        Ipv6Status::Unknown => (
            "未测到有效的公网 IPv6 回显",
            "No valid public IPv6 echo received",
        ),
    };
    health.ipv6_detail = localized(language, zh, en);
}

fn localized(language: &str, zh: &str, en: &str) -> Option<String> {
    Some(if language == "zh-CN" { zh } else { en }.to_owned())
}

async fn http_probe(
    client: Option<&reqwest::Client>,
    index: usize,
    target: &HealthTarget,
    interface: Option<&NetworkInterface>,
) -> HealthProbeObservation {
    let started = Instant::now();
    let expect = match &target.expect {
        HealthExpect::AnySuccess => HealthProbeExpectation::AnySuccess,
        HealthExpect::Status(code) => HealthProbeExpectation::Status(*code),
        HealthExpect::BodyEquals(_) => HealthProbeExpectation::BodyEquals,
        HealthExpect::BodyContains(_) => HealthProbeExpectation::BodyContains,
    };
    let mut observation = HealthProbeObservation {
        target: index,
        expect,
        outcome: ProbeOutcome::InvalidTarget,
        status: None,
        body_match: None,
        used_ipv6: None,
        on_selected_interface: None,
        elapsed_ms: 0,
    };
    if let Ok(url) = target.parsed_url() {
        if let Some(client) = client {
            observe_http(client, url, &target.expect, interface, &mut observation).await;
        } else {
            observation.outcome = ProbeOutcome::RequestFailed;
        }
    }
    observation.elapsed_ms = elapsed_ms(started);
    observation
}

async fn observe_http(
    client: &reqwest::Client,
    url: url::Url,
    expect: &HealthExpect,
    interface: Option<&NetworkInterface>,
    observation: &mut HealthProbeObservation,
) {
    let mut response = match client.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            observation.outcome = request_outcome(error);
            return;
        }
    };
    let status = response.status();
    observation.status = Some(status.as_u16());
    if let Some(info) = response.extensions().get::<HttpInfo>() {
        let remote_address = info.remote_addr().ip().to_canonical();
        let local_address = info.local_addr().ip().to_canonical();
        observation.used_ipv6 = Some(remote_address.is_ipv6());
        observation.on_selected_interface = interface.map(|interface| match local_address {
            IpAddr::V4(address) => interface.ipv4.contains(&address),
            IpAddr::V6(address) => {
                interface.ipv6.contains(&address)
                    && is_public_ipv6(address)
                    && matches!(remote_address, IpAddr::V6(remote) if is_public_ipv6(remote))
            }
        });
    }
    let success = match expect {
        HealthExpect::AnySuccess => status.is_success(),
        HealthExpect::Status(expected) => *expected == status.as_u16(),
        _ => {
            if !status.is_success() {
                observation.outcome = ProbeOutcome::UnexpectedStatus;
                return;
            }
            match observe_body(&mut response, expect).await {
                Ok(matched) => observation.body_match = Some(matched),
                Err(outcome) => {
                    observation.outcome = outcome;
                    return;
                }
            }
            observation.outcome = if observation.body_match == Some(true) {
                ProbeOutcome::Match
            } else {
                ProbeOutcome::UnexpectedBody
            };
            return;
        }
    };
    observation.outcome = if success {
        ProbeOutcome::Match
    } else {
        ProbeOutcome::UnexpectedStatus
    };
}

async fn observe_body(
    response: &mut reqwest::Response,
    expect: &HealthExpect,
) -> Result<bool, ProbeOutcome> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(request_outcome)? {
        let previous_len = body.len();
        let admitted = chunk.len().min(BODY_LIMIT - previous_len);
        body.extend_from_slice(&chunk[..admitted]);
        match expect {
            HealthExpect::BodyEquals(expected) if !expected.as_bytes().starts_with(&body) => {
                return Ok(false);
            }
            HealthExpect::BodyContains(expected)
                if !expected.is_empty()
                    && body[previous_len.saturating_sub(expected.len() - 1)..]
                        .windows(expected.len())
                        .any(|window| window == expected.as_bytes()) =>
            {
                return Ok(true);
            }
            _ => {}
        }
        if admitted < chunk.len() {
            return Err(ProbeOutcome::BodyLimit);
        }
    }
    Ok(matches!(expect, HealthExpect::BodyEquals(expected) if body == expected.as_bytes()))
}

fn request_outcome(error: reqwest::Error) -> ProbeOutcome {
    if error.is_timeout() {
        ProbeOutcome::Timeout
    } else {
        ProbeOutcome::RequestFailed
    }
}

async fn dns_probe() -> DnsProbeObservation {
    let started = Instant::now();
    let (result, resolved_addresses) =
        match time::timeout(Duration::from_secs(3), net::lookup_host((DNS_HOST, 80))).await {
            Ok(Ok(addresses)) => {
                let addresses: Vec<_> = addresses
                    .map(|address| address.ip())
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .take(16)
                    .collect();
                let result = if addresses.is_empty() {
                    DnsProbeResult::Empty
                } else {
                    DnsProbeResult::Ok
                };
                (result, addresses)
            }
            Ok(Err(_)) => (DnsProbeResult::LookupFailed, Vec::new()),
            Err(_) => (DnsProbeResult::Timeout, Vec::new()),
        };
    DnsProbeObservation {
        host: DNS_HOST.to_owned(),
        result,
        addresses: resolved_addresses.len(),
        resolved_addresses,
        elapsed_ms: elapsed_ms(started),
    }
}

struct Ipv6Target {
    index: usize,
    url: url::Url,
    kind: Ipv6ProbeKind,
    /// Stable identifier of the configured education-network probe.
    probe_id: Option<String>,
}

fn ipv6_probe_targets(settings: &Settings) -> Vec<Ipv6Target> {
    let mut targets = Vec::new();
    let mut detailed = false;
    for (index, configured) in settings.ipv6_targets.iter().enumerate() {
        let Ok(url) = parse_ipv6_target(configured) else {
            continue;
        };
        let is_test_ipv6 = url
            .host_str()
            .is_some_and(|host| host.ends_with(".test-ipv6.com"))
            && url.path().trim_end_matches('/') == "/ip";
        targets.push(Ipv6Target {
            index: index + 1,
            url: url.clone(),
            kind: if is_test_ipv6 {
                Ipv6ProbeKind::TestIpv6
            } else {
                Ipv6ProbeKind::Address
            },
            probe_id: None,
        });
        if is_test_ipv6 && !detailed {
            detailed = true;
            let mut large_packet = url.clone();
            let parameters: Vec<_> = url
                .query_pairs()
                .into_owned()
                .filter(|(key, _)| key != "size" && key != "fill")
                .collect();
            large_packet
                .query_pairs_mut()
                .clear()
                .extend_pairs(parameters)
                .append_pair("size", "1600");
            targets.push(Ipv6Target {
                index: index + 1,
                url: large_packet,
                kind: Ipv6ProbeKind::LargePacket,
                probe_id: None,
            });
            targets.push(Ipv6Target {
                index: index + 1,
                url: url::Url::parse("http://ds.test-ipv6.com/ip/?callback=cb")
                    .expect("fixed dual stack URL"),
                kind: Ipv6ProbeKind::DualStack,
                probe_id: None,
            });
        }
    }
    for (index, probe) in settings.education_probes.iter().enumerate() {
        let Ok(url) = parse_ipv6_target(&probe.url) else {
            continue;
        };
        targets.push(Ipv6Target {
            index: index + 1,
            url,
            kind: Ipv6ProbeKind::Education,
            probe_id: Some(probe.id.clone()),
        });
    }
    targets
}

fn client(timeout: Duration) -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let target = attempt.url();
            if attempt.previous().len() == 1
                && same_host_https_upgrade(&attempt.previous()[0], target)
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .timeout(timeout)
        .user_agent(concat!("BitNet/", env!("CARGO_PKG_VERSION")))
}

fn same_host_https_upgrade(source: &url::Url, target: &url::Url) -> bool {
    source.scheme() == "http"
        && target.scheme() == "https"
        && source.host() == target.host()
        && source.port().is_none()
        && target.port().is_none()
        && target.username().is_empty()
        && target.password().is_none()
}

async fn ipv6_observations(settings: &Settings) -> Vec<Ipv6ProbeObservation> {
    let timeout = Duration::from_secs(settings.request_timeout_secs.min(8));
    join_all(
        ipv6_probe_targets(settings)
            .iter()
            .map(|target| ipv6_probe(target, timeout)),
    )
    .await
}

#[derive(Default)]
struct ProviderResponse {
    status: Option<u16>,
    reported: Option<IpAddr>,
    echo: Option<Ipv6EchoKind>,
    local_address: Option<IpAddr>,
    peer_address: Option<IpAddr>,
    literal_https_reachable: bool,
}

async fn provider_request(
    client: reqwest::ClientBuilder,
    url: url::Url,
    probe: Ipv6ProbeKind,
) -> Result<ProviderResponse, (ProviderResponse, ProbeOutcome)> {
    let mut result = ProviderResponse::default();
    let client = client
        .build()
        .map_err(|_| (ProviderResponse::default(), ProbeOutcome::RequestFailed))?;
    let callback = url
        .query_pairs()
        .find(|(key, _)| key == "callback")
        .map(|(_, value)| value.into_owned());
    let direct_https = url.as_str() == DNS_FREE_IPV6_TARGET;
    let mut request = client.get(url);
    if direct_https {
        request = request.header(reqwest::header::ACCEPT, "application/dns-message");
    }
    let mut response = request
        .send()
        .await
        .map_err(|error| (ProviderResponse::default(), request_outcome(error)))?;
    result.status = Some(response.status().as_u16());
    if let Some(info) = response.extensions().get::<HttpInfo>() {
        result.local_address = Some(info.local_addr().ip().to_canonical());
        result.peer_address = Some(info.remote_addr().ip().to_canonical());
    }
    if !response.status().is_success() {
        return Err((result, ProbeOutcome::UnexpectedStatus));
    }
    let dns_response = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .is_some_and(|header| header == "application/dns-message");
    let mut body = Vec::new();
    loop {
        let chunk = match response.chunk().await {
            Ok(chunk) => chunk,
            Err(e) => return Err((result, request_outcome(e))),
        };
        let Some(chunk) = chunk else { break };
        if body.len() + chunk.len() > BODY_LIMIT {
            return Err((result, ProbeOutcome::BodyLimit));
        }
        body.extend_from_slice(&chunk);
    }
    if direct_https {
        if dns_response && has_dns_reply_header(&body) {
            result.literal_https_reachable = true;
            return Ok(result);
        }
        return Err((result, ProbeOutcome::UnexpectedBody));
    }
    let reply = classify_echo(&body, callback.as_deref());
    result.echo = Some(reply.kind());
    result.reported = reply.address();
    if probe == Ipv6ProbeKind::Education {
        // A reachable measurement point is evidence on its own, but a challenge
        // page is not: it proves only that a gate answered.
        if reply.kind() == Ipv6EchoKind::Challenge {
            return Err((result, ProbeOutcome::UnexpectedBody));
        }
        return Ok(result);
    }
    let accepted = match probe {
        Ipv6ProbeKind::DualStack => {
            matches!(reply, EchoReply::Public(_))
                || matches!(reply, EchoReply::NonPublic(IpAddr::V4(_)))
        }
        _ => matches!(reply, EchoReply::Public(_)),
    };
    if !accepted {
        return Err((result, ProbeOutcome::UnexpectedBody));
    }
    if probe == Ipv6ProbeKind::LargePacket && body.len() < 1280 {
        return Err((result, ProbeOutcome::UnexpectedBody));
    }
    Ok(result)
}

fn has_dns_reply_header(body: &[u8]) -> bool {
    // A response to the fixed RFC 8484 GET uses the request's zero ID and sets QR.
    body.len() >= 12 && body[..2] == [0, 0] && body[2] & 0x80 != 0
}

/// What an echo endpoint's body contained. Only `Public` proves a public IPv6
/// address, and a challenge page is never treated as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EchoReply {
    Public(Ipv6Addr),
    NonPublic(IpAddr),
    Truncated,
    Invalid,
    Challenge,
}

impl EchoReply {
    fn kind(self) -> Ipv6EchoKind {
        match self {
            Self::Public(_) => Ipv6EchoKind::PublicAddress,
            Self::NonPublic(_) => Ipv6EchoKind::NonPublicAddress,
            Self::Truncated => Ipv6EchoKind::Truncated,
            Self::Invalid => Ipv6EchoKind::Invalid,
            Self::Challenge => Ipv6EchoKind::Challenge,
        }
    }

    fn address(self) -> Option<IpAddr> {
        match self {
            Self::Public(address) => Some(IpAddr::V6(address)),
            Self::NonPublic(address) => Some(address),
            Self::Truncated | Self::Invalid | Self::Challenge => None,
        }
    }
}

fn classify_echo(body: &[u8], callback: Option<&str>) -> EchoReply {
    let Ok(text) = std::str::from_utf8(body) else {
        return EchoReply::Invalid;
    };
    let text = text.trim().trim_start_matches('\u{feff}');
    if text.is_empty() {
        return EchoReply::Invalid;
    }
    if let Some(reply) = classify_echo_text(text) {
        return reply;
    }
    // Test IPv6's endpoint returns JSONP. Only the JSON payload is parsed as data.
    let json = match callback
        .and_then(|name| text.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix('('))
    {
        Some(payload) => payload
            .strip_suffix(");")
            .or_else(|| payload.strip_suffix(')'))
            .unwrap_or_default(),
        None => text,
    }
    .trim();
    if let Ok(reply) = serde_json::from_str::<serde_json::Value>(json) {
        let value = ["processedString", "ip"]
            .iter()
            .find_map(|key| reply.get(*key).and_then(serde_json::Value::as_str));
        return value
            .and_then(classify_echo_text)
            .unwrap_or(EchoReply::Invalid);
    }
    if looks_like_challenge(text) {
        EchoReply::Challenge
    } else if looks_truncated(text, json) {
        EchoReply::Truncated
    } else {
        EchoReply::Invalid
    }
}

/// Classifies a single plain token. `None` means the text is not an address at
/// all, so the caller can look for JSON or a challenge page instead.
fn classify_echo_text(value: &str) -> Option<EchoReply> {
    // Providers decorate the address with extra prose such as "… (CERNET)";
    // the address must still be the leading, complete token.
    let token = value.split_whitespace().next().unwrap_or_default();
    let token = token.split('%').next().unwrap_or_default();
    if let Ok(address) = token.parse::<IpAddr>() {
        return Some(match address {
            IpAddr::V6(address) if is_public_ipv6(address) => EchoReply::Public(address),
            address => EchoReply::NonPublic(address),
        });
    }
    (!token.is_empty() && looks_like_address_prefix(token)).then_some(EchoReply::Truncated)
}

/// An address-shaped fragment: hex digits and colons, and no complete address.
fn looks_like_address_prefix(token: &str) -> bool {
    token.contains(':')
        && token
            .chars()
            .all(|character| character.is_ascii_hexdigit() || matches!(character, ':' | '.'))
}

fn looks_like_challenge(text: &str) -> bool {
    const MARKERS: [&str; 9] = [
        "captcha",
        "recaptcha",
        "cf_chl",
        "cf-chl",
        "challenge-platform",
        "just a moment",
        "checking your browser",
        "enable javascript",
        "turnstile",
    ];
    let lower = text.to_ascii_lowercase();
    MARKERS.iter().any(|marker| lower.contains(marker))
}

/// Whether the body stopped before the payload its opening delimiter announced.
fn looks_truncated(text: &str, json: &str) -> bool {
    let unclosed = |value: &str| {
        (value.starts_with('{') || value.starts_with('['))
            && !(value.ends_with('}') || value.ends_with(']'))
    };
    unclosed(json) || (text.contains('(') && !text.trim_end().ends_with(')'))
}

fn probe_destinations(answers: &[IpAddr], from_dns: bool) -> Vec<SocketAddr> {
    answers
        .iter()
        .copied()
        .filter(|ip| match ip.to_canonical() {
            IpAddr::V6(ip) => is_public_ipv6(ip) || (from_dns && ip.is_unique_local()),
            IpAddr::V4(ip) => {
                from_dns
                    && !ip.is_unspecified()
                    && !ip.is_loopback()
                    && !ip.is_link_local()
                    && !ip.is_multicast()
                    && !ip.is_broadcast()
            }
        })
        // Let the system route fake IPs. The probe validates the site's response separately.
        // A port of zero leaves HTTP(S) port selection to the client.
        .map(|ip| SocketAddr::new(ip, 0))
        .collect()
}

async fn ipv6_probe(target: &Ipv6Target, timeout: Duration) -> Ipv6ProbeObservation {
    let started = Instant::now();
    let mut observation = Ipv6ProbeObservation {
        target: target.index,
        kind: target.kind,
        probe_id: target.probe_id.clone(),
        dns_result: None,
        addresses: None,
        resolved_addresses: Vec::new(),
        local_address: None,
        peer_address: None,
        non_public_target: false,
        reported_ipv4: false,
        literal_https_reachable: false,
        status: None,
        outcome: None,
        echo: None,
        reported_address: None,
        attempted: 0,
        succeeded: 0,
        local_is_v6: None,
        timed_out: false,
        elapsed_ms: 0,
    };
    let work = async {
        let host = target.url.host().expect("validated URL");
        let from_dns = matches!(host, url::Host::Domain(_));
        let answers = match host {
            url::Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
            url::Host::Ipv4(_) => Vec::new(),
            url::Host::Domain(name) => match net::lookup_host((name, 0)).await {
                Ok(addresses) => {
                    let answers: Vec<_> = addresses.map(|a| a.ip()).collect();
                    observation.dns_result = Some(if answers.is_empty() {
                        DnsProbeResult::Empty
                    } else {
                        DnsProbeResult::Ok
                    });
                    answers
                }
                Err(_) => {
                    observation.dns_result = Some(DnsProbeResult::LookupFailed);
                    return;
                }
            },
        };
        observation.addresses = Some(answers.len());
        observation
            .resolved_addresses
            .extend(answers.iter().take(8));
        let addresses = probe_destinations(&answers, from_dns);
        if addresses.is_empty() {
            observation.non_public_target = !answers.is_empty();
            return;
        }
        let mut builder = client(timeout);
        if let url::Host::Domain(name) = host {
            // Pin the checked destinations while leaving source and route selection to the OS.
            builder = builder.resolve_to_addrs(name, &addresses);
        }
        observation.attempted = 1;
        let (response, outcome) =
            match provider_request(builder, target.url.clone(), target.kind).await {
                Ok(response) => (response, ProbeOutcome::Match),
                Err(error) => error,
            };
        observation.status = response.status;
        observation.outcome = Some(outcome);
        observation.echo = response.echo;
        observation.local_address = response.local_address;
        observation.peer_address = response.peer_address;
        observation.local_is_v6 = response.local_address.map(|ip| ip.is_ipv6());
        observation.reported_ipv4 = matches!(response.reported, Some(IpAddr::V4(_)));
        observation.literal_https_reachable = response.literal_https_reachable;
        observation.timed_out = outcome == ProbeOutcome::Timeout;
        observation.succeeded = usize::from(outcome == ProbeOutcome::Match);
        observation.reported_address = match response.reported {
            Some(IpAddr::V6(address)) if is_public_ipv6(address) => Some(address),
            _ => None,
        };
    };
    if time::timeout(timeout, work).await.is_err() {
        observation.timed_out = true;
        observation.outcome = Some(ProbeOutcome::Timeout);
        observation.echo = Some(Ipv6EchoKind::Timeout);
        if observation.dns_result.is_none()
            && matches!(target.url.host(), Some(url::Host::Domain(_)))
        {
            observation.dns_result = Some(DnsProbeResult::Timeout);
        }
    }
    observation.elapsed_ms = elapsed_ms(started);
    observation
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn http_fixture(
        expect: HealthExpect,
        raw_response: &'static [u8],
    ) -> HealthProbeObservation {
        let listener = net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(raw_response).await.unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        let observation = http_probe(
            Some(&client),
            1,
            &HealthTarget {
                url: format!("http://{address}/?private=fixture"),
                expect,
            },
            None,
        )
        .await;
        server.await.unwrap();
        observation
    }

    #[tokio::test]
    async fn configured_body_and_status_expectations_use_direct_http_evidence() {
        let raw_body =
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nbody text";
        let observation = http_fixture(HealthExpect::BodyContains("text".into()), raw_body).await;
        assert_eq!(observation.outcome, ProbeOutcome::Match);
        assert_eq!(observation.status, Some(200));
        assert_eq!(observation.body_match, Some(true));
        assert_eq!(observation.used_ipv6, Some(false));
        let serialized = serde_json::to_string(&observation).unwrap();
        assert!(!serialized.contains("private="));
        assert!(!serialized.contains("text"));
        let observation = http_fixture(HealthExpect::BodyEquals("body".into()), raw_body).await;
        assert_eq!(observation.outcome, ProbeOutcome::UnexpectedBody);
        assert_eq!(observation.body_match, Some(false));

        let redirect = b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/unreachable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let observation = http_fixture(HealthExpect::Status(302), redirect).await;
        assert_eq!(observation.outcome, ProbeOutcome::Match);
        assert_eq!(observation.status, Some(302));
    }

    #[tokio::test]
    async fn incomplete_response_cannot_satisfy_exact_body_expectation() {
        let observation = http_fixture(
            HealthExpect::BodyEquals("body".into()),
            b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\nbody",
        )
        .await;
        assert_eq!(observation.status, Some(200));
        assert_eq!(observation.body_match, None);
        assert_eq!(observation.outcome, ProbeOutcome::RequestFailed);
    }

    async fn provider_fixture(
        bind: &str,
        body: &str,
        kind: Ipv6ProbeKind,
        path: &str,
    ) -> (ProviderResponse, ProbeOutcome) {
        let listener = net::TcpListener::bind(bind).await.unwrap();
        let address = listener.local_addr().unwrap();
        let raw = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            assert!(stream.read(&mut request).await.unwrap() > 0);
            stream.write_all(raw.as_bytes()).await.unwrap();
        });
        let url = url::Url::parse(&format!("http://{address}{path}")).unwrap();
        let response = provider_request(client(Duration::from_secs(3)), url, kind).await;
        server.await.unwrap();
        match response {
            Ok(response) => (response, ProbeOutcome::Match),
            Err(error) => error,
        }
    }

    #[tokio::test]
    async fn ipv6_echo_over_system_route_accepts_public_address_and_large_packet() {
        let public: IpAddr = "2001:4860::1234".parse().unwrap();
        let (result, outcome) =
            provider_fixture("[::1]:0", "2001:4860::1234", Ipv6ProbeKind::Address, "/ip").await;
        assert!(result.local_address.is_some_and(|ip| ip.is_ipv6()));
        assert_eq!(result.reported, Some(public));
        assert_eq!(outcome, ProbeOutcome::Match);
        let reply = format!(
            r#"cb({{"ip":"2001:4860::1234","fill":"{}"}});"#,
            "a".repeat(1600)
        );
        let (result, outcome) = provider_fixture(
            "[::1]:0",
            &reply,
            Ipv6ProbeKind::LargePacket,
            "/ip/?callback=cb",
        )
        .await;
        assert_eq!(result.reported, Some(public));
        assert_eq!(outcome, ProbeOutcome::Match);
        for (body, path, kind) in [
            (
                r#"cb({"ip":"2001:4860::1234"});"#,
                "/ip/?callback=cb",
                Ipv6ProbeKind::LargePacket,
            ),
            ("fe80::1", "/ip", Ipv6ProbeKind::Address),
            ("fdfe:dcba:9876::1", "/ip", Ipv6ProbeKind::Address),
        ] {
            let (_, outcome) = provider_fixture("[::1]:0", body, kind, path).await;
            assert_eq!(outcome, ProbeOutcome::UnexpectedBody);
        }
    }

    #[test]
    fn literal_https_requires_a_dns_reply() {
        assert!(!has_dns_reply_header(b"<html>Sign in</html>"));
        assert!(!has_dns_reply_header(&[0; 12]));
        assert!(!has_dns_reply_header(&[0, 0, 0x80]));
        assert!(has_dns_reply_header(&[
            0, 0, 0x81, 0x80, 0, 1, 0, 0, 0, 0, 0, 0
        ]));
    }

    #[test]
    fn processed_string_echoes_are_parsed_strictly() {
        let public = "2001:da8:204::1".parse::<Ipv6Addr>().unwrap();
        let cases: [(&str, EchoReply); 12] = [
            (
                r#"{"processedString":"2001:da8:204::1","organization":"CERNET"}"#,
                EchoReply::Public(public),
            ),
            (
                r#"{"processedString":"2001:da8:204::1 (CERNET)"}"#,
                EchoReply::Public(public),
            ),
            (
                r#"{"processedString":"10.0.0.8 (Campus)"}"#,
                EchoReply::NonPublic("10.0.0.8".parse().unwrap()),
            ),
            (
                r#"{"processedString":"fdfe:dcba:9876::1"}"#,
                EchoReply::NonPublic("fdfe:dcba:9876::1".parse().unwrap()),
            ),
            (
                r#"{"processedString":"2001:da8:204"}"#,
                EchoReply::Truncated,
            ),
            (
                r#"{"processedString":"2001:da8:zzzz::1"}"#,
                EchoReply::Invalid,
            ),
            (r#"{"processedString":""}"#, EchoReply::Invalid),
            (
                r#"<!DOCTYPE html><html><head><title>Just a moment...</title></head></html>"#,
                EchoReply::Challenge,
            ),
            (r#"{"ip":"2001:da8:204::1""#, EchoReply::Truncated),
            ("2001:da8:204::1", EchoReply::Public(public)),
            (
                r#"cb({"processedString":"2001:da8:204::1"});"#,
                EchoReply::Public(public),
            ),
            ("", EchoReply::Invalid),
        ];
        for (body, expected) in cases {
            let callback = body.contains("cb(").then_some("cb");
            assert_eq!(classify_echo(body.as_bytes(), callback), expected, "{body}");
        }
    }

    #[tokio::test]
    async fn challenge_pages_and_truncated_echoes_are_not_success() {
        for (body, echo, education_outcome) in [
            (
                "<html><body>Checking your browser. Enable JavaScript.</body></html>",
                Ipv6EchoKind::Challenge,
                ProbeOutcome::UnexpectedBody,
            ),
            (
                r#"{"processedString":"2001:da8:204"}"#,
                Ipv6EchoKind::Truncated,
                ProbeOutcome::Match,
            ),
            (
                r#"{"processedString":"10.0.0.8"}"#,
                Ipv6EchoKind::NonPublicAddress,
                ProbeOutcome::Match,
            ),
        ] {
            for (kind, expected) in [
                (Ipv6ProbeKind::Address, ProbeOutcome::UnexpectedBody),
                (Ipv6ProbeKind::Education, education_outcome),
            ] {
                let (result, outcome) = provider_fixture("127.0.0.1:0", body, kind, "/ip").await;
                assert_eq!(result.echo, Some(echo), "{body} {kind:?}");
                assert_eq!(outcome, expected, "{body} {kind:?}");
                assert_eq!(
                    result.reported.is_some(),
                    echo == Ipv6EchoKind::NonPublicAddress
                );
            }
        }
    }

    #[tokio::test]
    async fn education_measurement_points_accept_any_page_but_not_a_challenge() {
        let (result, outcome) = provider_fixture(
            "127.0.0.1:0",
            "<!DOCTYPE html><html><body>CERNET</body></html>",
            Ipv6ProbeKind::Education,
            "/",
        )
        .await;
        assert_eq!(outcome, ProbeOutcome::Match);
        assert_eq!(result.echo, Some(Ipv6EchoKind::Invalid));
        assert_eq!(result.reported, None);

        let (result, outcome) = provider_fixture(
            "127.0.0.1:0",
            r#"{"processedString":"2001:da8:204::1"}"#,
            Ipv6ProbeKind::Education,
            "/ip",
        )
        .await;
        assert_eq!(outcome, ProbeOutcome::Match);
        assert_eq!(
            result.reported,
            Some("2001:da8:204::1".parse::<IpAddr>().unwrap())
        );
        assert_eq!(result.echo, Some(Ipv6EchoKind::PublicAddress));
    }

    #[tokio::test]
    async fn unusable_literals_are_skipped_before_connecting() {
        for (url, non_public, family_count) in [
            ("http://[fdfe:dcba:9876::10]/ip", true, 1),
            ("http://[::1]/ip", true, 1),
            ("http://127.0.0.1/ip", false, 0),
        ] {
            let observation = ipv6_probe(
                &Ipv6Target {
                    index: 1,
                    url: parse_ipv6_target(url).unwrap(),
                    kind: Ipv6ProbeKind::Address,
                    probe_id: None,
                },
                Duration::from_secs(1),
            )
            .await;
            assert_eq!(observation.non_public_target, non_public);
            assert_eq!(observation.attempted, 0);
            assert_eq!(observation.addresses, Some(family_count));
            assert!(observation.dns_result.is_none()); // Literal targets never use DNS.
            assert!(!observation.timed_out);
        }
    }

    #[test]
    fn dns_answers_keep_routable_fake_ips_for_site_requests() {
        let fake = "fdfe:dcba:9876::12".parse().unwrap();
        let global = "2606:4700:4700::1111".parse().unwrap();
        let fake_v4 = "198.18.0.18".parse().unwrap();
        let local = "::1".parse().unwrap();
        let link_local = "fe80::1".parse().unwrap();
        assert_eq!(
            probe_destinations(&[fake], true),
            [SocketAddr::new(fake, 0)]
        );
        assert_eq!(
            probe_destinations(&[fake, fake_v4, local, global, link_local], true),
            [fake, fake_v4, global].map(|ip| SocketAddr::new(ip, 0))
        );
    }

    #[tokio::test]
    async fn site_can_echo_ipv6_over_an_ipv4_proxy_socket() {
        let (result, outcome) = provider_fixture(
            "127.0.0.1:0",
            r#"cb({"ip":"2001:4860::1234"});"#,
            Ipv6ProbeKind::DualStack,
            "/ip/?callback=cb",
        )
        .await;
        assert!(result.local_address.is_some_and(|ip| ip.is_ipv4()));
        assert_eq!(result.reported, Some("2001:4860::1234".parse().unwrap()));
        assert_eq!(outcome, ProbeOutcome::Match);
    }

    #[test]
    fn only_successful_public_echo_proves_connectivity() {
        let mut interface: NetworkInterface = serde_json::from_value(serde_json::json!({
            "id": "fixture",
            "name": "Fixture",
            "index": 1,
            "ipv4": [],
            "up": true,
            "wireless": false,
            "virtual_adapter": false,
            "ipv6_enabled": false
        }))
        .unwrap();
        let mut health = HealthStatus::default();
        update_ipv6_status(
            &mut health,
            Some(&interface),
            std::slice::from_ref(&interface),
            "en",
        );
        assert_eq!(health.ipv6_status, Ipv6Status::Disabled);

        interface.ipv6_enabled = Some(true);
        interface.ipv6 = vec!["fe80::1".parse().unwrap()];
        update_ipv6_status(
            &mut health,
            Some(&interface),
            std::slice::from_ref(&interface),
            "en",
        );
        assert_eq!(health.ipv6_status, Ipv6Status::LinkLocalOnly);

        let cernet = "2001:da8::1".parse().unwrap();
        interface.ipv6.push(cernet);
        // The detected CERNET address can belong to another network adapter.
        update_ipv6_status(&mut health, None, std::slice::from_ref(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Unknown);
        assert_eq!(health.local_ipv6, Some(cernet));
        assert_eq!(health.public_ipv6, None);

        let outside_cernet = "2001:4860::1234".parse().unwrap();
        let provider = serde_json::from_value(serde_json::json!({
            "target": 1,
            "attempted": 1,
            "succeeded": 1,
            "elapsed_ms": 1,
            "timed_out": false,
            "reported_address": outside_cernet
        }))
        .unwrap();
        health.ipv6_probes = vec![provider];
        update_ipv6_status(
            &mut health,
            Some(&interface),
            std::slice::from_ref(&interface),
            "en",
        );
        assert_eq!(health.ipv6_status, Ipv6Status::Normal);
        assert_eq!(health.local_ipv6, Some(cernet));
        assert_eq!(health.public_ipv6, Some(outside_cernet));

        let private_address = "fd00::1234".parse().unwrap();
        interface.ipv6 = vec![private_address];
        update_ipv6_status(
            &mut health,
            Some(&interface),
            std::slice::from_ref(&interface),
            "en",
        );
        assert_eq!(health.ipv6_status, Ipv6Status::Normal);
        assert_eq!(health.local_ipv6, Some(private_address));
        assert_eq!(health.public_ipv6, Some(outside_cernet));

        health.ipv6_probes.clear();
        update_ipv6_status(
            &mut health,
            Some(&interface),
            std::slice::from_ref(&interface),
            "en",
        );
        assert_eq!(health.ipv6_status, Ipv6Status::Unknown);
    }

    #[test]
    fn dedicated_defaults_produce_ipv6_only_and_detailed_test_ipv6_probes() {
        let settings = Settings::default();
        let targets = ipv6_probe_targets(&settings);
        let echo_targets = &targets[..5];
        assert_eq!(
            echo_targets
                .iter()
                .map(|target| target.kind)
                .collect::<Vec<_>>(),
            [
                Ipv6ProbeKind::Address,
                Ipv6ProbeKind::TestIpv6,
                Ipv6ProbeKind::LargePacket,
                Ipv6ProbeKind::DualStack,
                Ipv6ProbeKind::Address
            ]
        );
        assert_eq!(echo_targets[2].index, 2);
        assert!(echo_targets[2]
            .url
            .query_pairs()
            .any(|(key, value)| key == "size" && value == "1600"));
        assert_eq!(echo_targets[3].url.host_str(), Some("ds.test-ipv6.com"));
        assert!(same_host_https_upgrade(
            &echo_targets[0].url,
            &url::Url::parse("https://api-ipv6.ip.sb/ip").unwrap()
        ));
        assert!(!same_host_https_upgrade(
            &echo_targets[0].url,
            &url::Url::parse("https://example.com/ip").unwrap()
        ));
        // The education group keeps its stable identifiers and stays informational.
        assert_eq!(
            targets[5..]
                .iter()
                .map(|target| (target.kind, target.probe_id.as_deref()))
                .collect::<Vec<_>>(),
            [
                (Ipv6ProbeKind::Education, Some("cernet")),
                (Ipv6ProbeKind::Education, Some("cernet2")),
                (Ipv6ProbeKind::Education, Some("fiti")),
            ]
        );
    }
}
