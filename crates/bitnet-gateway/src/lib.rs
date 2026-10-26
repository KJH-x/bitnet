//! Bound, deadline-limited SRUN HTTP client. No browser, proxy, GUI, or polling loop.
mod binding;
mod crypto;

use bitnet_core::{AppError, AuthState, ErrorCode, GatewayConfig, GatewaySession, NetworkBinding};
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{client::conn::http1, header, Request, StatusCode};
use hyper_util::rt::TokioIo;
use serde::Deserialize;
use std::{
    fmt,
    net::{Ipv4Addr, SocketAddrV4},
    time::Duration,
};
use tokio::{
    net::TcpSocket,
    time::{timeout_at, Instant},
};
use url::Url;
use zeroize::Zeroizing;

const MAX_BODY: usize = 64 * 1024;
const MAX_LOCATION: usize = 4096;
const CONFIRM_ATTEMPTS: usize = 4;
const CONFIRM_INTERVAL: Duration = Duration::from_millis(250);

/// Only static messages are exposed: transport errors and response bodies may
/// contain credentials or complete authentication URLs.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GatewayError {
    #[error("gateway configuration must be an HTTP IPv4 origin with a valid deadline")]
    InvalidConfiguration,
    #[error("gateway credentials or parameters are invalid")]
    InvalidInput,
    #[error("an explicit network binding is required")]
    BindingRequired,
    #[error("the selected network binding is invalid")]
    InvalidBinding,
    #[error("could not constrain the connection to the selected interface")]
    InterfaceBinding,
    #[error("gateway transport is unavailable")]
    Transport,
    #[error("the gateway operation exceeded its deadline")]
    Deadline,
    #[error("the gateway returned HTTP status {0}")]
    HttpStatus(u16),
    #[error("the gateway returned a disallowed redirect")]
    RedirectRejected,
    #[error("the gateway response exceeded its size limit")]
    ResponseTooLarge,
    #[error("the gateway returned an invalid or inconsistent response")]
    InvalidResponse,
    #[error("the gateway authentication state is unknown")]
    UnknownGatewayStatus,
    #[error("could not discover a valid network access-controller identifier")]
    DiscoveryFailed,
    #[error("the gateway rejected the account credentials")]
    CredentialsRejected,
    #[error("the gateway rejected the requested operation")]
    Rejected,
    #[error("another account already owns this network session")]
    AccountMismatch,
    #[error("the resulting gateway session could not be confirmed")]
    ConfirmationFailed,
}

impl GatewayError {
    pub fn into_app_error(self) -> AppError {
        let code = match self {
            Self::InvalidConfiguration | Self::InvalidInput | Self::InvalidBinding => {
                ErrorCode::InvalidInput
            }
            Self::BindingRequired | Self::AccountMismatch => ErrorCode::ConfirmationRequired,
            Self::CredentialsRejected => ErrorCode::Credentials,
            Self::InterfaceBinding => ErrorCode::Network,
            Self::InvalidResponse | Self::ResponseTooLarge | Self::RedirectRejected => {
                ErrorCode::Incompatible
            }
            Self::Rejected => ErrorCode::Unavailable,
            _ => ErrorCode::GatewayUnavailable,
        };
        AppError::new(code, self.to_string())
    }
}

#[derive(Clone)]
pub struct GatewayClient {
    origin: Url,
    address: SocketAddrV4,
    configured_ac_id: Option<String>,
    timeout: Duration,
}
impl fmt::Debug for GatewayClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GatewayClient { endpoint: [ADMIN_CONFIGURED] }")
    }
}

