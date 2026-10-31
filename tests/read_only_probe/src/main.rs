//! Explicitly authorized, one-shot status observation. No runtime or account store.
use bitnet_core::{AuthState, GatewayConfig, GatewaySession};
use bitnet_gateway::{GatewayClient, GatewayError};
use bitnet_platform::network;
use serde_json::{json, Map, Value};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    net::Ipv4Addr,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

const MAX_RAW: usize = 64 * 1024;
const MAX_HEADERS: usize = 16 * 1024;
const OPERATION_TIMEOUT: Duration = Duration::from_secs(12);
const LIVE_TARGET: Ipv4Addr = Ipv4Addr::new(10, 0, 0, 55);
const STATUS_REQUEST: &[u8] = b"GET /cgi-bin/rad_user_info?callback=jsonp HTTP/1.1\r\n";
type ProbeResult<T> = Result<T, &'static str>;

struct Arguments {
    jsonp: PathBuf,
    report: PathBuf,
    live: bool,
}

fn arguments() -> ProbeResult<Option<Arguments>> {
    let mut values = std::env::args_os().skip(1);
    let (mut jsonp, mut report, mut live) = (None, None, false);
    while let Some(argument) = values.next() {
        match argument.to_str() {
            Some("--help") => return Ok(None),
            Some("--jsonp") if jsonp.is_none() => {
                jsonp = Some(PathBuf::from(values.next().ok_or("argument_missing")?));
            }
            Some("--report") if report.is_none() => {
                report = Some(PathBuf::from(values.next().ok_or("argument_missing")?));
            }
            Some("--live-read-only") if !live => live = true,
            _ => return Err("argument_invalid"),
        }
    }
    Ok(Some(Arguments {
        jsonp: jsonp.ok_or("jsonp_argument_required")?,
        report: report.ok_or("report_argument_required")?,
        live,
    }))
}

fn private_paths(arguments: &Arguments) -> ProbeResult<(PathBuf, PathBuf)> {
    if !arguments.jsonp.is_absolute() || !arguments.report.is_absolute() {
        return Err("absolute_private_paths_required");
    }
    let project = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .ok_or("project_directory_invalid")?;
    let private = project
        .join(".local/live-readonly")
        .canonicalize()
        .map_err(|_| "private_directory_unavailable")?;
    let input = arguments
        .jsonp
        .canonicalize()
        .map_err(|_| "private_input_unavailable")?;
    let report_parent = arguments
        .report
        .parent()
        .ok_or("private_report_parent_required")?
        .canonicalize()
        .map_err(|_| "private_report_parent_unavailable")?;
    if !input.starts_with(&private) || !report_parent.starts_with(&private) {
        return Err("paths_outside_private_directory");
    }
    let report_name = arguments
        .report
        .file_name()
        .ok_or("private_report_filename_required")?;
    Ok((input, report_parent.join(report_name)))
}

fn read_bounded(path: &Path) -> ProbeResult<Vec<u8>> {
    let file = File::open(path).map_err(|_| "private_input_unavailable")?;
    if !file
        .metadata()
        .map_err(|_| "private_input_metadata_failed")?
        .is_file()
    {
        return Err("private_input_not_regular_file");
    }
    let mut raw = Vec::new();
    file.take((MAX_RAW + 1) as u64)
        .read_to_end(&mut raw)
        .map_err(|_| "private_input_read_failed")?;
    if raw.len() > MAX_RAW {
        return Err("private_input_too_large");
    }
    Ok(raw)
}

/// Replace only the callback identifier. Every other original byte is retained.
fn normalize_callback(raw: &[u8]) -> ProbeResult<(Vec<u8>, Value)> {
    if raw.len() > MAX_RAW {
        return Err("private_input_too_large");
    }
    let text = std::str::from_utf8(raw).map_err(|_| "jsonp_utf8_invalid")?;
    let trimmed = text.trim();
    let wrapped = trimmed.strip_suffix(';').unwrap_or(trimmed).trim_end();
    let inner = wrapped
        .strip_prefix("bitnetReadOnly(")
        .and_then(|body| body.strip_suffix(')'))
        .ok_or("jsonp_wrapper_invalid")?;
    let original: Value = serde_json::from_str(inner).map_err(|_| "jsonp_payload_invalid")?;
    if !original.is_object() {
        return Err("jsonp_payload_not_object");
    }
    let prefix = text.len() - text.trim_start().len();
    let mut replay = Vec::with_capacity(raw.len());
    replay.extend_from_slice(&raw[..prefix]);
    replay.extend_from_slice(b"jsonp");
    replay.extend_from_slice(&raw[prefix + b"bitnetReadOnly".len()..]);
    Ok((replay, original))
}

