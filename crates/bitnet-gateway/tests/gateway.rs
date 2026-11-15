//! Only synthetic credentials and literal loopback HTTP fixtures are used here.
use bitnet_core::{AuthState, ErrorCode, GatewayConfig, NetworkBinding};
use bitnet_gateway::{GatewayClient, GatewayError};
use std::{net::Ipv4Addr, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::JoinHandle,
};
use url::Url;

const OFFLINE: &str = r#"jsonp({"error":"not_online_error","res":"not_online_error","online_ip":"10.1.2.3","sum_seconds":0})"#;
const ONLINE: &str = r#"jsonp({"error":"ok","res":"ok","online_ip":"10.1.2.3","user_name":"fixture-user","sum_bytes":"12345","sum_seconds":12,"user_balance":"4.50"})"#;
const CHALLENGE: &str =
    r#"jsonp({"challenge":"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"})"#;
const OK: &str = r#"jsonp({"error":"ok"})"#;

struct Step {
    path: &'static str,
    reply: String,
    delay: Duration,
}
fn step(path: &'static str, body: &str) -> Step {
    Step {
        path,
        reply: format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
        delay: Duration::ZERO,
    }
}
fn redirect(path: &'static str, location: &str) -> Step {
    Step { path, reply: format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"), delay: Duration::ZERO }
}
async fn fixture(
    steps: Vec<Step>,
    ac_id: Option<&str>,
    timeout_secs: u64,
) -> (GatewayClient, JoinHandle<Vec<Url>>) {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let address = listener.local_addr().unwrap();
    let client = GatewayClient::new(GatewayConfig {
        base_url: format!("http://{address}"),
        ac_id: ac_id.map(str::to_owned),
        timeout_secs,
    })
    .unwrap();
    let server = tokio::spawn(async move {
        let mut observed = Vec::new();
        for step in steps {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut part = [0; 1024];
                let count = connection.read(&mut part).await.unwrap();
                assert!(count > 0, "client closed before sending a request");
                request.extend_from_slice(&part[..count]);
                assert!(request.len() <= 32 * 1024);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            let request = std::str::from_utf8(&request).unwrap();
            let line = request.lines().next().unwrap();
            let target = line.split_whitespace().nth(1).unwrap();
            let url = Url::parse(&format!("http://{address}{target}")).unwrap();
            assert_eq!(url.path(), step.path);
            observed.push(url);
            if !step.delay.is_zero() {
                tokio::time::sleep(step.delay).await;
            }
            let _ = connection.write_all(step.reply.as_bytes()).await;
        }
        observed
    });
    (client, server)
}
async fn finish(server: JoinHandle<Vec<Url>>) -> Vec<Url> {
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .expect("fixture did not receive expected requests")
        .unwrap()
}

#[test]
fn configuration_is_a_literal_http_origin_and_errors_are_redacted() {
    for base_url in [
        "https://10.0.0.55",
        "http://localhost",
        "http://name.invalid",
        "http://user:secret@127.0.0.1",
        "http://127.0.0.1/path",
        "http://127.0.0.1?secret=x",
        "http://0.0.0.0",
    ] {
        assert!(GatewayClient::new(GatewayConfig {
            base_url: base_url.into(),
            ..GatewayConfig::default()
        })
        .is_err());
    }
    let config = GatewayConfig {
        timeout_secs: 0,
        ..GatewayConfig::default()
    };
    assert!(GatewayClient::new(config).is_err());
    let error = GatewayError::CredentialsRejected.into_app_error();
    assert_eq!(error.code, ErrorCode::Credentials);
    assert!(!error.retryable);
    assert!(!format!("{error:?}").contains("fixture-password"));
}

#[tokio::test]
async fn all_non_loopback_actions_require_a_binding_before_any_network_io() {
    let client = GatewayClient::new(GatewayConfig::default()).unwrap();
    assert_eq!(
        client.status(None).await.unwrap_err(),
        GatewayError::BindingRequired
    );
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::BindingRequired
    );
    assert_eq!(
        client.logout("fixture-user", None).await.unwrap_err(),
        GatewayError::BindingRequired
    );
    let invalid = NetworkBinding {
        interface_id: "invalid".into(),
        interface_index: 0,
        source_ip: Ipv4Addr::LOCALHOST,
    };
    assert_eq!(
        client.status(Some(&invalid)).await.unwrap_err(),
        GatewayError::InvalidBinding
    );
}

#[tokio::test]
async fn offline_ip_and_zero_statistics_are_not_online_evidence() {
    let (client, server) = fixture(vec![step("/cgi-bin/rad_user_info", OFFLINE)], None, 2).await;
    let status = client.status(None).await.unwrap();
    assert_eq!(status.auth, AuthState::Unauthenticated);
    assert_eq!(status.ip, Some(Ipv4Addr::new(10, 1, 2, 3)));
    finish(server).await;
}

