use crate::{write_private_file, Result, Session, IDLE_TIMEOUT, LOOPBACK, SESSION_FILE};
use bitnet_core::{Command, ReplyData, Scope};
use bitnet_runtime::client::Client;
use http_body_util::Full;
use hyper::{
    body::{Bytes, Incoming},
    header::{self, HeaderMap},
    server::conn::http1,
    service::service_fn,
    Method, Request, Response, StatusCode,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{convert::Infallible, path::Path, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{watch, Mutex},
    task::JoinSet,
    time::{self, Instant},
};

const MAX_CONNECTIONS: usize = 8;
const HISTORY_LIMIT: u32 = 40;
const IPC_TIMEOUT: Duration = Duration::from_secs(4);

struct Viewer {
    session: Session,
    host: String,
    origin: String,
    client: Client,
    shutdown: watch::Sender<bool>,
    activity: watch::Sender<Instant>,
    cache: Mutex<Option<(Instant, Bytes)>>,
}

impl Viewer {
    fn new(session: Session, scope: Scope) -> Self {
        let host = session.address().to_string();
        Self {
            session,
            origin: format!("http://{host}"),
            host,
            client: Client::new(scope),
            shutdown: watch::channel(false).0,
            activity: watch::channel(Instant::now()).0,
            cache: Mutex::new(None),
        }
    }

    async fn response(&self, request: Request<Incoming>) -> Response<Full<Bytes>> {
        if request.uri().authority().is_some()
            || request.uri().scheme().is_some()
            || !allowed_headers(request.headers(), &self.host, &self.origin)
        {
            return response(StatusCode::FORBIDDEN, "text/plain", "Forbidden");
        }

        let method = request.method();
        let path = request.uri().path();
        match (method, path) {
            (&Method::GET, "/") => response(
                StatusCode::OK,
                "text/html; charset=utf-8",
                include_str!("index.html"),
            ),
            (&Method::GET, "/app.js") => response(
                StatusCode::OK,
                "text/javascript; charset=utf-8",
                include_str!("app.js"),
            ),
            (&Method::GET, "/app.css") => response(
                StatusCode::OK,
                "text/css; charset=utf-8",
                include_str!("app.css"),
            ),
            (&Method::GET, "/api/history")
            | (&Method::POST, "/api/stop")
            | (&Method::GET, "/control/ready")
            | (&Method::POST, "/control/stop") => {
                let expected = if path.starts_with("/control/") {
                    &self.session.control
                } else {
                    &self.session.view
                };
                if !has_token(request.headers(), expected) {
                    return response(StatusCode::UNAUTHORIZED, "text/plain", "Session expired");
                }
                if path == "/api/history" {
                    let query = request.uri().query();
                    let before_id = match history_cursor(query) {
                        Ok(before_id) => before_id,
                        Err(()) => {
                            return response(
                                StatusCode::BAD_REQUEST,
                                "text/plain",
                                "Invalid history cursor",
                            )
                        }
                    };
                    if query != Some("watch") {
                        self.activity.send_replace(Instant::now());
                    }
                    return self.history(before_id).await;
                }
                self.activity.send_replace(Instant::now());
                if path.ends_with("/stop") {
                    self.shutdown.send_replace(true);
                }
                response(StatusCode::NO_CONTENT, "text/plain", "")
            }
            _ => response(StatusCode::NOT_FOUND, "text/plain", "Not found"),
        }
    }

    async fn history(&self, before_id: Option<i64>) -> Response<Full<Bytes>> {
        if before_id.is_none() {
            if let Some((at, body)) = self.cache.lock().await.as_ref() {
                if at.elapsed() < Duration::from_secs(1) {
                    return response(StatusCode::OK, "application/json", body.clone());
                }
            }
        }
        let command = match before_id {
            Some(before_id) => Command::HistoryBefore {
                limit: HISTORY_LIMIT,
                before_id,
            },
            None => Command::History {
                limit: HISTORY_LIMIT,
            },
        };
        let result = time::timeout(IPC_TIMEOUT, async {
            if self.client.effective_scope() != self.client.scope {
                return None;
            }
            let reply = self.client.command(command).await.ok()?;
            if let ReplyData::History(events) = reply {
                serde_json::to_vec(&events).ok().map(Bytes::from)
            } else {
                None
            }
        })
        .await;
        match result {
            Ok(Some(body)) => {
                if before_id.is_none() {
                    *self.cache.lock().await = Some((Instant::now(), body.clone()));
                }
                response(StatusCode::OK, "application/json", body)
            }
            _ => response(
                StatusCode::SERVICE_UNAVAILABLE,
                "text/plain",
                "History unavailable for this scope",
            ),
        }
    }
}

fn history_cursor(query: Option<&str>) -> std::result::Result<Option<i64>, ()> {
    match query {
        None | Some("watch") => Ok(None),
        Some(query) => query
            .strip_prefix("before=")
            .and_then(|id| id.parse::<i64>().ok())
            .filter(|id| *id > 0)
            .map(Some)
            .ok_or(()),
    }
}

fn allowed_headers(headers: &HeaderMap, host: &str, origin: &str) -> bool {
    let mut hosts = headers.get_all(header::HOST).iter();
    let mut origins = headers.get_all(header::ORIGIN).iter();
    hosts.next().is_some_and(|value| value == host)
        && hosts.next().is_none()
        && origins.next().is_none_or(|value| value == origin)
        && origins.next().is_none()
        && !headers.contains_key(header::TRANSFER_ENCODING)
        && headers
            .get(header::CONTENT_LENGTH)
            .is_none_or(|value| value == "0")
}

fn has_token(headers: &HeaderMap, expected: &str) -> bool {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let token = values
        .next()
        .and_then(|value| value.as_bytes().strip_prefix(b"Bearer "));
    token.is_some_and(|token| bool::from(token.ct_eq(expected.as_bytes())))
        && values.next().is_none()
}

fn response(
    status: StatusCode,
    mime: &'static str,
    body: impl Into<Bytes>,
) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CACHE_CONTROL, "no-store")
        .header("Content-Security-Policy", "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'")
        .header("X-Frame-Options", "DENY")
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cross-Origin-Resource-Policy", "same-origin")
        .body(Full::new(body.into()))
        .expect("Static HTTP response headers")
}

