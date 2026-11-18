use crate::app::{display_text, scope_label, App, Page, Request};
use bitnet_core::{now_secs, HistoryEvent};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
    Frame,
};

const EMPHASIS: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);

pub(super) fn draw(frame: &mut Frame<'_>, app: &App, request: Option<Request>) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        frame.render_widget(
            Paragraph::new("请放大终端 · q 退出").wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let [header, content, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(2),
    ])
    .areas(area);
    let title = match app.page {
        Page::Status => "连接状态",
        Page::History => "事件日志",
    };
    let simulation = app.status.as_ref().is_some_and(|status| status.simulation);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("BitNet", EMPHASIS),
            Span::raw(format!("   {title}")),
            Span::styled(
                if simulation { "   模拟模式" } else { "" },
                Style::new().fg(Color::Yellow),
            ),
        ])),
        header,
    );
    let mut hint = match app.page {
        Page::Status => {
            status(frame, app, content);
            "Tab 日志 · r 刷新 · c 检测 · l 登录 · a 暂停/恢复 · s 启动 · q 退出"
        }
        Page::History => {
            history(frame, app, content);
            if app.filter_editing {
                "Enter / Esc 结束编辑"
            } else if app.history_detail {
                "Enter / Esc 返回日志 · q 退出"
            } else {
                "↑↓ 选择 · PgUp/PgDn 翻页 · Enter 详情 · w Web UI · r 刷新 · Tab 状态"
            }
        }
    }
    .to_owned();
    if !app.filter_editing {
        hint.push_str(if app.addresses_revealed {
            " · p 隐藏 IP"
        } else {
            " · p 显示 IP"
        });
    }
    let notice = request
        .filter(|request| *request != Request::Status)
        .map(|request| format!("正在{}…", request.label()))
        .or_else(|| app.notice.clone())
        .unwrap_or_default();
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                display_text(&notice, 200, app.addresses_revealed),
                EMPHASIS,
            )),
            Line::raw(hint),
        ]),
        footer,
    );
}

fn status(frame: &mut Frame<'_>, app: &App, content: Rect) {
    let lines = if let Some(status) = &app.status {
        status
            .fields
            .iter()
            .map(|(label, value)| {
                let style = if *label == "认证" && status.online {
                    Style::new().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                };
                Line::from(vec![
                    Span::styled(*label, EMPHASIS),
                    Span::raw("  "),
                    Span::styled(display_text(value, 200, app.addresses_revealed), style),
                ])
            })
            .collect()
    } else {
        vec![
            Line::styled("引擎未连接", EMPHASIS),
            Line::raw(format!("请求范围  {}", scope_label(app.requested_scope))),
            Line::raw(display_text(
                app.status_error.as_deref().unwrap_or("正在读取状态…"),
                200,
                app.addresses_revealed,
            )),
            Line::raw("按 s 启动个人引擎，按 r 重试。"),
        ]
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(" 连接 "))
            .wrap(Wrap { trim: false }),
        content,
    );
}

fn history(frame: &mut Frame<'_>, app: &App, content: Rect) {
    let [filter, list] =
        Layout::vertical([Constraint::Length(2), Constraint::Min(1)]).areas(content);
    let events = app.visible_history();
    let total = app.history.as_ref().map_or(0, Vec::len);
    let size = app.history_page_size();
    let page = app.history_cursor / size;
    let filter_label = if app.filter_editing {
        "筛选中"
    } else {
        "/ 筛选"
    };
    let filter_value = if app.filter.is_empty() {
        "全部".to_owned()
    } else {
        display_text(&app.filter, 64, app.addresses_revealed)
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(filter_label, EMPHASIS),
                Span::raw(format!("  {filter_value}   ·   x 清除")),
            ]),
            Line::raw(format!(
                "匹配 {} / 最近 {} 条   ·   第 {} / {} 页",
                events.len(),
                total,
                page + 1,
                events.len().div_ceil(size).max(1),
            )),
        ]),
        filter,
    );
    if let Some(event) = events
        .get(app.history_cursor)
        .filter(|_| app.history_detail)
    {
        detail(frame, list, event, app.addresses_revealed);
        return;
    }
    let lines = if let Some(error) = app.history_error.as_ref() {
        vec![
            Line::raw(format!(
                "读取失败 · {}",
                display_text(error, 200, app.addresses_revealed)
            )),
            Line::raw("按 r 重试。"),
        ]
    } else if app.history.is_none() {
        vec![Line::raw("正在读取最近的日志…")]
    } else if events.is_empty() {
        vec![Line::raw("没有符合条件的日志。")]
    } else {
        events
            .iter()
            .enumerate()
            .skip(page * size)
            .take(size)
            .map(|(index, event)| {
                let label = format!(
                    "{} #{:<4} · {} · {}",
                    age(event.at),
                    event.id,
                    display_text(event_label(&event.kind), 32, app.addresses_revealed),
                    display_text(&event.message, 180, app.addresses_revealed)
                );
                if index == app.history_cursor {
                    Line::styled(format!("› {label}"), EMPHASIS)
                } else {
                    Line::raw(format!("  {label}"))
                }
            })
            .collect()
    };
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title(" 最近事件 ")),
        list,
    );
}

fn event_label(kind: &str) -> &str {
    match kind {
        "reconnect" => "会话重连成功",
        "login_success" => "校园网登录成功",
        "login_failure" => "校园网登录失败",
        "environment_observed" => "网络状态已更新",
        "online_segment" => "在线时段已记录",
        "session_lost" => "认证失效，等待重连",
        "health_degraded" => "认证有效，网络异常",
        _ => kind,
    }
}

fn detail(frame: &mut Frame<'_>, area: Rect, event: &HistoryEvent, addresses_revealed: bool) {
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!(
                    "#{}  {}",
                    event.id,
                    display_text(event_label(&event.kind), 48, addresses_revealed)
                ),
                EMPHASIS,
            ),
            Span::raw(format!("  ·  {}", age(event.at))),
        ]),
        Line::raw(display_text(&event.message, 1500, addresses_revealed)),
    ];
    if let Some(ms) = event.duration_ms {
        lines.push(Line::raw(format!("耗时 {ms} ms")));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::bordered().title(" 事件详情 "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn age(at: u64) -> String {
    match now_secs().saturating_sub(at) / 60 {
        0 => "刚刚".into(),
        minutes => format!("{minutes} 分钟前"),
    }
}