#[tokio::test]
async fn online_status_supports_numeric_strings_and_single_optional_semicolon() {
    let body = format!(" \r\n{ONLINE}; \n");
    let (client, server) = fixture(vec![step("/cgi-bin/rad_user_info", &body)], None, 2).await;
    let status = client.status(None).await.unwrap();
    assert_eq!(status.auth, AuthState::Authenticated);
    assert_eq!(status.username.as_deref(), Some("fixture-user"));
    assert_eq!(status.bytes_used, Some(12345));
    assert_eq!(status.balance, Some(4.5));
    finish(server).await;
}

#[tokio::test]
async fn malformed_unknown_and_conflicting_statuses_never_become_online_or_offline() {
    for (body, expected) in [
        (
            r#"jsonp({"error":"ok","online_ip":"10.1.2.3"})"#,
            GatewayError::InvalidResponse,
        ),
        (
            r#"jsonp({"error":"not_online_error","online_ip":"10.1.2.3","user_name":"other"})"#,
            GatewayError::InvalidResponse,
        ),
        (
            r#"jsonp({"error":"server_error","online_ip":"10.1.2.3","sum_seconds":42})"#,
            GatewayError::UnknownGatewayStatus,
        ),
        (
            r#"jsonp({"error":"ok","error":"not_online_error","online_ip":"10.1.2.3"})"#,
            GatewayError::InvalidResponse,
        ),
        (
            r#"jsonp({"error":"ok","online_ip":"0.0.0.0","user_name":"fixture-user"})"#,
            GatewayError::InvalidResponse,
        ),
        (
            r#"{"error":"not_online_error","online_ip":"10.1.2.3"}"#,
            GatewayError::InvalidResponse,
        ),
        ("not_online_error", GatewayError::InvalidResponse),
        ("<html>login</html>", GatewayError::InvalidResponse),
        ("evil({});", GatewayError::InvalidResponse),
        ("jsonp({});alert(1)", GatewayError::InvalidResponse),
    ] {
        let (client, server) = fixture(vec![step("/cgi-bin/rad_user_info", body)], None, 2).await;
        assert_eq!(client.status(None).await.unwrap_err(), expected);
        finish(server).await;
    }
}

#[tokio::test]
async fn http_errors_do_not_become_unauthenticated() {
    let broken = Step {
        path: "/cgi-bin/rad_user_info",
        reply: "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(),
        delay: Duration::ZERO,
    };
    let (client, server) = fixture(vec![broken], None, 2).await;
    assert_eq!(
        client.status(None).await.unwrap_err(),
        GatewayError::HttpStatus(503)
    );
    finish(server).await;
}

#[tokio::test]
async fn response_limits_cover_content_length_and_chunked_bodies() {
    for reply in [
        "HTTP/1.1 200 OK\r\nContent-Length: 65537\r\nConnection: close\r\n\r\n".to_string(),
        format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n10001\r\n{}\r\n0\r\n\r\n", "x".repeat(65537)),
    ] {
        let (client, server) = fixture(vec![Step { path: "/cgi-bin/rad_user_info", reply, delay: Duration::ZERO }], None, 2).await;
        assert_eq!(client.status(None).await.unwrap_err(), GatewayError::ResponseTooLarge);
        finish(server).await;
    }
}

#[tokio::test]
async fn operation_deadline_covers_slow_http_response() {
    let mut slow = step("/cgi-bin/rad_user_info", OFFLINE);
    slow.delay = Duration::from_secs(2);
    let (client, server) = fixture(vec![slow], None, 1).await;
    let started = tokio::time::Instant::now();
    assert_eq!(
        client.status(None).await.unwrap_err(),
        GatewayError::Deadline
    );
    assert!(started.elapsed() < Duration::from_millis(1600));
    server.abort();
}

#[tokio::test]
async fn successful_login_discovers_ac_id_and_waits_for_actual_session() {
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            redirect("/", "/index_1.html"),
            redirect("/index_1.html", "/srun_portal_pc?ac_id=7"),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", OK),
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/rad_user_info", ONLINE),
        ],
        None,
        3,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap()
            .auth,
        AuthState::Authenticated
    );
    let observed = finish(server).await;
    let login = &observed[4];
    let query: std::collections::HashMap<_, _> = login.query_pairs().into_owned().collect();
    assert_eq!(query.get("ac_id").unwrap(), "7");
    assert_eq!(query.get("action").unwrap(), "login");
    assert!(!login.as_str().contains("fixture-password"));
    assert!(query.get("info").unwrap().starts_with("{SRBX1}"));
}

