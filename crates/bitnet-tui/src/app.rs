use bitnet_core::{
    AppError, AuthState, Command, EngineState, Environment, ErrorCode, Health, HistoryEvent,
    Ipv6Status, ReplyData, Scope, Snapshot,
};
use bitnet_runtime::client::Client;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

const HISTORY_LIMIT: u32 = 100;
const FILTER_LIMIT: usize = 64;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Request {
    Status,
    Start,
    Check,
    Login,
    Pause,
    Resume,
    History,
    OpenWeb,
}

impl Request {
    pub(super) async fn send(self, client: &Client) -> Result<ReplyData, AppError> {
        let command = match self {
            Self::Status => Command::Status,
            Self::Start => {
                client.ensure_running().await?;
                Command::Status
            }
            Self::Check => Command::Check,
            Self::Login => Command::Login {
                account_id: None,
                force: false,
                confirmed: false,
            },
            Self::Pause => Command::Pause,
            Self::Resume => Command::Resume,
            Self::History => Command::History {
                limit: HISTORY_LIMIT,
            },
            Self::OpenWeb => {
                let client = client.clone();
                tokio::task::spawn_blocking(move || {
                    bitnet_launcher::web(
                        client.effective_scope(),
                        bitnet_launcher::WebAction::Open,
                        false,
                    )
                })
                .await
                .map_err(|_| AppError::new(ErrorCode::Internal, "日志 Web UI 启动任务异常退出"))?
                .map_err(|error| AppError::new(ErrorCode::Unavailable, error.to_string()))?;
                return Ok(ReplyData::Ok {
                    message: "日志 Web UI 已打开".to_owned(),
                });
            }
        };
        client.command(command).await
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Status => "读取状态",
            Self::Start => "连接引擎",
            Self::Check => "检测网络",
            Self::Login => "登录校园网",
            Self::Pause => "暂停自动认证",
            Self::Resume => "恢复自动认证",
            Self::History => "读取日志",
            Self::OpenWeb => "打开日志 Web UI",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Page {
    Status,
    History,
}

#[derive(PartialEq, Eq)]
pub(super) struct StatusView {
    pub fields: Vec<(&'static str, String)>,
    automatic: bool,
    paused: bool,
    pub simulation: bool,
    pub online: bool,
}

impl StatusView {
    fn from_snapshot(snapshot: &Snapshot) -> Self {
        let adapter = snapshot.active_binding.as_ref().map(|binding| {
            let name = snapshot
                .interfaces
                .iter()
                .find(|adapter| adapter.id == binding.interface_id)
                .map(|adapter| adapter.name.clone())
                .unwrap_or_else(|| format!("#{}", binding.interface_index));
            format!("{name} · {}", binding.source_ip)
        });
        let automatic = snapshot.settings.automatic;
        let paused = snapshot.settings.paused || snapshot.verification_hold;
        let mut fields = vec![
            ("版本", snapshot.app_version.clone()),
            ("引擎", scope_label(snapshot.scope).to_owned()),
            ("状态", engine_state(&snapshot.state).to_owned()),
            ("网络", environment(&snapshot.environment).to_owned()),
            ("认证", auth_state(&snapshot.session.auth).to_owned()),
            ("网卡", adapter.unwrap_or_else(|| "—".to_owned())),
            (
                "网络信任",
                if snapshot.network_trusted {
                    "已确认"
                } else {
                    "未确认"
                }
                .to_owned(),
            ),
            (
                "自动认证",
                if paused {
                    "已暂停"
                } else if automatic {
                    "已开启"
                } else {
                    "未开启"
                }
                .to_owned(),
            ),
            (
                "网络检测",
                format!(
                    "互联网 {} · DNS {} · IPv6 {}",
                    health(&snapshot.health.internet),
                    health(&snapshot.health.dns),
                    ipv6_health(snapshot.health.ipv6_status)
                ),
            ),
        ];
        if let Some(account) = snapshot.accounts.iter().find(|account| {
            Some(account.id.as_str()) == snapshot.settings.selected_account.as_deref()
        }) {
            fields.push(("已选账户", account.username.clone()));
        }
        if let Some(error) = &snapshot.last_error {
            fields.push(("最近错误", error.message.clone()));
        }
        Self {
            fields,
            automatic,
            paused,
            simulation: snapshot
                .capabilities
                .iter()
                .any(|item| item == "simulation"),
            online: snapshot.session.auth == AuthState::Authenticated,
        }
    }
}

pub(super) struct App {
    pub requested_scope: Scope,
    pub status: Option<StatusView>,
    pub status_error: Option<String>,
    pub notice: Option<String>,
    pub page: Page,
    pub history: Option<Vec<HistoryEvent>>,
    pub history_error: Option<String>,
    pub history_detail: bool,
    pub history_cursor: usize,
    pub filter: String,
    pub filter_editing: bool,
    pub addresses_revealed: bool,
    height: u16,
    exit: bool,
}

impl App {
    pub(super) fn new(requested_scope: Scope) -> Self {
        Self {
            requested_scope,
            status: None,
            status_error: None,
            notice: None,
            page: Page::Status,
            history: None,
            history_error: None,
            history_detail: false,
            history_cursor: 0,
            filter: String::new(),
            filter_editing: false,
            addresses_revealed: false,
            height: 0,
            exit: false,
        }
    }

    pub(super) fn resize(&mut self, height: u16) {
        self.height = height;
    }

    pub(super) fn history_page_size(&self) -> usize {
        usize::from(self.height.saturating_sub(8).max(1))
    }

    pub(super) fn visible_history(&self) -> Vec<&HistoryEvent> {
        let query = self.filter.trim().to_lowercase();
        self.history
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|event| {
                query.is_empty()
                    || event.kind.to_lowercase().contains(&query)
                    || event.message.to_lowercase().contains(&query)
                    || event.id.to_string().contains(&query)
            })
            .collect()
    }

    pub(super) fn exiting(&self) -> bool {
        self.exit
    }

    pub(super) fn pending_history(&self) -> Option<Request> {
        (self.page == Page::History && self.history.is_none() && self.history_error.is_none())
            .then_some(Request::History)
    }

    pub(super) fn poll_request(&self) -> Option<Request> {
        match self.page {
            Page::Status => Some(Request::Status),
            Page::History => self.pending_history(),
        }
    }

    pub(super) fn can_repeat(&self, key: KeyEvent) -> bool {
        self.page == Page::History
            && !self.history_detail
            && !self.filter_editing
            && matches!(
                key.code,
                KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown
            )
    }

    pub(super) fn key(&mut self, key: KeyEvent, busy: bool) -> Option<Request> {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.exit = true;
            return None;
        }
        if self.filter_editing {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => self.filter_editing = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c)
                    if !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                        && !is_unsafe_char(c)
                        && self.filter.chars().count() < FILTER_LIMIT =>
                {
                    self.filter.push(c);
                }
                _ => {}
            }
            self.history_cursor = 0;
            return None;
        }
        if self.history_detail && key.code == KeyCode::Esc {
            self.history_detail = false;
            return None;
        }
        if key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char('p') => {
                self.addresses_revealed = !self.addresses_revealed;
                return None;
            }
            KeyCode::Char('q') | KeyCode::Esc => {
                self.exit = true;
                return None;
            }
            KeyCode::Tab | KeyCode::Char('h') => {
                self.page = match self.page {
                    Page::Status => Page::History,
                    Page::History => Page::Status,
                };
                self.notice = None;
                self.history_detail = false;
                return (!busy).then(|| self.poll_request()).flatten();
            }
            _ => {}
        }
        if self.page == Page::History {
            return self.history_key(key.code, busy);
        }
        if busy {
            return None;
        }
        match key.code {
            KeyCode::Char('r') => Some(Request::Status),
            KeyCode::Char('s') => Some(Request::Start),
            KeyCode::Char('c') if self.status.is_some() => Some(Request::Check),
            KeyCode::Char('l') if self.status.is_some() => Some(Request::Login),
            KeyCode::Char('a') => match &self.status {
                Some(status) if status.automatic => Some(if status.paused {
                    Request::Resume
                } else {
                    Request::Pause
                }),
                Some(_) => {
                    self.notice = Some("自动认证尚未开启，请在 GUI 或 bitctl 中配置。".to_owned());
                    None
                }
                None => None,
            },
            _ => None,
        }
    }

    fn history_key(&mut self, key: KeyCode, busy: bool) -> Option<Request> {
        if self.history_detail {
            if key == KeyCode::Enter {
                self.history_detail = false;
            }
            return None;
        }
        let count = self.visible_history().len();
        let last = count.saturating_sub(1);
        match key {
            KeyCode::Char('r') if !busy => return Some(Request::History),
            KeyCode::Char('w') if !busy => return Some(Request::OpenWeb),
            KeyCode::Char('/') => self.filter_editing = true,
            KeyCode::Char('x') => {
                self.filter.clear();
                self.history_cursor = 0;
            }
            KeyCode::Up => self.history_cursor = self.history_cursor.saturating_sub(1),
            KeyCode::Down => self.history_cursor = (self.history_cursor + 1).min(last),
            KeyCode::PageUp => {
                self.history_cursor = self.history_cursor.saturating_sub(self.history_page_size())
            }
            KeyCode::PageDown => {
                self.history_cursor = (self.history_cursor + self.history_page_size()).min(last)
            }
            KeyCode::Home => self.history_cursor = 0,
            KeyCode::End => self.history_cursor = last,
            KeyCode::Enter if count > 0 => self.history_detail = true,
            _ => {}
        }
        None
    }

    pub(super) fn complete(
        &mut self,
        request: Request,
        response: Result<ReplyData, AppError>,
    ) -> bool {
        if request == Request::OpenWeb {
            self.notice = Some(match response {
                Ok(ReplyData::Ok { message }) => message,
                Ok(_) => "日志 Web UI 返回了未知结果。".to_owned(),
                Err(error) => error.message,
            });
            return true;
        }
        if request == Request::History {
            match response {
                Ok(ReplyData::History(events)) => {
                    self.history = Some(events);
                    self.history_error = None;
                    self.history_cursor = self
                        .history_cursor
                        .min(self.visible_history().len().saturating_sub(1));
                }
                Ok(_) => self.history_error = Some("引擎返回了非日志结果。".to_owned()),
                Err(error) => self.history_error = Some(error.message),
            }
            return self.page == Page::History;
        }
        let result = response.and_then(ReplyData::snapshot);
        match result {
            Ok(snapshot) => {
                let status = StatusView::from_snapshot(&snapshot);
                let changed = self.status.as_ref() != Some(&status) || self.status_error.is_some();
                self.status = Some(status);
                self.status_error = None;
                if request != Request::Status {
                    self.notice = Some(format!("{}完成", request.label()));
                }
                changed || request != Request::Status
            }
            Err(error) => {
                let message =
                    if request == Request::Login && error.code == ErrorCode::ConfirmationRequired {
                        "当前网络尚未确认。请用 GUI 或 bitctl 确认网络后再登录。".to_owned()
                    } else {
                        error.message
                    };
                if request == Request::Status || request == Request::Start {
                    let changed =
                        self.status.is_some() || self.status_error.as_deref() != Some(&message);
                    self.status = None;
                    self.status_error = Some(message);
                    changed || request == Request::Start
                } else {
                    self.notice = Some(format!("{}失败 · {message}", request.label()));
                    true
                }
            }
        }
    }
}

fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

pub(super) fn display_text(value: &str, limit: usize, addresses_revealed: bool) -> String {
    let masked;
    let value = if addresses_revealed {
        value
    } else {
        masked = mask_addresses(value);
        &masked
    };
    let mut text = String::new();
    let mut chars = value.chars();
    for c in chars.by_ref().take(limit) {
        if is_unsafe_char(c) {
            text.push(' ');
        } else {
            text.push(c);
        }
    }
    if chars.next().is_some() {
        text.push('…');
    }
    text
}

fn mask_addresses(value: &str) -> String {
    let address_char = |c: char| c.is_ascii_hexdigit() || matches!(c, ':' | '.');
    let mut result = String::with_capacity(value.len());
    for run in value.split_inclusive(|c| !address_char(c)) {
        let candidate = run.trim_end_matches(|c| !address_char(c));
        let tail = &run[candidate.len()..];
        let address = candidate.trim_end_matches('.');
        let punctuation = &candidate[address.len()..];
        let leading = usize::from(address.starts_with(':'));
        let trailing = address
            .len()
            .saturating_sub(usize::from(address.ends_with(':')));
        let label_end = address.find(':').map_or(0, |end| end + 1);
        let parsed = [
            (0, address.len()),
            (leading, address.len()),
            (0, trailing),
            (leading, trailing),
            (label_end, address.len()),
            (label_end, trailing),
        ]
        .into_iter()
        .filter_map(|(start, end)| address.get(start..end).map(|part| (start, end, part)))
        .find(|(_, _, part)| part.parse::<IpAddr>().is_ok() || part.parse::<SocketAddr>().is_ok());
        if let Some((start, end, part)) = parsed {
            result.push_str(&address[..start]);
            result.extend(std::iter::repeat_n('•', part.len()));
            result.push_str(&address[end..]);
        } else if address.contains('.') {
            for (index, part) in address.split(':').enumerate() {
                if index != 0 {
                    result.push(':');
                }
                if part.parse::<Ipv4Addr>().is_ok() {
                    result.extend(std::iter::repeat_n('•', part.len()));
                } else {
                    result.push_str(part);
                }
            }
        } else {
            result.push_str(address);
        }
        result.push_str(punctuation);
        result.push_str(tail);
    }
    result
}

