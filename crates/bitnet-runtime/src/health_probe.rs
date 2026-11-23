use bitnet_core::{
    is_cernet_ipv6, is_public_ipv6, now_secs, parse_ipv6_target, DnsProbeObservation,
    DnsProbeResult, Health, HealthExpect, HealthProbeExpectation, HealthProbeObservation,
    HealthStatus, HealthTarget, Ipv6ProbeKind, Ipv6ProbeObservation, Ipv6Status, NetworkInterface,
    ProbeOutcome, Settings, DNS_FREE_IPV6_TARGET,
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
            "IPv6 可用，双栈检测使用了 IPv4",
            "IPv6 works. The dual-stack probe used IPv4.",
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
        Ipv6Status::Unknown if health.ipv6_probes.is_empty() => (
            "尚未测试公网 IPv6 连接",
            "Public IPv6 connectivity has not been tested",
        ),
        Ipv6Status::Unknown if health.ipv6_probes.iter().any(|p| p.non_public_target) => (
            "未测到真实 IPv6，检测目标包含非公网地址",
            "IPv6 is unconfirmed. A probe target had only non-public IPv6 addresses.",
        ),
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
}

fn ipv6_probe_targets(urls: &[String]) -> Vec<Ipv6Target> {
    let mut targets = Vec::new();
    let mut detailed = false;
    for (index, configured) in urls.iter().enumerate() {
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
            });
            targets.push(Ipv6Target {
                index: index + 1,
                url: url::Url::parse("http://ds.test-ipv6.com/ip/?callback=cb")
                    .expect("fixed dual stack URL"),
                kind: Ipv6ProbeKind::DualStack,
            });
        }
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
        ipv6_probe_targets(&settings.ipv6_targets)
            .iter()
            .map(|target| ipv6_probe(target, timeout)),
    )
    .await
}