#[tokio::test]
async fn already_online_is_idempotent_and_different_account_is_preserved() {
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", ONLINE),
            step("/cgi-bin/rad_user_info", ONLINE),
            step("/cgi-bin/rad_user_info", ONLINE),
        ],
        None,
        2,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap()
            .auth,
        AuthState::Authenticated
    );
    assert_eq!(
        client
            .login("other", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::AccountMismatch
    );
    assert_eq!(
        client.logout("other", None).await.unwrap_err(),
        GatewayError::AccountMismatch
    );
    assert!(finish(server)
        .await
        .iter()
        .all(|url| url.path() == "/cgi-bin/rad_user_info"));
}

#[tokio::test]
async fn credential_verification_requires_a_fresh_and_accepted_login() {
    let (client, server) =
        fixture(vec![step("/cgi-bin/rad_user_info", ONLINE)], Some("1"), 2).await;
    assert_eq!(
        client
            .verify_credentials("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::AccountMismatch
    );
    finish(server).await;

    let rejected = r#"jsonp({"error":"login_error","error_msg":"You are already online"})"#;
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", rejected),
        ],
        Some("1"),
        2,
    )
    .await;
    assert_eq!(
        client
            .verify_credentials("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::Rejected
    );
    finish(server).await;

    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", OK),
            step("/cgi-bin/rad_user_info", ONLINE),
        ],
        Some("1"),
        2,
    )
    .await;
    let session = client
        .verify_credentials("fixture-user", "fixture-password", None)
        .await
        .unwrap();
    assert_eq!(session.auth, AuthState::Authenticated);
    assert_eq!(session.username.as_deref(), Some("fixture-user"));
    finish(server).await;
}

#[tokio::test]
async fn credential_rejection_is_classified_without_echoing_server_text() {
    let bad = r#"jsonp({"error":"login_error","error_msg":"Password is error. untrusted fixture-password","ecode":"E2531"})"#;
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", bad),
            step("/cgi-bin/rad_user_info", OFFLINE),
        ],
        Some("1"),
        2,
    )
    .await;
    let error = client
        .login("fixture-user", "fixture-password", None)
        .await
        .unwrap_err();
    assert_eq!(error, GatewayError::CredentialsRejected);
    assert!(!error.to_string().contains("untrusted"));
    finish(server).await;
}

#[tokio::test]
async fn malformed_challenge_stops_before_authentication() {
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", r#"jsonp({"challenge":"a"})"#),
        ],
        Some("1"),
        2,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::InvalidResponse
    );
    finish(server).await;
}

#[tokio::test]
async fn authentication_redirect_is_not_followed_but_existing_session_is_rechecked() {
    let forbidden = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let location = format!("http://{}/would-leak-auth", forbidden.local_addr().unwrap());
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            redirect("/cgi-bin/srun_portal", &location),
            step("/cgi-bin/rad_user_info", OFFLINE),
        ],
        Some("1"),
        2,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::RedirectRejected
    );
    finish(server).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), forbidden.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn ac_id_is_rediscovered_for_each_new_authentication_operation() {
    let mut script = Vec::new();
    for ac_id in ["1", "2"] {
        script.extend([
            step("/cgi-bin/rad_user_info", OFFLINE),
            redirect("/", &format!("/portal?ac_id={ac_id}")),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", OK),
            step("/cgi-bin/rad_user_info", ONLINE),
        ]);
    }
    let (client, server) = fixture(script, None, 2).await;
    for _ in 0..2 {
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap();
    }
    let observed = finish(server).await;
    let ids: Vec<String> = observed
        .iter()
        .filter(|url| url.path() == "/cgi-bin/srun_portal")
        .flat_map(|url| {
            url.query_pairs()
                .filter(|(key, _)| key == "ac_id")
                .map(|(_, v)| v.into_owned())
        })
        .collect();
    assert_eq!(ids, ["1", "2"]);
}

#[tokio::test]
async fn ac_id_discovery_rejects_cross_origin_and_duplicate_identifiers() {
    for (location, expected) in [
        (
            "http://127.0.0.2/portal?ac_id=1",
            GatewayError::RedirectRejected,
        ),
        ("/portal?ac_id=1&ac_id=2", GatewayError::InvalidResponse),
        (
            "/cgi-bin/srun_portal?action=logout",
            GatewayError::RedirectRejected,
        ),
    ] {
        let (client, server) = fixture(
            vec![
                step("/cgi-bin/rad_user_info", OFFLINE),
                redirect("/", location),
            ],
            None,
            2,
        )
        .await;
        assert_eq!(
            client
                .login("fixture-user", "fixture-password", None)
                .await
                .unwrap_err(),
            expected
        );
        finish(server).await;
    }
}