impl GatewayClient {
    pub fn new(config: GatewayConfig) -> Result<Self, GatewayError> {
        let origin =
            Url::parse(&config.base_url).map_err(|_| GatewayError::InvalidConfiguration)?;
        let ip: Ipv4Addr = origin
            .host_str()
            .and_then(|s| s.parse().ok())
            .ok_or(GatewayError::InvalidConfiguration)?;
        if origin.scheme() != "http"
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || ip.is_unspecified()
            || ip.is_multicast()
            || ip.is_broadcast()
            || !(1..=60).contains(&config.timeout_secs)
            || config.ac_id.as_ref().is_some_and(|id| !valid_ac_id(id))
        {
            return Err(GatewayError::InvalidConfiguration);
        }
        let port = origin
            .port_or_known_default()
            .ok_or(GatewayError::InvalidConfiguration)?;
        Ok(Self {
            address: SocketAddrV4::new(ip, port),
            origin,
            configured_ac_id: config.ac_id,
            timeout: Duration::from_secs(config.timeout_secs),
        })
    }

    pub async fn status(
        &self,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        self.validate_binding(binding)?;
        timeout_at(Instant::now() + self.timeout, self.status_inner(binding))
            .await
            .map_err(|_| GatewayError::Deadline)?
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        self.validate_binding(binding)?;
        if !valid_username(username)
            || password.is_empty()
            || password.len() > 1024
            || password.contains('\0')
        {
            return Err(GatewayError::InvalidInput);
        }
        timeout_at(
            Instant::now() + self.timeout,
            self.login_inner(username, password, binding),
        )
        .await
        .map_err(|_| GatewayError::Deadline)?
    }

    pub async fn logout(
        &self,
        username: &str,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        self.validate_binding(binding)?;
        if !valid_username(username) {
            return Err(GatewayError::InvalidInput);
        }
        timeout_at(
            Instant::now() + self.timeout,
            self.logout_inner(username, binding),
        )
        .await
        .map_err(|_| GatewayError::Deadline)?
    }

    fn validate_binding(&self, binding: Option<&NetworkBinding>) -> Result<(), GatewayError> {
        if binding.is_none() && !self.address.ip().is_loopback() {
            return Err(GatewayError::BindingRequired);
        }
        if let Some(binding) = binding {
            if binding.interface_index == 0
                || binding.interface_id.is_empty()
                || binding.source_ip.is_unspecified()
                || binding.source_ip.is_multicast()
                || binding.source_ip.is_broadcast()
                || (binding.source_ip.is_loopback() && !self.address.ip().is_loopback())
            {
                return Err(GatewayError::InvalidBinding);
            }
        }
        Ok(())
    }

    async fn status_inner(
        &self,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        let url = self.endpoint("/cgi-bin/rad_user_info", &[("callback", "jsonp")])?;
        let response = self.request(&url, binding).await?;
        response.ensure_success()?;
        parse_status(&response.body)
    }

    async fn login_inner(
        &self,
        username: &str,
        password: &str,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        let current = self.status_inner(binding).await?;
        if current.auth == AuthState::Authenticated {
            return if current.username.as_deref() == Some(username) {
                Ok(current)
            } else {
                Err(GatewayError::AccountMismatch)
            };
        }
        let ip = current.ip.ok_or(GatewayError::InvalidResponse)?.to_string();
        // Deliberately rediscovered for every operation: ac_id belongs to a
        // network, not to the process. Explicit administrative override is stable.
        let ac_id = self.discover_ac_id(binding).await?;
        let url = self.endpoint(
            "/cgi-bin/get_challenge",
            &[("callback", "jsonp"), ("username", username), ("ip", &ip)],
        )?;
        let response = self.request(&url, binding).await?;
        response.ensure_success()?;
        let challenge = Zeroizing::new(parse_jsonp::<Challenge>(&response.body)?.challenge);
        if !crypto::valid_challenge(&challenge) {
            return Err(GatewayError::InvalidResponse);
        }
        let info = crypto::login_info(username, password, &ip, &ac_id, &challenge)?;
        let hmd5 = crypto::password_hash(&challenge)?;
        let checksum = crypto::checksum(&challenge, username, &hmd5, &ac_id, &ip, &info);
        let encoded_password = format!("{{MD5}}{hmd5}");
        let url = self.endpoint(
            "/cgi-bin/srun_portal",
            &[
                ("callback", "jsonp"),
                ("action", "login"),
                ("username", username),
                ("password", &encoded_password),
                ("info", &info),
                ("chksum", &checksum),
                ("ac_id", &ac_id),
                ("ip", &ip),
                ("type", "1"),
                ("n", "200"),
            ],
        )?;
        let result = self.request(&url, binding).await.and_then(|r| {
            r.ensure_success()?;
            parse_portal(&r.body)
        });
        self.confirm(username, true, result, binding).await
    }

