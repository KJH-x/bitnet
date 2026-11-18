use bitnet_core::{
    is_public_ipv6, now_secs, DnsProbeObservation, DnsProbeResult, Health, HealthExpect,
    HealthProbeExpectation, HealthProbeObservation, HealthStatus, HealthTarget,
    Ipv6ProbeObservation, Ipv6Status, NetworkInterface, ProbeOutcome, Settings,
};
use futures_util::{future::join_all, stream, StreamExt};
use hyper_util::client::legacy::connect::HttpInfo;
use std::{
    net::{IpAddr, Ipv6Addr, SocketAddr},
    time::{Duration, Instant},
};
use tokio::{net, time};

const DNS_HOST: &str = "www.bit.edu.cn";
const BODY_LIMIT: usize = 16 * 1024;

pub(crate) async fn run(settings: &Settings, interface: Option<&NetworkInterface>) -> HealthStatus {
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
    let sources: Vec<_> = interface
        .into_iter()
        .flat_map(|interface| interface.ipv6.iter().copied())
        .filter(|addr| is_public_ipv6(*addr))
        .take(4)
        .collect();
    let ipv6_probes = join_all(
        targets
            .iter()
            .enumerate()
            .map(|(index, target)| ipv6_probe(target, index + 1, timeout, &sources)),
    );
    let (health_probes, dns_probe, ipv6_probes) =
        tokio::join!(health_probes, dns_probe(), ipv6_probes);
    let ipv6_probes: Vec<_> = ipv6_probes.into_iter().flatten().collect();
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
        health_probes,
        dns_probe: Some(dns_probe),
        ipv6_probes,
        internet_detail,
        dns_detail,
        ..HealthStatus::default()
    };
    update_ipv6_status(&mut health, interface, &settings.language);
    health
}

pub(crate) fn update_ipv6_status(
    health: &mut HealthStatus,
    interface: Option<&NetworkInterface>,
    language: &str,
) {
    let (status, zh, en) = ipv6_status(interface, &health.health_probes, &health.ipv6_probes);
    health.ipv6_status = status;
    health.ipv6 = match status {
        Ipv6Status::Normal | Ipv6Status::GlobalNotPreferred => Health::Healthy,
        Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable => Health::Unavailable,
        Ipv6Status::Unknown | Ipv6Status::Disabled => Health::Unknown,
    };
    health.ipv6_detail = localized(language, zh, en);
}

fn ipv6_status(
    interface: Option<&NetworkInterface>,
    http: &[HealthProbeObservation],
    tcp: &[Ipv6ProbeObservation],
) -> (Ipv6Status, &'static str, &'static str) {
    let Some(interface) = interface else {
        return (
            Ipv6Status::Unknown,
            "尚未选定网络接口",
            "No active network adapter",
        );
    };
    if interface.ipv6_enabled == Some(false) {
        return (Ipv6Status::Disabled, "已关闭 IPv6", "IPv6 is disabled");
    }
    if interface.ipv6.is_empty() {
        return (
            Ipv6Status::Unknown,
            "未获取到 IPv6 地址",
            "No IPv6 address available",
        );
    }
    if !interface.ipv6.iter().copied().any(is_public_ipv6) {
        return (
            Ipv6Status::LinkLocalOnly,
            "网卡只有本地 IPv6 地址",
            "The adapter has only local IPv6 addresses",
        );
    }
    let matching_http = |probe: &&HealthProbeObservation| {
        probe.outcome == ProbeOutcome::Match && probe.on_selected_interface == Some(true)
    };
    if http
        .iter()
        .filter(matching_http)
        .any(|probe| probe.used_ipv6 == Some(true))
    {
        return (
            Ipv6Status::Normal,
            "探测使用了 IPv6",
            "The internet probe used IPv6",
        );
    }
    let reachable = |probe: &&Ipv6ProbeObservation| probe.local_is_v6 == Some(true);
    if tcp.iter().filter(reachable).any(|probe| {
        http.iter().any(|http| {
            http.outcome == ProbeOutcome::Match
                && http.target == probe.target
                && http.used_ipv6 == Some(false)
        })
    }) {
        return (
            Ipv6Status::GlobalNotPreferred,
            "IPv6 可连接，探测实际使用了 IPv4",
            "IPv6 connected and the internet probe used IPv4",
        );
    }
    if tcp.iter().any(|probe| probe.local_is_v6 == Some(true)) {
        return (
            Ipv6Status::Unknown,
            "已测试 IPv6 连通，尚未确认实际使用的协议",
            "IPv6 connected. The protocol in use is unknown",
        );
    }
    if tcp
        .iter()
        .any(|probe| probe.attempted > 0 || probe.timed_out)
    {
        return (
            Ipv6Status::GlobalUnreachable,
            "未能建立 IPv6 连接",
            "Could not establish an IPv6 connection",
        );
    }
    if tcp
        .iter()
        .any(|probe| probe.dns_result == Some(DnsProbeResult::LookupFailed))
    {
        return (
            Ipv6Status::Unknown,
            "探测地址的 IPv6 DNS 查询失败",
            "The IPv6 DNS lookup for the probe targets failed",
        );
    }
    if !tcp.is_empty() && tcp.iter().all(|probe| probe.addresses == Some(0)) {
        return (
            Ipv6Status::Unknown,
            "探测地址未提供 IPv6 目标",
            "The probe targets returned no public IPv6 addresses",
        );
    }
    (
        Ipv6Status::Unknown,
        "尚未测试公网 IPv6 连接",
        "A public IPv6 connection has not been tested",
    )
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
    let (result, addresses) =
        match time::timeout(Duration::from_secs(3), net::lookup_host((DNS_HOST, 80))).await {
            Ok(Ok(addresses)) => {
                let count = addresses.count();
                (
                    if count == 0 {
                        DnsProbeResult::Empty
                    } else {
                        DnsProbeResult::Ok
                    },
                    count,
                )
            }
            Ok(Err(_)) => (DnsProbeResult::LookupFailed, 0),
            Err(_) => (DnsProbeResult::Timeout, 0),
        };
    DnsProbeObservation {
        host: DNS_HOST.to_owned(),
        result,
        addresses,
        elapsed_ms: elapsed_ms(started),
    }
}