async fn replay_status(body: Vec<u8>) -> ProbeResult<(Result<GatewaySession, GatewayError>, bool)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "loopback_bind_failed")?;
    let address = listener
        .local_addr()
        .map_err(|_| "loopback_address_failed")?;
    let server = tokio::spawn(async move {
        let (mut stream, peer) = listener
            .accept()
            .await
            .map_err(|_| "loopback_accept_failed")?;
        if !peer.ip().is_loopback() {
            return Err("loopback_peer_invalid");
        }
        let mut request = Vec::new();
        while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
            let mut block = [0; 1024];
            let length = stream
                .read(&mut block)
                .await
                .map_err(|_| "loopback_request_read_failed")?;
            if length == 0 || request.len() + length > MAX_HEADERS {
                return Err("loopback_request_invalid");
            }
            request.extend_from_slice(&block[..length]);
        }
        // Fixed GET target is the only request this server accepts. No portal,
        // challenge, authentication or discovery path is ever served.
        if !request.starts_with(STATUS_REQUEST) {
            return Err("loopback_status_path_assertion_failed");
        }
        let header_end = request
            .windows(4)
            .position(|bytes| bytes == b"\r\n\r\n")
            .ok_or("loopback_headers_invalid")?
            + 4;
        if request.len() != header_end {
            return Err("loopback_unexpected_request_body");
        }
        let headers = std::str::from_utf8(&request).map_err(|_| "loopback_headers_invalid")?;
        for header in headers
            .split("\r\n")
            .skip(1)
            .filter(|line| !line.is_empty())
        {
            let (name, value) = header.split_once(':').ok_or("loopback_headers_invalid")?;
            if name.eq_ignore_ascii_case("transfer-encoding")
                || (name.eq_ignore_ascii_case("content-length") && value.trim() != "0")
            {
                return Err("loopback_unexpected_request_body");
            }
        }
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/javascript\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(header.as_bytes())
            .await
            .map_err(|_| "loopback_response_write_failed")?;
        stream
            .write_all(&body)
            .await
            .map_err(|_| "loopback_response_write_failed")?;
        stream
            .shutdown()
            .await
            .map_err(|_| "loopback_response_shutdown_failed")?;
        drop(stream);
        if timeout(Duration::from_millis(100), listener.accept())
            .await
            .is_ok()
        {
            return Err("loopback_unexpected_additional_connection");
        }
        Ok(true)
    });
    let client = GatewayClient::new(GatewayConfig {
        base_url: format!("http://{address}"),
        timeout_secs: 8,
        ..GatewayConfig::default()
    })
    .map_err(|_| "loopback_client_configuration_failed")?;
    let parsed = client.status(None).await;
    let request_exact = match timeout(Duration::from_secs(2), server).await {
        Ok(Ok(result)) => result?,
        _ => return Err("loopback_server_failed"),
    };
    Ok((parsed, request_exact))
}

fn gateway_error(error: &GatewayError) -> &'static str {
    match error {
        GatewayError::InvalidConfiguration => "gateway_configuration_invalid",
        GatewayError::InvalidInput => "gateway_input_invalid",
        GatewayError::BindingRequired => "gateway_binding_required",
        GatewayError::InvalidBinding => "gateway_binding_invalid",
        GatewayError::InterfaceBinding => "gateway_interface_binding_failed",
        GatewayError::Transport => "gateway_transport_failed",
        GatewayError::Deadline => "gateway_deadline",
        GatewayError::HttpStatus(_) => "gateway_http_status_error",
        GatewayError::RedirectRejected => "gateway_redirect_rejected",
        GatewayError::ResponseTooLarge => "gateway_response_too_large",
        GatewayError::InvalidResponse => "gateway_response_invalid",
        GatewayError::UnknownGatewayStatus => "gateway_status_unknown",
        _ => "gateway_status_failed",
    }
}

fn unsigned_value(value: Option<&Value>) -> (Option<u64>, bool) {
    match value {
        None | Some(Value::Null) => (None, true),
        Some(Value::String(text)) => (text.parse().ok(), text.parse::<u64>().is_ok()),
        Some(Value::Number(number)) => (number.as_u64(), number.as_u64().is_some()),
        _ => (None, false),
    }
}

fn balance_value(value: Option<&Value>) -> (Option<f64>, bool) {
    let parsed = match value {
        None | Some(Value::Null) => return (None, true),
        Some(Value::String(text)) => text.parse::<f64>().ok(),
        Some(Value::Number(number)) => number.as_f64(),
        _ => None,
    };
    (parsed, parsed.is_some_and(f64::is_finite))
}