    async fn logout_inner(
        &self,
        username: &str,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        let current = self.status_inner(binding).await?;
        if current.auth == AuthState::Unauthenticated {
            return Ok(current);
        }
        if current.username.as_deref() != Some(username) {
            return Err(GatewayError::AccountMismatch);
        }
        let ip = current.ip.ok_or(GatewayError::InvalidResponse)?.to_string();
        let ac_id = self.discover_ac_id(binding).await?;
        let url = self.endpoint(
            "/cgi-bin/srun_portal",
            &[
                ("callback", "jsonp"),
                ("action", "logout"),
                ("username", username),
                ("ac_id", &ac_id),
                ("ip", &ip),
            ],
        )?;
        let result = self.request(&url, binding).await.and_then(|r| {
            r.ensure_success()?;
            parse_portal(&r.body)
        });
        self.confirm(username, false, result, binding).await
    }

    async fn confirm(
        &self,
        username: &str,
        online: bool,
        original: Result<(), GatewayError>,
        binding: Option<&NetworkBinding>,
    ) -> Result<GatewaySession, GatewayError> {
        let attempts = if original.is_ok() {
            CONFIRM_ATTEMPTS
        } else {
            1
        };
        for attempt in 0..attempts {
            if attempt > 0 {
                tokio::time::sleep(CONFIRM_INTERVAL).await;
            }
            match self.status_inner(binding).await {
                Ok(status)
                    if status.auth == AuthState::Authenticated
                        && status.username.as_deref() != Some(username) =>
                {
                    return Err(GatewayError::AccountMismatch)
                }
                Ok(status) if (status.auth == AuthState::Authenticated) == online => {
                    return Ok(status)
                }
                Ok(_) => {}
                Err(error) if original.is_ok() && attempt + 1 == attempts => return Err(error),
                Err(_) => {}
            }
        }
        original.and(Err(GatewayError::ConfirmationFailed))
    }

    async fn discover_ac_id(
        &self,
        binding: Option<&NetworkBinding>,
    ) -> Result<String, GatewayError> {
        if let Some(id) = &self.configured_ac_id {
            return Ok(id.clone());
        }
        let mut url = self.origin.clone();
        for _ in 0..4 {
            let response = self.request(&url, binding).await?;
            if response.status.is_redirection() {
                let location = response.location.ok_or(GatewayError::DiscoveryFailed)?;
                let next = url
                    .join(&location)
                    .map_err(|_| GatewayError::DiscoveryFailed)?;
                if !self.same_origin(&next) {
                    return Err(GatewayError::RedirectRejected);
                }
                let ids: Vec<_> = next
                    .query_pairs()
                    .filter(|(name, _)| name == "ac_id")
                    .map(|(_, value)| value.into_owned())
                    .collect();
                if ids.len() > 1 {
                    return Err(GatewayError::InvalidResponse);
                }
                if let Some(id) = ids.into_iter().next() {
                    return if valid_ac_id(&id) {
                        Ok(id)
                    } else {
                        Err(GatewayError::DiscoveryFailed)
                    };
                }
                // Discovery must never be turned into a login/logout request
                // by a redirect. Only fixed presentation pages may be visited;
                // a Location containing ac_id is inspected, not requested.
                if ![
                    "/",
                    "/index.html",
                    "/srun_portal_pc",
                    "/srun_portal",
                    "/srun_portal_success",
                    "/portal",
                ]
                .contains(&next.path())
                {
                    return Err(GatewayError::RedirectRejected);
                }
                url = next;
            } else {
                response.ensure_success()?;
                return Err(GatewayError::DiscoveryFailed);
            }
        }
        Err(GatewayError::DiscoveryFailed)
    }

