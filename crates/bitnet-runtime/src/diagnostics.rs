//! On-demand diagnostics in the interactive user's context.
use bitnet_core::{AppError, AuthState, ErrorCode, Health, Scope, Snapshot};
use futures_util::{stream, StreamExt};
use serde::Deserialize;
use std::{
    fmt::Write as _,
    io::Read as _,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

type Rows = Vec<Vec<String>>;

const DNS_NAME: &str = "www.bit.edu.cn";
const CAMPUS_DNS: [Ipv4Addr; 2] = [Ipv4Addr::new(10, 0, 32, 32), Ipv4Addr::new(10, 0, 32, 33)];
#[cfg(not(windows))]
const ROUTE_TARGETS: [&str; 6] = [
    "223.5.5.5",
    "2400:3200::1",
    "10.0.0.55",
    "10.0.0.54",
    "10.0.32.32",
    "10.0.32.33",
];
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const OUTPUT_LIMIT: usize = 512 * 1024;

#[derive(Debug, Clone)]
pub struct Report {
    pub findings: Vec<String>,
    pub text: String,
    html: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct NativeEvidence {
    adapters: Rows,
    routes: Rows,
    egress: Rows,
    proxies: Rows,
    processes: Rows,
    sockets: Rows,
    relations: Rows,
    interception: Rows,
    reachability: Rows,
    controllers: Rows,
    unavailable: Vec<String>,
}

fn t(english: bool, zh: &'static str, en: &'static str) -> &'static str {
    if english {
        en
    } else {
        zh
    }
}

fn unavailable(english: bool, zh: &'static str, en: &'static str) -> AppError {
    AppError::new(ErrorCode::Unavailable, t(english, zh, en))
}

pub async fn collect(snapshot: Snapshot, english: bool) -> Report {
    let simulation = snapshot
        .capabilities
        .iter()
        .any(|item| item == "simulation");
    let (native, dns, probes) = if simulation {
        (NativeEvidence::default(), vec![], vec![])
    } else {
        let native = tokio::task::spawn_blocking(native_evidence);
        let (native, dns, probes) = tokio::join!(native, check_dns(&snapshot), check_targets());
        (
            native.unwrap_or_else(|_| NativeEvidence {
                unavailable: vec!["operating-system evidence".into()],
                ..NativeEvidence::default()
            }),
            dns,
            probes,
        )
    };
    let controller = if simulation {
        vec![]
    } else {
        read_controller_chains(&native.controllers).await
    };
    render(
        snapshot, native, dns, probes, controller, english, simulation,
    )
}

pub async fn open_in_browser(
    report: std::sync::Arc<Report>,
    english: bool,
) -> Result<(), AppError> {
    let failure = || unavailable(english, "未能打开浏览器报告", "Could not open the report");
    tokio::task::spawn_blocking(move || {
        let directory = bitnet_platform::paths(Scope::User)?
            .data_dir
            .join("diagnostics");
        bitnet_platform::prepare_private_directory(&directory)?;
        let file = directory.join("latest.html");
        bitnet_platform::write_private_file(&file, report.html.as_bytes())?;
        let uri = url::Url::from_file_path(&file)
            .map_err(|_| unavailable(english, "报告路径有误", "The report path is invalid"))?;
        if launch_browser(uri.as_str()) {
            Ok(())
        } else {
            Err(unavailable(
                english,
                "未能打开浏览器报告",
                "Could not open the report",
            ))
        }
    })
    .await
    .map_err(|_| failure())?
}

#[cfg(windows)]
fn launch_browser(uri: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let target: Vec<u16> = uri.encode_utf16().chain([0]).collect();
    let operation: Vec<u16> = "open".encode_utf16().chain([0]).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            operation.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
        )
    };
    result as isize > 32
}