async fn ipv6_probe(
    target: &HealthTarget,
    index: usize,
    timeout: Duration,
    sources: &[Ipv6Addr],
) -> Option<Ipv6ProbeObservation> {
    if sources.is_empty() {
        return None;
    }
    let url = target.parsed_url().ok()?;
    let host = url
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']');
    let port = url.port_or_known_default()?;
    let started = Instant::now();
    let mut observation = Ipv6ProbeObservation {
        target: index,
        dns_result: None,
        addresses: None,
        attempted: 0,
        succeeded: 0,
        local_is_v6: None,
        timed_out: false,
        elapsed_ms: 0,
    };
    let lookup_and_connect = async {
        let addresses = match net::lookup_host((host, port)).await {
            Ok(addresses) => addresses,
            Err(_) => {
                observation.dns_result = Some(DnsProbeResult::LookupFailed);
                return;
            }
        };
        let public_source = sources.iter().copied().all(is_public_ipv6);
        let addresses: Vec<_> = addresses
            .filter(|address| {
                matches!(address.ip(), IpAddr::V6(ip) if !public_source || is_public_ipv6(ip))
            })
            .take(2)
            .collect();
        observation.addresses = Some(addresses.len());
        observation.dns_result = Some(if addresses.is_empty() {
            DnsProbeResult::Empty
        } else {
            DnsProbeResult::Ok
        });
        observation.attempted = addresses.len() * sources.len();
        let probes = addresses.iter().flat_map(|address| {
            sources.iter().map(move |source| async move {
                let connect = async {
                    let socket = net::TcpSocket::new_v6()?;
                    socket.bind(SocketAddr::from((*source, 0)))?;
                    socket.connect(*address).await
                };
                time::timeout(Duration::from_secs(2), connect).await
            })
        });
        let mut results = stream::iter(probes).buffer_unordered(8);
        while let Some(result) = results.next().await {
            match result {
                Ok(Ok(stream)) => {
                    observation.succeeded += 1;
                    if let Ok(local_addr) = stream.local_addr() {
                        observation.local_is_v6 = Some(local_addr.is_ipv6());
                    }
                }
                Ok(Err(_)) => {}
                Err(_) => observation.timed_out = true,
            }
        }
    };
    if time::timeout(timeout.min(Duration::from_secs(5)), lookup_and_connect)
        .await
        .is_err()
    {
        observation.timed_out = true;
        if observation.dns_result.is_none() {
            observation.dns_result = Some(DnsProbeResult::Timeout);
        }
    }
    observation.elapsed_ms = elapsed_ms(started);
    Some(observation)
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

    #[tokio::test]
    async fn ipv6_diagnostic_proves_transport_on_a_loopback_fixture() {
        let Ok(listener) = net::TcpListener::bind("[::1]:0").await else {
            return;
        };
        let address = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move {
            listener.accept().await.unwrap();
        });
        let observation = ipv6_probe(
            &HealthTarget {
                url: format!("http://{address}"),
                expect: HealthExpect::AnySuccess,
            },
            1,
            Duration::from_secs(3),
            &[Ipv6Addr::LOCALHOST],
        )
        .await
        .unwrap();
        assert_eq!(observation.attempted, 1);
        assert_eq!(observation.dns_result, Some(DnsProbeResult::Ok));
        assert_eq!(observation.addresses, Some(1));
        assert_eq!(observation.succeeded, 1);
        assert_eq!(observation.local_is_v6, Some(true));
        accepted.await.unwrap();
    }

    #[tokio::test]
    async fn ipv6_uses_another_adapter_address_when_the_first_source_cannot_bind() {
        let Ok(listener) = net::TcpListener::bind("[::1]:0").await else {
            return;
        };
        let address = listener.local_addr().unwrap();
        let accepted = tokio::spawn(async move { listener.accept().await.unwrap() });
        let observation = ipv6_probe(
            &HealthTarget {
                url: format!("http://{address}"),
                expect: HealthExpect::AnySuccess,
            },
            1,
            Duration::from_secs(3),
            &["::2".parse().unwrap(), Ipv6Addr::LOCALHOST],
        )
        .await
        .unwrap();
        assert_eq!(observation.attempted, 2);
        assert_eq!(observation.succeeded, 1);
        assert_eq!(observation.local_is_v6, Some(true));
        accepted.await.unwrap();
    }

    #[tokio::test]
    async fn an_ipv4_only_target_records_a_completed_empty_ipv6_lookup() {
        let observation = ipv6_probe(
            &HealthTarget {
                url: "http://127.0.0.1:1/".into(),
                expect: HealthExpect::AnySuccess,
            },
            1,
            Duration::from_secs(1),
            &[Ipv6Addr::LOCALHOST],
        )
        .await
        .unwrap();
        assert_eq!(observation.dns_result, Some(DnsProbeResult::Empty));
        assert_eq!(observation.addresses, Some(0));
        assert_eq!(observation.attempted, 0);
        assert!(!observation.timed_out);
    }

    #[test]
    fn ipv6_status_requires_reachability_and_connection_family_evidence() {
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
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Disabled);

        interface.ipv6_enabled = Some(true);
        interface.ipv6 = vec!["fe80::1".parse().unwrap()];
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::LinkLocalOnly);

        interface.ipv6.push("2600::1".parse().unwrap());
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Unknown);

        health.ipv6_probes.push(Ipv6ProbeObservation {
            target: 1,
            dns_result: Some(DnsProbeResult::Ok),
            addresses: Some(1),
            attempted: 1,
            succeeded: 0,
            local_is_v6: None,
            timed_out: true,
            elapsed_ms: 1,
        });
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::GlobalUnreachable);

        health.ipv6_probes[0].attempted = 0;
        health.ipv6_probes[0].dns_result = Some(DnsProbeResult::Timeout);
        health.ipv6_probes[0].addresses = None;
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::GlobalUnreachable);

        health.ipv6_probes[0].timed_out = false;
        health.ipv6_probes[0].dns_result = Some(DnsProbeResult::Empty);
        health.ipv6_probes[0].addresses = Some(0);
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Unknown);
        assert!(health
            .ipv6_detail
            .as_deref()
            .unwrap()
            .contains("no public IPv6"));

        health.ipv6_probes[0].local_is_v6 = Some(true);
        health.ipv6_probes[0].succeeded = 1;
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Unknown);

        health.health_probes.push(HealthProbeObservation {
            target: 1,
            expect: HealthProbeExpectation::Status(204),
            outcome: ProbeOutcome::Match,
            status: Some(204),
            body_match: None,
            used_ipv6: Some(false),
            on_selected_interface: Some(false),
            elapsed_ms: 1,
        });
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::GlobalNotPreferred);

        health.health_probes[0].on_selected_interface = Some(true);
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::GlobalNotPreferred);

        health.health_probes[0].used_ipv6 = Some(true);
        update_ipv6_status(&mut health, Some(&interface), "en");
        assert_eq!(health.ipv6_status, Ipv6Status::Normal);
    }
}