async fn connection(socket: TcpStream, viewer: Arc<Viewer>) {
    let service = service_fn(move |request| {
        let viewer = viewer.clone();
        async move { Ok::<_, Infallible>(viewer.response(request).await) }
    });
    let _ = time::timeout(
        Duration::from_secs(9),
        http1::Builder::new()
            .keep_alive(false)
            .max_headers(32)
            .max_buf_size(16 * 1024)
            .timer(TokioTimer::new())
            .header_read_timeout(Duration::from_secs(2))
            .serve_connection(TokioIo::new(socket), service),
    )
    .await;
}

pub(super) async fn serve(directory: &Path, scope: Scope) -> Result<()> {
    let listener = TcpListener::bind((LOOPBACK, 0)).await?;
    let session = Session::new(listener.local_addr()?.port())?;
    let viewer = Arc::new(Viewer::new(session, scope));
    let mut shutdown = viewer.shutdown.subscribe();
    let mut idle_check = time::interval(Duration::from_secs(1));
    let mut tasks = JoinSet::new();
    write_private_file(
        &directory.join(SESSION_FILE),
        &serde_json::to_vec(&viewer.session)?,
    )?;
    loop {
        tokio::select! {
            biased;
            _ = shutdown.changed() => break,
            _ = idle_check.tick() => {
                if viewer.activity.borrow().elapsed() >= IDLE_TIMEOUT {
                    break;
                }
            }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            result = listener.accept(), if tasks.len() < MAX_CONNECTIONS => {
                let (socket, peer) = result?;
                if peer.ip().is_loopback() {
                    tasks.spawn(connection(socket, viewer.clone()));
                }
            }
        }
    }
    drop(listener);
    // Let the connection that requested shutdown deliver its HTTP reply.
    let _ = time::timeout(Duration::from_secs(1), async {
        while tasks.join_next().await.is_some() {}
    })
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyper::header::HeaderValue;

    #[test]
    fn requires_exact_host_and_rejects_cross_origin() {
        let mut headers = HeaderMap::new();
        headers.insert(header::HOST, HeaderValue::from_static("127.0.0.1:43210"));
        assert!(allowed_headers(
            &headers,
            "127.0.0.1:43210",
            "http://127.0.0.1:43210"
        ));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://example.com"),
        );
        assert!(!allowed_headers(
            &headers,
            "127.0.0.1:43210",
            "http://127.0.0.1:43210"
        ));
        headers.remove(header::ORIGIN);
        headers.append(header::HOST, HeaderValue::from_static("127.0.0.1:43210"));
        assert!(!allowed_headers(
            &headers,
            "127.0.0.1:43210",
            "http://127.0.0.1:43210"
        ));
    }

    #[test]
    fn accepts_only_one_matching_bearer() {
        let mut headers = HeaderMap::new();
        assert!(!has_token(&headers, "fixture"));
        headers.insert(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture"),
        );
        assert!(!has_token(&headers, "other"));
        assert!(has_token(&headers, "fixture"));
        headers.append(
            header::AUTHORIZATION,
            HeaderValue::from_static("Bearer fixture"),
        );
        assert!(!has_token(&headers, "fixture"));
    }
}