#[cfg(not(windows))]
fn launch_browser(uri: &str) -> bool {
    #[cfg(target_os = "macos")]
    let program = "/usr/bin/open";
    #[cfg(not(target_os = "macos"))]
    let program = "/usr/bin/xdg-open";
    Command::new(program)
        .arg(uri)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn capture(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut process = Command::new(program);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        process.creation_flags(0x0800_0000);
    }
    let mut child = process.spawn().ok()?;
    let pipe = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        pipe.take(OUTPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        (bytes.len() <= OUTPUT_LIMIT).then(|| String::from_utf8_lossy(&bytes).into_owned())
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return reader.join().ok().flatten(),
            Ok(None) if started.elapsed() < timeout => {
                std::thread::sleep(Duration::from_millis(30))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
}

#[cfg(windows)]
fn native_evidence() -> NativeEvidence {
    use base64::Engine as _;
    let collector = include_str!("diagnostics_windows.ps1");
    let bytes: Vec<u8> = collector
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    let command = base64::engine::general_purpose::STANDARD.encode(bytes);
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .map(|root| root.join("System32/WindowsPowerShell/v1.0/powershell.exe"))
        .filter(|path| path.is_absolute())
        .and_then(|path| {
            capture(
                &path,
                &[
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-EncodedCommand",
                    &command,
                ],
                Duration::from_secs(18),
            )
        })
        .and_then(|output| serde_json::from_str(output.trim()).ok())
        .unwrap_or_else(|| NativeEvidence {
            unavailable: vec![
                "Windows adapters, routes, proxy settings, processes and sockets".into(),
            ],
            ..NativeEvidence::default()
        })
}

#[cfg(not(windows))]
fn native_evidence() -> NativeEvidence {
    let seconds = Duration::from_secs(2);
    let mut evidence = NativeEvidence::default();
    #[cfg(target_os = "linux")]
    {
        for family in ["-4", "-6"] {
            if let Some(output) = capture(
                Path::new("/usr/sbin/ip"),
                &[family, "route", "show", "default"],
                seconds,
            ) {
                evidence.routes.extend(
                    output
                        .lines()
                        .take(96)
                        .map(|line| vec![family.into(), line.to_owned()]),
                );
            } else {
                evidence.unavailable.push(format!("{family} routes"));
            }
        }
        if let Some(output) = capture(Path::new("/usr/bin/ss"), &["-H", "-tuna", "-p"], seconds) {
            evidence
                .sockets
                .extend(output.lines().take(200).map(|line| vec![line.to_owned()]));
        } else {
            evidence.unavailable.push("sockets".into());
        }
        for target in ROUTE_TARGETS {
            if let Some(output) = capture(
                Path::new("/usr/sbin/ip"),
                &["route", "get", target],
                seconds,
            ) {
                let mut row = vec![
                    target.into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                ];
                for pair in output.split_whitespace().collect::<Vec<_>>().windows(2) {
                    let column = match pair[0] {
                        "dev" => 3,
                        "src" => 4,
                        "via" => 5,
                        _ => continue,
                    };
                    row[column] = pair[1].into();
                }
                evidence.egress.push(row);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        for family in ["inet", "inet6"] {
            if let Some(output) = capture(
                Path::new("/usr/sbin/netstat"),
                &["-rn", "-f", family],
                seconds,
            ) {
                evidence.routes.extend(
                    output
                        .lines()
                        .filter(|line| line.starts_with("default"))
                        .take(96)
                        .map(|line| vec![family.into(), line.to_owned()]),
                );
            } else {
                evidence.unavailable.push(format!("{family} routes"));
            }
        }
        if let Some(output) = capture(
            Path::new("/usr/sbin/netstat"),
            &["-an", "-p", "tcp"],
            seconds,
        ) {
            evidence.sockets.extend(
                output
                    .lines()
                    .filter(|line| line.contains("LISTEN") || line.contains("ESTABLISHED"))
                    .take(200)
                    .map(|line| vec![line.to_owned()]),
            );
        } else {
            evidence.unavailable.push("sockets".into());
        }
        for target in ROUTE_TARGETS {
            let family = if target.contains(':') {
                "-inet6"
            } else {
                "-inet"
            };
            if let Some(output) = capture(
                Path::new("/sbin/route"),
                &["-n", "get", family, target],
                seconds,
            ) {
                let mut row = vec![
                    target.into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                    "—".into(),
                ];
                for (field, value) in output
                    .lines()
                    .filter_map(|line| line.trim().split_once(':'))
                {
                    let column = match field {
                        "destination" => 1,
                        "interface" => 3,
                        "gateway" => 5,
                        _ => continue,
                    };
                    row[column] = value.trim().into();
                }
                evidence.egress.push(row);
            }
        }
    }
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        if let Ok(value) = std::env::var(key) {
            let endpoint = url::Url::parse(&value)
                .ok()
                .and_then(|parsed| {
                    Some(format!(
                        "{}:{}",
                        parsed.host_str()?,
                        parsed.port_or_known_default()?
                    ))
                })
                .unwrap_or_else(|| "configured".into());
            evidence
                .proxies
                .push(vec!["environment".into(), key.into(), endpoint]);
        }
    }
    evidence
}

async fn check_targets() -> Rows {
    const TARGETS: [(&str, &str, bool); 4] = [
        ("公网 IPv4 / Public IPv4", "223.5.5.5:53", false),
        ("公网 IPv6 / Public IPv6", "[2400:3200::1]:53", false),
        ("校园门户 / Campus portal", "10.0.0.55:80", true),
        ("校园接口 / Campus API", "10.0.0.54:8800", true),
    ];
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(PROBE_TIMEOUT)
        .build()
        .ok();
    let requests: Vec<_> = TARGETS
        .into_iter()
        .map(|(purpose, endpoint, http)| check_target(purpose, endpoint, http, client.as_ref()))
        .collect();
    stream::iter(requests)
        .buffered(TARGETS.len())
        .collect()
        .await
}

async fn check_target(
    purpose: &str,
    endpoint: &str,
    http: bool,
    client: Option<&reqwest::Client>,
) -> Vec<String> {
    let started = Instant::now();
    let result =
        tokio::time::timeout(PROBE_TIMEOUT, tokio::net::TcpStream::connect(endpoint)).await;
    let (status, source) = match result {
        Ok(Ok(socket)) => (
            "connected",
            socket
                .local_addr()
                .ok()
                .map(|address| address.ip().to_string()),
        ),
        Ok(Err(_)) => ("unreachable", None),
        Err(_) => ("timeout", None),
    };
    let elapsed = format!("{} ms", started.elapsed().as_millis());
    let http_result = match (http, status, client) {
        (true, "connected", Some(client)) => {
            match client.get(format!("http://{endpoint}/")).send().await {
                Ok(reply) => format!(
                    "HTTP {} · {} ms",
                    reply.status().as_u16(),
                    started.elapsed().as_millis()
                ),
                Err(_) => "unavailable".into(),
            }
        }
        _ => "—".into(),
    };
    vec![
        purpose.into(),
        endpoint.into(),
        status.into(),
        source.unwrap_or_else(|| "—".into()),
        elapsed,
        http_result,
    ]
}

async fn check_dns(snapshot: &Snapshot) -> Rows {
    let mut addresses: Vec<IpAddr> = CAMPUS_DNS.into_iter().map(IpAddr::from).collect();
    for adapter in &snapshot.interfaces {
        let servers = adapter.dns_config.as_ref().map_or_else(
            || adapter.dns.iter().copied().map(IpAddr::from).collect(),
            |config| config.servers.clone(),
        );
        for server in servers {
            if !addresses.contains(&server) && addresses.len() < 12 {
                addresses.push(server);
            }
        }
    }
    stream::iter(addresses)
        .map(|ip| async move { check_resolver(ip).await })
        .buffered(8)
        .collect()
        .await
}

async fn check_resolver(ip: IpAddr) -> Vec<String> {
    let started = Instant::now();
    let query = async {
        let bind_address = match ip {
            IpAddr::V4(_) => SocketAddr::from(([0, 0, 0, 0], 0)),
            IpAddr::V6(_) => SocketAddr::from(([0; 8], 0)),
        };
        let socket = tokio::net::UdpSocket::bind(bind_address).await?;
        socket.connect(SocketAddr::new(ip, 53)).await?;
        // Each probe owns an ephemeral connected socket; it never accepts datagrams from other resolvers.
        let mut packet = vec![0x42, 0x4e, 0x01, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in DNS_NAME.split('.') {
            packet.push(label.len() as u8);
            packet.extend_from_slice(label.as_bytes());
        }
        packet.extend_from_slice(&[0, 0, 1, 0, 1]);
        socket.send(&packet).await?;
        let mut reply = [0u8; 4096];
        let count = socket.recv(&mut reply).await?;
        Ok::<_, std::io::Error>(dns_response(&reply[..count]))
    };
    let result = match tokio::time::timeout(PROBE_TIMEOUT, query).await {
        Ok(Ok(Some((rcode, answers)))) => format!("RCODE {rcode} · {answers} answers"),
        Ok(Ok(None)) => "invalid response".into(),
        Ok(Err(_)) => "unavailable".into(),
        Err(_) => "timeout".into(),
    };
    vec![
        ip.to_string(),
        DNS_NAME.into(),
        result,
        format!("{} ms", started.elapsed().as_millis()),
    ]
}

fn dns_response(reply: &[u8]) -> Option<(u8, u16)> {
    (reply.len() >= 12 && reply[..2] == [0x42, 0x4e] && reply[2] & 0x80 != 0)
        .then(|| (reply[3] & 0x0f, u16::from_be_bytes([reply[6], reply[7]])))
}

async fn read_controller_chains(controllers: &Rows) -> Rows {
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_millis(800))
        .build()
    else {
        return vec![];
    };
    let client = &client;
    let requests: Vec<_> = controllers
        .iter()
        .take(4)
        .filter_map(|row| {
            let [host, port, owner] = row.as_slice() else {
                return None;
            };
            let endpoint = match (host.parse::<IpAddr>(), port.parse::<u16>()) {
                (Ok(host), Ok(port @ (9090 | 9097))) if host.is_loopback() => {
                    SocketAddr::new(host, port)
                }
                _ => return None,
            };
            Some((endpoint, owner.clone()))
        })
        .map(|(endpoint, owner)| async move {
            let url = format!("http://{endpoint}/connections");
            let reply = client.get(url).send().await;
            let Ok(mut reply) = reply else {
                return vec![vec![
                    owner.clone(),
                    endpoint.to_string(),
                    "unavailable".into(),
                    "—".into(),
                ]];
            };
            if !reply.status().is_success() {
                return vec![vec![
                    owner.clone(),
                    endpoint.to_string(),
                    format!("HTTP {}", reply.status().as_u16()),
                    "—".into(),
                ]];
            }
            let mut bytes = Vec::new();
            while let Ok(Some(chunk)) = reply.chunk().await {
                if bytes.len() + chunk.len() > OUTPUT_LIMIT {
                    return vec![];
                }
                bytes.extend_from_slice(&chunk);
            }
            let Ok(payload) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                return vec![];
            };
            let Some(connections) = payload.get("connections").and_then(|data| data.as_array())
            else {
                return vec![];
            };
            connections
                .iter()
                .take(128)
                .map(|connection| {
                    let rule = connection["rule"]
                        .as_str()
                        .unwrap_or("—")
                        .chars()
                        .take(128)
                        .collect();
                    let chain = connection["chains"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(8)
                        .filter_map(|name| name.as_str())
                        .map(|name| name.chars().take(64).collect::<String>())
                        .collect::<Vec<_>>()
                        .join(" → ");
                    vec![owner.clone(), endpoint.to_string(), rule, chain]
                })
                .collect()
        })
        .collect();
    stream::iter(requests)
        .buffered(4)
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .flatten()
        .collect()
}

struct Section {
    id: &'static str,
    title: &'static str,
    columns: &'static [&'static str],
    rows: Rows,
}

fn snapshot_adapters(snapshot: &Snapshot) -> Rows {
    let joined = |values: Vec<String>| {
        if values.is_empty() {
            "—".into()
        } else {
            values.join(", ")
        }
    };
    snapshot
        .interfaces
        .iter()
        .map(|adapter| {
            vec![
                adapter.index.to_string(),
                adapter.name.clone(),
                if adapter.up {
                    "up".into()
                } else {
                    "down".into()
                },
                if adapter.virtual_adapter {
                    "virtual".into()
                } else {
                    "physical".into()
                },
                "—".into(),
                "—".into(),
                joined(adapter.ipv4.iter().map(ToString::to_string).collect()),
                joined(adapter.ipv6.iter().map(ToString::to_string).collect()),
                adapter
                    .gateway
                    .map(|ip| ip.to_string())
                    .unwrap_or_else(|| "—".into()),
                joined(adapter.dns_config.as_ref().map_or_else(
                    || adapter.dns.iter().map(ToString::to_string).collect(),
                    |config| config.servers.iter().map(ToString::to_string).collect(),
                )),
            ]
        })
        .collect()
}

fn render(
    snapshot: Snapshot,
    mut native: NativeEvidence,
    dns: Rows,
    probes: Rows,
    controller: Rows,
    english: bool,
    simulation: bool,
) -> Report {
    let translate = |zh, en| t(english, zh, en);
    let mut findings = Vec::new();
    if simulation {
        findings.push(
            translate(
                "模拟模式，使用模拟网络数据。",
                "Simulation mode. The report uses simulated network data.",
            )
            .into(),
        );
    }
    if snapshot.active_binding.is_none() {
        findings.push(
            translate(
                "未找到校园网出口。",
                "No campus network adapter is selected.",
            )
            .into(),
        );
    }
    if snapshot.session.auth != AuthState::Authenticated {
        findings.push(
            translate(
                "当前校园账号尚未通过认证。",
                "Campus authentication has not been confirmed.",
            )
            .into(),
        );
    }
    if probes
        .iter()
        .skip(2)
        .all(|row| row.get(2).is_some_and(|status| status != "connected"))
        && !probes.is_empty()
    {
        findings.push(
            translate(
                "校园门户和校园接口均无法连接。",
                "Neither campus service responded to the TCP check.",
            )
            .into(),
        );
    }
    let source_mismatches = probes
        .iter()
        .filter_map(|row| {
            let address = row.get(1)?.parse::<SocketAddr>().ok()?.ip().to_string();
            let observed = row.get(3)?.parse::<IpAddr>().ok()?;
            let route = native
                .egress
                .iter()
                .find(|entry| entry.first() == Some(&address))?;
            let expected = route.get(4)?.parse::<IpAddr>().ok()?;
            (observed != expected)
                .then(|| vec![address, expected.to_string(), observed.to_string()])
        })
        .collect::<Rows>();
    if !source_mismatches.is_empty() {
        findings.push(
            translate(
                "有探测连接使用了与路由预测不同的源地址。",
                "An observed source address differs from the routing prediction.",
            )
            .into(),
        );
    }
    native
        .interception
        .extend(source_mismatches.into_iter().map(|row| {
            vec![
                "Route source mismatch".into(),
                row[0].clone(),
                format!("{} → {}", row[1], row[2]),
            ]
        }));
    for detail in [
        &snapshot.health.internet_detail,
        &snapshot.health.dns_detail,
        &snapshot.health.ipv6_detail,
    ] {
        if let Some(detail) = detail.as_ref().filter(|text| !text.is_empty()) {
            findings.push(detail.clone());
        }
    }
    if !native.unavailable.is_empty() {
        findings.push(
            translate(
                "有些系统证据无法读取。",
                "Some operating-system evidence was unavailable.",
            )
            .into(),
        );
    }
    if findings.is_empty() {
        findings.push(translate("未发现需要处理的问题。", "No issues were found.").into());
    }

    let check_time = snapshot
        .health
        .checked_at
        .map_or_else(|| "—".into(), |time| format!("{time} (Unix UTC)"));
    let ipv6_time = snapshot
        .health
        .ipv6_checked_at
        .map_or_else(|| "—".into(), |time| format!("{time} (Unix UTC)"));
    let mut checks = vec![
        vec![
            "Internet".into(),
            health_label(&snapshot.health.internet).into(),
            check_time.clone(),
        ],
        vec![
            "DNS".into(),
            health_label(&snapshot.health.dns).into(),
            check_time.clone(),
        ],
        vec![
            "IPv6".into(),
            format!(
                "{:?} · local {} · public {}",
                snapshot.health.ipv6_status,
                snapshot
                    .health
                    .local_ipv6
                    .map_or_else(|| "—".into(), |ip| ip.to_string()),
                snapshot
                    .health
                    .public_ipv6
                    .map_or_else(|| "—".into(), |ip| ip.to_string()),
            ),
            ipv6_time.clone(),
        ],
    ];
    checks.extend(snapshot.health.health_probes.iter().map(|probe| {
        vec![
            format!("HTTP #{}", probe.target),
            format!(
                "{:?} · status {} · {} ms",
                probe.outcome,
                probe
                    .status
                    .map_or_else(|| "—".into(), |value| value.to_string()),
                probe.elapsed_ms
            ),
            check_time.clone(),
        ]
    }));
    checks.extend(snapshot.health.ipv6_probes.iter().map(|probe| {
        vec![
            format!("IPv6 {:?} #{}", probe.kind, probe.target),
            format!(
                "{:?} · status {} · address {} · {}/{} connections · {} ms",
                probe.outcome,
                probe
                    .status
                    .map_or_else(|| "—".into(), |value| value.to_string()),
                probe
                    .reported_address
                    .map_or_else(|| "—".into(), |ip| ip.to_string()),
                probe.succeeded,
                probe.attempted,
                probe.elapsed_ms,
            ),
            ipv6_time.clone(),
        ]
    }));
    if let Some(probe) = &snapshot.health.dns_probe {
        checks.push(vec![
            format!("DNS {}", probe.host),
            format!(
                "{:?} · {} answers · {} ms",
                probe.result, probe.addresses, probe.elapsed_ms
            ),
            check_time.clone(),
        ]);
    }
    let active = snapshot
        .active_binding
        .as_ref()
        .map(|binding| {
            vec![
                translate("校园认证", "Campus authentication").into(),
                format!("ifIndex {}", binding.interface_index),
                binding.source_ip.to_string(),
            ]
        })
        .into_iter()
        .collect();
    if native.adapters.is_empty() {
        native.adapters = snapshot_adapters(&snapshot);
    }
    let routes = native.egress.clone();
    let adapters = native
        .adapters
        .iter()
        .map(|row| row.get(1).cloned())
        .collect::<Vec<_>>();
    let relationship_count = native.relations.len();
    let socket_count = native.sockets.len();
    let sections = [
        Section {
            id: "campus",
            title: translate("校园认证路径", "Campus authentication path"),
            columns: &["Path", "Adapter", "Local address"],
            rows: active,
        },
        Section {
            id: "adapters",
            title: translate(
                "网卡、地址与 DNS 配置",
                "Network adapters, addresses and DNS",
            ),
            columns: &[
                "Index",
                "Name",
                "State",
                "Type",
                "Speed",
                "Description",
                "IPv4",
                "IPv6",
                "Gateway",
                "DNS",
            ],
            rows: native.adapters,
        },
        Section {
            id: "routes",
            title: translate(
                "默认路由及其分流与跃点数",
                "Default routes and effective metrics",
            ),
            columns: if cfg!(windows) {
                &[
                    "Prefix",
                    "Index",
                    "Interface",
                    "Next hop",
                    "Route metric",
                    "Interface metric",
                    "Effective metric",
                ]
            } else {
                &["Family", "Route"]
            },
            rows: native.routes,
        },
        Section {
            id: "egress",
            title: translate(
                "目标出口与源地址",
                "Destination routes and source addresses",
            ),
            columns: &[
                "Destination",
                "Matched prefix",
                "Index",
                "Interface",
                "Source",
                "Next hop",
            ],
            rows: native.egress,
        },
        Section {
            id: "proxy",
            title: translate("系统代理配置", "System proxy settings"),
            columns: &["Source", "Setting", "Value"],
            rows: native.proxies,
        },
        Section {
            id: "processes",
            title: translate("代理、隧道与 VPN 进程", "Proxy, tunnel and VPN processes"),
            columns: &["PID", "Name", "Evidence"],
            rows: native.processes,
        },
        Section {
            id: "sockets",
            title: translate("监听端口与活动套接字", "Listeners and related connections"),
            columns: if cfg!(windows) {
                &[
                    "Protocol",
                    "State",
                    "Local endpoint",
                    "Remote endpoint",
                    "PID",
                    "Process",
                ]
            } else {
                &["Socket"]
            },
            rows: native.sockets,
        },
        Section {
            id: "relationships",
            title: translate(
                "本地进程连接与代理出口",
                "Local process peers and proxy egress",
            ),
            columns: &[
                "Evidence",
                "Process",
                "Source",
                "Peer or adapter",
                "Destination",
            ],
            rows: native.relations,
        },
        Section {
            id: "interception",
            title: translate("接管网络的线索", "Interception evidence"),
            columns: &["Type", "Name / target", "Observation"],
            rows: native.interception,
        },
        Section {
            id: "controller",
            title: translate(
                "Mihomo 本地控制器观测",
                "Local Mihomo controller observations",
            ),
            columns: &["Process", "Endpoint", "Rule / result", "Chain"],
            rows: controller,
        },
        Section {
            id: "icmp",
            title: translate(
                "网关、外网及校园基础设施 ICMP",
                "Gateway, internet and campus ICMP",
            ),
            columns: &["Target", "Probe", "Result", "Elapsed"],
            rows: native.reachability,
        },
        Section {
            id: "tcp",
            title: translate(
                "外网与校园服务连通性",
                "Internet and campus service reachability",
            ),
            columns: &[
                "Purpose",
                "Target",
                "TCP",
                "Source address",
                "TCP elapsed",
                "HTTP",
            ],
            rows: probes,
        },
        Section {
            id: "dns",
            title: translate("逐服务器 DNS 检测", "Per-server DNS checks"),
            columns: &["Server", "Name", "Result", "Elapsed"],
            rows: dns,
        },
        Section {
            id: "health",
            title: translate("BitNet 健康检测记录", "BitNet health checks"),
            columns: &["Probe", "Result", "Checked"],
            rows: checks,
        },
        Section {
            id: "missing",
            title: translate("无法读取的证据", "Unavailable evidence"),
            columns: &["Source"],
            rows: native
                .unavailable
                .into_iter()
                .map(|source| vec![source])
                .collect(),
        },
    ];
    let title = translate("BitNet 网络诊断", "BitNet network diagnostics");
    let timestamp = bitnet_core::now_secs();
    let mut plain = format!(
        "{title} · {} · {timestamp} (Unix UTC)\n",
        snapshot.app_version
    );
    let mut html = format!(
        "<!doctype html><html lang=\"{}\"><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'\"><title>{title}</title><style>{STYLE}</style><main><h1>{title}</h1><p>BitNet {} · {timestamp} (Unix UTC)</p><ul>",
        if english { "en" } else { "zh-CN" },
        escaped(&snapshot.app_version),
    );
    for finding in &findings {
        let _ = writeln!(plain, "- {finding}");
        let _ = write!(html, "<li>{}</li>", escaped(finding));
    }
    html.push_str("</ul><nav>");
    for section in &sections {
        let _ = write!(html, "<a href=\"#{}\">{}</a>", section.id, section.title);
    }
    html.push_str("</nav>");
    let topology_title = translate("出口拓扑", "Egress topology");
    let _ = write!(html, "<h2>{topology_title}</h2>");
    for (index, route) in routes.iter().enumerate() {
        let [destination, prefix, _, adapter, source, next_hop] = route.as_slice() else {
            continue;
        };
        if adapter == "—" {
            continue;
        }
        let adapter_anchor = adapters
            .iter()
            .position(|name| name.as_ref() == Some(adapter))
            .map_or_else(
                || "adapters".into(),
                |position| format!("adapters-{position}"),
            );
        let _ = write!(
            html,
            "<div class=\"flow\"><a href=\"#{adapter_anchor}\">{} · {}</a><span>→</span><a href=\"#routes\">{} · {}</a><span>→</span><a href=\"#egress-{index}\">{}</a></div>",
            escaped(adapter), escaped(source), escaped(next_hop), escaped(prefix), escaped(destination),
        );
    }
    if routes
        .iter()
        .all(|route| route.get(3).is_none_or(|adapter| adapter == "—"))
    {
        let _ = write!(
            html,
            "<p>{}</p>",
            translate(
                "系统未报告目标出口。",
                "The system reported no destination routes."
            )
        );
    }
    if socket_count > 0 {
        let _ = write!(
            html,
            "<div class=\"flow\"><a href=\"#sockets\">{} ({socket_count})</a><a href=\"#relationships\">{} ({relationship_count})</a><a href=\"#controller\">{}</a></div>",
            translate("套接字", "Sockets"), translate("进程及网卡关联", "Process and adapter relationships"), translate("代理转发规则", "Proxy rules"),
        );
    }
    for section in sections {
        let _ = writeln!(
            plain,
            "\n{}\n{}",
            section.title,
            section.columns.join(" | ")
        );
        let _ = write!(
            html,
            "<section><details {}><summary>{} <small>({})</small></summary><div class=\"evidence\" id=\"{}\">",
            if section.id == "egress" || section.id == "health" {
                "open"
            } else {
                ""
            },
            section.title,
            section.rows.len(),
            section.id,
        );
        if section.rows.is_empty() {
            let empty = translate("未取得记录。", "No records collected.");
            let _ = writeln!(plain, "{empty}");
            let _ = write!(html, "<p>{empty}</p>");
        } else {
            html.push_str("<div class=\"table\"><table><thead><tr>");
            for column in section.columns {
                let _ = write!(html, "<th>{column}</th>");
            }
            html.push_str("</tr></thead><tbody>");
            for (index, row) in section.rows.into_iter().enumerate() {
                let _ = writeln!(plain, "{}", row.join(" | "));
                let _ = write!(html, "<tr id=\"{}-{index}\">", section.id);
                for cell in row {
                    let _ = write!(html, "<td>{}</td>", escaped(&cell));
                }
                html.push_str("</tr>");
            }
            html.push_str("</tbody></table></div>");
        }
        html.push_str("</div></details></section>");
    }
    html.push_str("</main></html>");
    Report {
        findings,
        text: plain,
        html,
    }
}

fn escaped(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        escaped.push_str(match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => {
                escaped.push(ch);
                continue;
            }
        });
    }
    escaped
}