pub(super) fn scope_label(scope: Scope) -> &'static str {
    match scope {
        Scope::User => "当前用户",
        Scope::System => "系统后台",
    }
}

fn engine_state(state: &EngineState) -> &'static str {
    match state {
        EngineState::Initializing => "正在初始化",
        EngineState::Checking => "正在检查",
        EngineState::Idle => "等待连接",
        EngineState::LoggingIn => "正在登录",
        EngineState::Online => "在线",
        EngineState::Retrying => "等待重试",
        EngineState::Paused => "暂停",
        EngineState::NeedsCredentials => "缺少账户",
        EngineState::CredentialsUnavailable => "无法读取凭据",
        EngineState::AuthError => "认证失败",
        EngineState::NetworkDegraded => "网络异常",
        EngineState::Stopping => "正在停止",
    }
}

fn environment(value: &Environment) -> &'static str {
    match value {
        Environment::Unknown => "正在识别",
        Environment::NoNetwork => "无网络",
        Environment::NonCampus => "外部网络",
        Environment::Campus => "校园直连",
        Environment::RoutedCampus => "经路由连接校园网",
    }
}

fn auth_state(value: &AuthState) -> &'static str {
    match value {
        AuthState::Unknown => "未知",
        AuthState::Unauthenticated => "未认证",
        AuthState::Authenticated => "已认证",
    }
}