#[tokio::test]
async fn logout_is_idempotent_and_requires_confirmation_of_new_state() {
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/rad_user_info", ONLINE),
            step("/cgi-bin/srun_portal", OK),
            step("/cgi-bin/rad_user_info", OFFLINE),
        ],
        Some("1"),
        2,
    )
    .await;
    assert_eq!(
        client.logout("fixture-user", None).await.unwrap().auth,
        AuthState::Unauthenticated
    );
    assert_eq!(
        client.logout("fixture-user", None).await.unwrap().auth,
        AuthState::Unauthenticated
    );
    let observed = finish(server).await;
    let query: std::collections::HashMap<_, _> = observed[2].query_pairs().into_owned().collect();
    assert_eq!(query.get("action").unwrap(), "logout");
    assert!(!query.contains_key("password"));
}

#[tokio::test]
async fn source_ip_and_interface_are_bound_on_the_supported_host() {
    // Index 1 is the Windows software loopback interface. Other platforms have
    // different loopback indices, covered by their target-native CI jobs.
    #[cfg(windows)]
    {
        let (client, server) =
            fixture(vec![step("/cgi-bin/rad_user_info", OFFLINE)], None, 2).await;
        let binding = NetworkBinding {
            interface_id: "loopback-fixture".into(),
            interface_index: 1,
            source_ip: Ipv4Addr::LOCALHOST,
        };
        assert_eq!(
            client.status(Some(&binding)).await.unwrap().auth,
            AuthState::Unauthenticated
        );
        finish(server).await;
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        #[cfg(target_os = "linux")]
        let name = c"lo";
        #[cfg(target_os = "macos")]
        let name = c"lo0";
        // SAFETY: a static NUL-terminated interface name is provided.
        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        assert_ne!(index, 0);
        let (client, server) =
            fixture(vec![step("/cgi-bin/rad_user_info", OFFLINE)], None, 2).await;
        let binding = NetworkBinding {
            interface_id: "loopback-fixture".into(),
            interface_index: index,
            source_ip: Ipv4Addr::LOCALHOST,
        };
        assert_eq!(
            client.status(Some(&binding)).await.unwrap().auth,
            AuthState::Unauthenticated
        );
        finish(server).await;
    }
}

#[tokio::test]
async fn invalid_interface_fails_closed_without_unbound_fallback() {
    let client = GatewayClient::new(GatewayConfig {
        base_url: "http://127.0.0.1:9".into(),
        ..GatewayConfig::default()
    })
    .unwrap();
    let binding = NetworkBinding {
        interface_id: "nonexistent-fixture".into(),
        interface_index: u32::MAX,
        source_ip: Ipv4Addr::LOCALHOST,
    };
    assert_eq!(
        client.status(Some(&binding)).await.unwrap_err(),
        GatewayError::InterfaceBinding
    );
}

#[tokio::test]
async fn lost_login_reply_is_reconciled_only_with_the_expected_account() {
    let portal_error = r#"jsonp({"error":"login_error","error_msg":"You are already online"})"#;
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", portal_error),
            step("/cgi-bin/rad_user_info", ONLINE),
        ],
        Some("1"),
        2,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap()
            .auth,
        AuthState::Authenticated
    );
    finish(server).await;
}

#[tokio::test]
async fn successful_portal_reply_is_not_enough_to_claim_success() {
    let (client, server) = fixture(
        vec![
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/get_challenge", CHALLENGE),
            step("/cgi-bin/srun_portal", OK),
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/rad_user_info", OFFLINE),
            step("/cgi-bin/rad_user_info", OFFLINE),
        ],
        Some("1"),
        3,
    )
    .await;
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::ConfirmationFailed
    );
    finish(server).await;
}

#[tokio::test]
async fn one_deadline_covers_all_requests_in_the_login_operation() {
    let mut first = step("/cgi-bin/rad_user_info", OFFLINE);
    first.delay = Duration::from_millis(600);
    let mut second = step("/cgi-bin/get_challenge", CHALLENGE);
    second.delay = Duration::from_millis(600);
    let (client, server) = fixture(vec![first, second], Some("1"), 1).await;
    let started = tokio::time::Instant::now();
    assert_eq!(
        client
            .login("fixture-user", "fixture-password", None)
            .await
            .unwrap_err(),
        GatewayError::Deadline
    );
    assert!(started.elapsed() < Duration::from_millis(1600));
    server.abort();
}

#[tokio::test]
async fn username_password_validation_happens_before_any_io() {
    let client = GatewayClient::new(GatewayConfig {
        base_url: "http://127.0.0.1:9".into(),
        ..GatewayConfig::default()
    })
    .unwrap();
    for username in ["", " ", "bad\nuser"] {
        assert_eq!(
            client
                .login(username, "fixture-password", None)
                .await
                .unwrap_err(),
            GatewayError::InvalidInput
        );
    }
    assert_eq!(
        client.login("fixture-user", "", None).await.unwrap_err(),
        GatewayError::InvalidInput
    );
    assert_eq!(
        client
            .login("fixture-user", "bad\0password", None)
            .await
            .unwrap_err(),
        GatewayError::InvalidInput
    );
}