fn compare_fields(original: &Value, session: &GatewaySession) -> (Value, bool) {
    let expected_auth = match original.get("error").and_then(Value::as_str) {
        Some("ok") => Some(AuthState::Authenticated),
        Some("not_online_error") => Some(AuthState::Unauthenticated),
        _ => None,
    };
    let username = original
        .get("user_name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty());
    let username_typed = original
        .get("user_name")
        .is_none_or(|value| value.is_null() || value.is_string());
    let ip = session.ip.map(|value| value.to_string());
    let (bytes, bytes_typed) = unsigned_value(original.get("sum_bytes"));
    let (seconds, seconds_typed) = unsigned_value(original.get("sum_seconds"));
    let (balance, balance_typed) = balance_value(original.get("user_balance"));
    let comparisons = [
        (
            "error",
            expected_auth.is_some(),
            expected_auth.as_ref() == Some(&session.auth),
            json!(session.auth),
        ),
        (
            "user_name",
            username_typed,
            username == session.username.as_deref(),
            json!(session.username),
        ),
        (
            "online_ip",
            original.get("online_ip").is_some_and(Value::is_string),
            original.get("online_ip").and_then(Value::as_str) == ip.as_deref(),
            json!(ip),
        ),
        (
            "sum_bytes",
            bytes_typed,
            bytes == session.bytes_used,
            json!(session.bytes_used),
        ),
        (
            "sum_seconds",
            seconds_typed,
            seconds == session.online_seconds,
            json!(session.online_seconds),
        ),
        (
            "user_balance",
            balance_typed,
            balance == session.balance,
            json!(session.balance),
        ),
    ];
    let all_match = comparisons
        .iter()
        .all(|(_, typed, same, _)| *typed && *same);
    let fields: Map<String, Value> = comparisons
        .into_iter()
        .map(|(name, typed, same, observed)| {
            (
                name.into(),
                json!({
                    "original": original.get(name), "parsed": observed,
                    "original_well_typed": typed, "matches": typed && same,
                }),
            )
        })
        .collect();
    (Value::Object(fields), all_match)
}

async fn collect(replay: Vec<u8>, original: Value, live_authorized: bool) -> ProbeResult<Value> {
    let (replayed, request_exact) = replay_status(replay).await?;
    let (replay_session, replay_error, comparisons, replay_matches) = match replayed {
        Ok(session) => {
            let (comparisons, same) = compare_fields(&original, &session);
            (json!(session), Value::Null, comparisons, same)
        }
        Err(error) => (
            Value::Null,
            json!(gateway_error(&error)),
            Value::Null,
            false,
        ),
    };
    let mut report = json!({
        "format_version": 1,
        "read_only": true,
        "live_authorized": live_authorized,
        "live_status_calls": 0,
        "max_live_status_calls": 1,
        "live": null,
        "replay": {"request_path_exact": request_exact, "callback_only_changed": true,
            "session": replay_session, "error": replay_error, "fields": comparisons},
        "checks": {"replay_matches_original_fields": replay_matches,
            "loopback_status_path_exact": request_exact, "live_status_success": false,
            "preferred_binding_matches_interface": false},
    });
    if live_authorized {
        let interfaces = match network::interfaces() {
            Ok(interfaces) => interfaces,
            Err(_) => {
                report["live"] = json!({"error": "interfaces_read_failed"});
                return Ok(report);
            }
        };
        let binding = match network::preferred_binding(LIVE_TARGET) {
            Ok(Some(binding)) => binding,
            Ok(None) => {
                report["live"] = json!({"error": "preferred_binding_unavailable"});
                return Ok(report);
            }
            Err(_) => {
                report["live"] = json!({"error": "preferred_binding_read_failed"});
                return Ok(report);
            }
        };
        let matches_interface = interfaces.iter().any(|interface| {
            interface.id == binding.interface_id
                && interface.index == binding.interface_index
                && interface.up
                && !interface.virtual_adapter
                && interface.ipv4.contains(&binding.source_ip)
        });
        report["checks"]["preferred_binding_matches_interface"] = json!(matches_interface);
        report["interfaces"] = json!(interfaces);
        report["preferred_binding"] = json!(binding);
        if !matches_interface {
            report["live"] = json!({"error": "binding_snapshot_inconsistent"});
            return Ok(report);
        }
        // There is exactly one live status() call in this executable, guarded
        // by the explicit flag. The production status path has no retry,
        // discovery, redirect following, credential access or auth operation.
        let client = GatewayClient::new(GatewayConfig {
            base_url: "http://10.0.0.55".into(),
            timeout_secs: 8,
            ..GatewayConfig::default()
        })
        .map_err(|_| "fixed_gateway_configuration_failed")?;
        report["live_status_calls"] = json!(1);
        match client.status(Some(&binding)).await {
            Ok(session) => {
                report["checks"]["live_status_success"] = json!(true);
                report["live"] = json!({"session": session, "error": null});
            }
            Err(error) => report["live"] = json!({"session": null, "error": gateway_error(&error)}),
        }
    }
    Ok(report)
}

fn run() -> ProbeResult<()> {
    let Some(arguments) = arguments()? else {
        println!("Use --jsonp <absolute-private-input> --report <absolute-new-private-report> [--live-read-only]");
        return Ok(());
    };
    let (input, output) = private_paths(&arguments)?;
    // Reserve the protected output before any live observation. An existing
    // file is never replaced, and no directory or ACL is created/modified.
    let mut report_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| "private_report_create_failed")?;
    let result = (|| {
        let raw = read_bounded(&input)?;
        let (replay, original) = normalize_callback(&raw)?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "probe_runtime_creation_failed")?;
        runtime.block_on(async {
            timeout(OPERATION_TIMEOUT, collect(replay, original, arguments.live))
                .await
                .map_err(|_| "probe_total_deadline")?
        })
    })();
    let report = match &result {
        Ok(report) => report.clone(),
        Err(error) => json!({"format_version": 1, "read_only": true, "error": error}),
    };
    let bytes = serde_json::to_vec_pretty(&report).map_err(|_| "private_report_encode_failed")?;
    report_file
        .write_all(&bytes)
        .and_then(|_| report_file.write_all(b"\n"))
        .and_then(|_| report_file.sync_all())
        .map_err(|_| "private_report_write_failed")?;
    let report = result?;
    if let Some(checks) = report.get("checks").and_then(Value::as_object) {
        for (label, value) in checks {
            println!("{label}={}", value.as_bool().unwrap_or(false));
        }
    }
    println!("private_report_written=true");
    if report["checks"]["replay_matches_original_fields"] != true
        || (arguments.live && report["checks"]["live_status_success"] != true)
    {
        return Err("read_only_checks_failed");
    }
    Ok(())
}