fn health(value: &Health) -> &'static str {
    match value {
        Health::Unknown => "未测",
        Health::Healthy => "正常",
        Health::Degraded => "异常",
        Health::Unavailable => "不可用",
    }
}

fn ipv6_health(value: Ipv6Status) -> &'static str {
    match value {
        Ipv6Status::Unknown => "未测",
        Ipv6Status::Disabled => "未启用",
        Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable => "无互联网",
        Ipv6Status::GlobalNotPreferred => "非首选",
        Ipv6Status::Normal => "正常",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_masking_covers_status_and_log_text_before_truncation() {
        assert_eq!(
            display_text("源地址10.0.13.24。", 100, false),
            "源地址••••••••••。"
        );
        assert_eq!(display_text("IP: 10.0.0.1:81", 6, false), "IP: ••…");
        assert_eq!(
            display_text("http://[2001:db8::1]:80/check", 100, false),
            "http://[•••••••••••]:80/check"
        );
        assert_eq!(display_text("DNS:fe80::1%3", 100, false), "DNS:•••••••%3");
        assert_eq!(display_text("IPv6:::1,", 100, false), "IPv6:•••,");
        assert_eq!(display_text("BitNet 0.1.2", 100, false), "BitNet 0.1.2");
        assert_eq!(display_text("10.0.0.1", 100, true), "10.0.0.1");
    }
}