    fn same_origin(&self, url: &Url) -> bool {
        url.scheme() == self.origin.scheme()
            && url.host_str() == self.origin.host_str()
            && url.port_or_known_default() == self.origin.port_or_known_default()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
    }

    fn endpoint(&self, path: &str, params: &[(&str, &str)]) -> Result<Url, GatewayError> {
        let mut url = self
            .origin
            .join(path)
            .map_err(|_| GatewayError::InvalidConfiguration)?;
        url.query_pairs_mut().extend_pairs(params.iter().copied());
        Ok(url)
    }

    async fn request(
        &self,
        url: &Url,
        binding: Option<&NetworkBinding>,
    ) -> Result<Response, GatewayError> {
        if !self.same_origin(url) {
            return Err(GatewayError::RedirectRejected);
        }
        let socket = TcpSocket::new_v4().map_err(|_| GatewayError::Transport)?;
        if let Some(binding) = binding {
            binding::apply(&socket, binding)?;
        }
        socket
            .set_nodelay(true)
            .map_err(|_| GatewayError::Transport)?;
        let stream = socket
            .connect(self.address.into())
            .await
            .map_err(|_| GatewayError::Transport)?;
        let (mut sender, connection) = http1::Builder::new()
            .max_buf_size(64 * 1024)
            .handshake::<_, Full<Bytes>>(TokioIo::new(stream))
            .await
            .map_err(|_| GatewayError::Transport)?;
        let _driver = ConnectionDriver(tokio::spawn(async move {
            let _ = connection.await;
        }));
        let target = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
        let request = Request::builder()
            .method("GET")
            .uri(target)
            .header(header::HOST, authority)
            .header(header::ACCEPT, "application/json, text/plain, */*")
            .header(header::CONNECTION, "close")
            .header(
                header::USER_AGENT,
                concat!("BitNet/", env!("CARGO_PKG_VERSION")),
            )
            .body(Full::new(Bytes::new()))
            .map_err(|_| GatewayError::InvalidInput)?;
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| GatewayError::Transport)?;
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .map(|v| v.to_str().map(str::to_owned))
            .transpose()
            .map_err(|_| GatewayError::InvalidResponse)?;
        if location.as_ref().is_some_and(|s| s.len() > MAX_LOCATION) {
            return Err(GatewayError::ResponseTooLarge);
        }
        if status.is_redirection() {
            return Ok(Response {
                status,
                location,
                body: Vec::new(),
            });
        }
        if !status.is_success() {
            return Err(GatewayError::HttpStatus(status.as_u16()));
        }
        if response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|len| len > MAX_BODY)
        {
            return Err(GatewayError::ResponseTooLarge);
        }
        let mut incoming = response.into_body();
        let mut body = Vec::new();
        while let Some(frame) = incoming.frame().await {
            let frame = frame.map_err(|_| GatewayError::Transport)?;
            if let Some(data) = frame.data_ref() {
                if data.len() > MAX_BODY - body.len() {
                    return Err(GatewayError::ResponseTooLarge);
                }
                body.extend_from_slice(data);
            }
        }
        Ok(Response {
            status,
            location,
            body,
        })
    }
}