#[derive(Default)]
struct ProviderResponse {
    status: Option<u16>,
    reported: Option<IpAddr>,
    local_is_v6: Option<bool>,
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
    result.local_is_v6 = response
        .extensions()
        .get::<HttpInfo>()
        .map(|info| info.local_addr().ip().to_canonical().is_ipv6());
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
    result.reported = parse_reported_ip(&body, callback.as_deref());
    if probe != Ipv6ProbeKind::DualStack
        && !matches!(result.reported, Some(IpAddr::V6(address)) if is_public_ipv6(address))
    {
        return Err((result, ProbeOutcome::UnexpectedBody));
    }
    if probe == Ipv6ProbeKind::DualStack && result.reported.is_none() {
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

fn parse_reported_ip(body: &[u8], callback: Option<&str>) -> Option<IpAddr> {
    let text = std::str::from_utf8(body).ok()?.trim();
    if let Ok(address) = text.parse() {
        return Some(address);
    }
    // Test IPv6's endpoint returns JSONP. Only parse the JSON payload as data.
    let json = match callback
        .and_then(|name| text.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix('('))
    {
        Some(payload) => payload
            .strip_suffix(");")
            .or_else(|| payload.strip_suffix(')'))?,
        None => text,
    };
    let reply: serde_json::Value = serde_json::from_str(json).ok()?;
    reply.get("ip")?.as_str()?.parse().ok()
}

fn public_probe_destinations(answers: &[IpAddr], dual_stack: bool) -> (Vec<SocketAddr>, bool) {
    let mut public_v6 = false;
    let mut non_public_v6 = false;
    let mut addresses = Vec::new();
    for ip in answers {
        match *ip {
            IpAddr::V6(address) if is_public_ipv6(address) => public_v6 = true,
            IpAddr::V6(_) => {
                non_public_v6 = true;
                continue;
            }
            IpAddr::V4(_) if !dual_stack => continue,
            IpAddr::V4(_) => {}
        }
        // Reqwest supplies the URL port, including on an HTTP to HTTPS upgrade.
        addresses.push(SocketAddr::new(*ip, 0));
    }
    (addresses, non_public_v6 && !public_v6)
}

async fn ipv6_probe(target: &Ipv6Target, timeout: Duration) -> Ipv6ProbeObservation {
    let started = Instant::now();
    let mut observation = Ipv6ProbeObservation {
        target: target.index,
        kind: target.kind,
        dns_result: None,
        addresses: None,
        non_public_target: false,
        literal_https_reachable: false,
        status: None,
        outcome: None,
        reported_address: None,
        attempted: 0,
        succeeded: 0,
        local_is_v6: None,
        timed_out: false,
        elapsed_ms: 0,
    };
    let work = async {
        let dual_stack = target.kind == Ipv6ProbeKind::DualStack;
        let host = target.url.host().expect("validated URL");
        let answers = match host {
            url::Host::Ipv6(ip) => vec![IpAddr::V6(ip)],
            url::Host::Ipv4(ip) => vec![IpAddr::V4(ip)],
            url::Host::Domain(name) => match net::lookup_host((name, 0)).await {
                Ok(addresses) => {
                    let answers: Vec<_> = addresses.map(|a| a.ip()).collect();
                    observation.dns_result =
                        Some(if answers.iter().any(|ip| dual_stack || ip.is_ipv6()) {
                            DnsProbeResult::Ok
                        } else {
                            DnsProbeResult::Empty
                        });
                    answers
                }
                Err(_) => {
                    observation.dns_result = Some(DnsProbeResult::LookupFailed);
                    return;
                }
            },
        };
        observation.addresses = Some(
            answers
                .iter()
                .filter(|ip| dual_stack || ip.is_ipv6())
                .count(),
        );
        let (addresses, non_public) = public_probe_destinations(&answers, dual_stack);
        observation.non_public_target = non_public;
        if non_public || addresses.is_empty() {
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
        observation.local_is_v6 = response.local_is_v6;
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

    async fn ipv6_fixture(
        body: &str,
        kind: Ipv6ProbeKind,
        path: &str,
    ) -> (ProviderResponse, ProbeOutcome) {
        let listener = net::TcpListener::bind("[::1]:0").await.unwrap();
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
            ipv6_fixture("2001:4860::1234", Ipv6ProbeKind::Address, "/ip").await;
        assert_eq!(result.local_is_v6, Some(true));
        assert_eq!(result.reported, Some(public));
        assert_eq!(outcome, ProbeOutcome::Match);
        let reply = format!(
            r#"cb({{"ip":"2001:4860::1234","fill":"{}"}});"#,
            "a".repeat(1600)
        );
        let (result, outcome) =
            ipv6_fixture(&reply, Ipv6ProbeKind::LargePacket, "/ip/?callback=cb").await;
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
            let (_, outcome) = ipv6_fixture(body, kind, path).await;
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

    #[tokio::test]
    async fn no_real_ipv6_target_is_skipped_before_connecting() {
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
        let fake = "fdfe:dcba:9876::12".parse().unwrap();
        let global = "2606:4700:4700::1111".parse().unwrap();
        let v4 = "1.1.1.1".parse().unwrap();
        assert_eq!(
            public_probe_destinations(&[fake, v4, global], false),
            (vec![SocketAddr::new(global, 0)], false)
        );
        assert!(public_probe_destinations(&[v4, fake], true).1);
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
        let targets = ipv6_probe_targets(&Settings::default().ipv6_targets);
        assert_eq!(
            targets.iter().map(|target| target.kind).collect::<Vec<_>>(),
            [
                Ipv6ProbeKind::Address,
                Ipv6ProbeKind::TestIpv6,
                Ipv6ProbeKind::LargePacket,
                Ipv6ProbeKind::DualStack,
                Ipv6ProbeKind::Address
            ]
        );
        assert_eq!(targets[2].index, 2);
        assert!(targets[2]
            .url
            .query_pairs()
            .any(|(key, value)| key == "size" && value == "1600"));
        assert_eq!(targets[3].url.host_str(), Some("ds.test-ipv6.com"));
        assert!(same_host_https_upgrade(
            &targets[0].url,
            &url::Url::parse("https://api-ipv6.ip.sb/ip").unwrap()
        ));
        assert!(!same_host_https_upgrade(
            &targets[0].url,
            &url::Url::parse("https://example.com/ip").unwrap()
        ));
    }
}