fn main() {
    std::panic::set_hook(Box::new(|_| eprintln!("probe_panic")));
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SYNTHETIC: &[u8] = br#"bitnetReadOnly({"error":"ok","user_name":"synthetic-user","online_ip":"192.0.2.8","sum_bytes":"12345","sum_seconds":67,"user_balance":"8.25"});"#;

    #[test]
    fn normalization_changes_callback_identifier_only() {
        let mut original = b" \r\n".to_vec();
        original.extend_from_slice(SYNTHETIC);
        original.extend_from_slice(b"\r\n");
        let (normalized, _) = normalize_callback(&original).expect("synthetic wrapper");
        let expected = String::from_utf8(original)
            .expect("synthetic utf8")
            .replacen("bitnetReadOnly", "jsonp", 1);
        assert_eq!(normalized, expected.as_bytes());
    }

    #[test]
    fn rejects_other_callback_extra_script_and_oversized_body() {
        assert!(normalize_callback(br#"other({"error":"ok"})"#).is_err());
        assert!(normalize_callback(br#"bitnetReadOnly({});alert(1)"#).is_err());
        assert!(normalize_callback(&vec![b' '; MAX_RAW + 1]).is_err());
    }

    #[tokio::test]
    async fn synthetic_replay_uses_production_status_and_all_six_fields_match() {
        let (body, original) = normalize_callback(SYNTHETIC).expect("synthetic wrapper");
        let report = timeout(OPERATION_TIMEOUT, collect(body, original, false))
            .await
            .expect("synthetic deadline")
            .expect("synthetic replay");
        assert_eq!(report["live_status_calls"], 0);
        assert_eq!(report["checks"]["replay_matches_original_fields"], true);
        assert_eq!(report["checks"]["loopback_status_path_exact"], true);
        assert_eq!(
            report["replay"]["fields"].as_object().map(Map::len),
            Some(6)
        );
        assert!(report.get("interfaces").is_none());
    }

    #[tokio::test]
    async fn malformed_source_is_recorded_without_body_or_transport_error_text() {
        let (body, original) = normalize_callback(
            br#"bitnetReadOnly({"error":"ok","user_name":"synthetic-user","online_ip":"invalid"})"#,
        )
        .expect("synthetic wrapper");
        let report = collect(body, original, false)
            .await
            .expect("synthetic replay");
        assert_eq!(report["replay"]["error"], "gateway_response_invalid");
        assert_eq!(report["checks"]["replay_matches_original_fields"], false);
        assert_eq!(report["live_status_calls"], 0);
    }
}