struct ConnectionDriver(tokio::task::JoinHandle<()>);
impl Drop for ConnectionDriver {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct Response {
    status: StatusCode,
    location: Option<String>,
    body: Vec<u8>,
}
impl Response {
    fn ensure_success(&self) -> Result<(), GatewayError> {
        if self.status.is_redirection() {
            Err(GatewayError::RedirectRejected)
        } else if !self.status.is_success() {
            Err(GatewayError::HttpStatus(self.status.as_u16()))
        } else {
            Ok(())
        }
    }
}

fn valid_ac_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 10 && value.bytes().all(|b| b.is_ascii_digit())
}
fn valid_username(value: &str) -> bool {
    !value.trim().is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

fn parse_jsonp<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, GatewayError> {
    let text = std::str::from_utf8(body)
        .map_err(|_| GatewayError::InvalidResponse)?
        .trim();
    let text = text.strip_suffix(';').unwrap_or(text).trim_end();
    let json = text
        .strip_prefix("jsonp(")
        .and_then(|s| s.strip_suffix(')'))
        .ok_or(GatewayError::InvalidResponse)?;
    serde_json::from_str(json).map_err(|_| GatewayError::InvalidResponse)
}

#[derive(Deserialize)]
struct Challenge {
    challenge: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Number {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
}
impl Number {
    fn unsigned(&self) -> Option<u64> {
        match self {
            Self::Unsigned(v) => Some(*v),
            Self::Signed(v) => (*v).try_into().ok(),
            Self::Text(v) => v.parse().ok(),
            Self::Float(_) => None,
        }
    }
    fn money(&self) -> Option<f64> {
        let v = match self {
            Self::Unsigned(v) => *v as f64,
            Self::Signed(v) => *v as f64,
            Self::Float(v) => *v,
            Self::Text(v) => v.parse().ok()?,
        };
        v.is_finite().then_some(v)
    }
}

#[derive(Deserialize)]
struct Status {
    error: String,
    online_ip: String,
    #[serde(default)]
    res: Option<String>,
    #[serde(default)]
    user_name: Option<String>,
    #[serde(default)]
    sum_bytes: Option<Number>,
    #[serde(default)]
    sum_seconds: Option<Number>,
    #[serde(default)]
    user_balance: Option<Number>,
}

fn parse_status(body: &[u8]) -> Result<GatewaySession, GatewayError> {
    let state: Status = parse_jsonp(body)?;
    let ip: Ipv4Addr = state
        .online_ip
        .parse()
        .map_err(|_| GatewayError::InvalidResponse)?;
    if ip.is_unspecified() || ip.is_multicast() || ip.is_broadcast() {
        return Err(GatewayError::InvalidResponse);
    }
    if state.res.as_deref().is_some_and(|res| res != state.error) {
        return Err(GatewayError::InvalidResponse);
    }
    let username = state.user_name.filter(|name| !name.trim().is_empty());
    let auth = match state.error.as_str() {
        "ok" if username.is_some() => AuthState::Authenticated,
        "not_online_error" if username.is_none() => AuthState::Unauthenticated,
        "ok" | "not_online_error" => return Err(GatewayError::InvalidResponse),
        _ => return Err(GatewayError::UnknownGatewayStatus),
    };
    Ok(GatewaySession {
        auth,
        username,
        ip: Some(ip),
        bytes_used: state.sum_bytes.and_then(|v| v.unsigned()),
        online_seconds: state.sum_seconds.and_then(|v| v.unsigned()),
        balance: state.user_balance.and_then(|v| v.money()),
    })
}

#[derive(Deserialize)]
struct Portal {
    error: String,
    #[serde(default)]
    error_msg: Option<String>,
    #[serde(default)]
    ecode: Option<serde_json::Value>,
}
fn parse_portal(body: &[u8]) -> Result<(), GatewayError> {
    let portal: Portal = parse_jsonp(body)?;
    if portal.error == "ok" {
        return Ok(());
    }
    let message = portal
        .error_msg
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    let ecode = portal
        .ecode
        .as_ref()
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");
    if [
        "password is error",
        "password error",
        "invalid password",
        "incorrect password",
        "user does not exist",
    ]
    .iter()
    .any(|s| message.contains(s))
        || ["E2531", "E2553"].contains(&ecode)
    {
        return Err(GatewayError::CredentialsRejected);
    }
    Err(GatewayError::Rejected)
}