fn health_label(health: &Health) -> &'static str {
    match health {
        Health::Healthy => "ok",
        Health::Degraded => "degraded",
        Health::Unavailable => "unavailable",
        Health::Unknown => "unknown",
    }
}

const STYLE: &str = "
    :root{color-scheme:light dark;font:15px/1.45 system-ui,sans-serif}
    body{margin:0;background:Canvas;color:CanvasText}main{max-width:90rem;margin:auto;padding:1.5rem}
    nav,.flow{display:flex;flex-wrap:wrap;align-items:center;gap:.65rem;margin:1rem 0}
    nav a,.flow a{padding:.45rem .65rem;border:1px solid GrayText;text-decoration:none}
    section{margin:.6rem 0;border:1px solid GrayText;padding:.6rem .8rem;scroll-margin-top:.5rem}
    .evidence:target,tr:target{outline:2px solid Highlight;outline-offset:-2px;scroll-margin-top:.5rem}
    summary{font-size:1.1rem;cursor:pointer}small{color:GrayText}.table{overflow:auto}
    table{border-collapse:collapse;width:100%;margin-top:.7rem}th,td{text-align:left;padding:.35rem .6rem;border-bottom:1px solid GrayText;overflow-wrap:anywhere}
    th{background:Canvas}ul{padding-left:1.5rem}";

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn simulation_report_is_openable_document_and_escapes_inventory() {
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.capabilities.push("simulation".into());
        snapshot.app_version = "<script>fixture</script>".into();
        let report = collect(snapshot, true).await;
        assert!(report.html.contains("&lt;script&gt;fixture&lt;/script&gt;"));
        assert!(!report.html.contains("<script>"));
        assert!(report.html.contains("href=\"#egress\""));
        assert!(report.html.contains("<details open>"));
        assert!(report.text.contains("Simulation mode"));
        assert!(report.text.contains("Default routes and effective metrics"));
    }

    #[test]
    fn rejects_malformed_dns_response() {
        assert_eq!(dns_response(&[0, 1]), None);
        assert_eq!(
            dns_response(&[0x42, 0x4e, 0x80, 3, 0, 1, 0, 0, 0, 0, 0, 0]),
            Some((3, 0))
        );
        assert_eq!(
            dns_response(&[0x42, 0x4e, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0]),
            None
        );
    }
}
