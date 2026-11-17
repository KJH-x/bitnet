use super::*;
use iced::{
    advanced::{layout, mouse, renderer, widget::Tree, Layout, Widget},
    alignment,
    widget::{
        button, checkbox, column, container, pick_list, row, rule, scrollable, stack, text,
        text_editor, text_input, Space, Widget as _,
    },
    Color, Length, Rectangle, Size,
};

pub(super) fn action_style(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    let primary = theme.palette().primary;
    match status {
        button::Status::Active => {
            style.background = None;
            style.text_color = theme.palette().background.base.text;
            style.border = style.border.color(primary.base.color).width(1);
        }
        button::Status::Hovered | button::Status::Pressed => {
            style = button::primary(theme, status);
            style.border = style.border.color(primary.base.color).width(1);
        }
        button::Status::Disabled => {}
    }
    style
}

fn card<'a, W>(content: W) -> iced::widget::Container<'a, W> {
    container(content)
        .padding(16)
        .width(Length::Fill)
        .style(container::rounded_box)
}
fn pair<'a>(label: &'a str, value: impl ToString) -> iced::widget::Row<Element<'a, Message>> {
    row![
        text(label).width(180),
        text(value.to_string()).width(Length::Fill)
    ]
    .spacing(12)
}

// Preserve the underlying page and its widget state while a modal owns input.
struct Inactive<'a> {
    content: Element<'a, Message>,
    inactive: bool,
}

impl iced::advanced::widget::Meta for Inactive<'_> {}

impl Widget<Message, Theme, iced::Renderer> for Inactive<'_> {
    fn size(&self) -> Size<Length> {
        self.content.size()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) {
        self.content.layout(&mut tree.children[0], renderer, limits);
        tree.size = tree.children[0].size;
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            if self.inactive {
                mouse::Cursor::Unavailable
            } else {
                cursor
            },
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &iced::Event,
        layout: Layout,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if !self.inactive {
            self.content.update(
                &mut tree.children[0],
                event,
                layout,
                cursor,
                renderer,
                shell,
                viewport,
            );
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        if !self.inactive {
            self.content
                .operate(&mut tree.children[0], layout, viewport, renderer, operation);
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if self.inactive {
            mouse::Interaction::None
        } else {
            self.content
                .mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
        window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'a, Message, Theme, iced::Renderer>> {
        if self.inactive {
            Vec::new()
        } else {
            self.content.overlay(
                &mut tree.children[0],
                layout,
                renderer,
                viewport,
                translation,
                window,
            )
        }
    }
}

fn layer<'a>(content: Element<'a, Message>, inactive: bool) -> Element<'a, Message> {
    Inactive { content, inactive }.boxed()
}

fn scrim(_: &Theme) -> container::Style {
    container::Style::default().background(Color::from_rgba(0.0, 0.0, 0.0, 0.58))
}

fn format_elapsed_minutes(elapsed: u64, english: bool) -> String {
    let minutes = elapsed / 60;
    match (minutes, english) {
        (0, false) => "刚刚".into(),
        (0, true) => "Just now".into(),
        (1, true) => "1 minute ago".into(),
        (_, true) => format!("{minutes} minutes ago"),
        (_, false) => format!("{minutes} 分钟前"),
    }
}

fn format_history_duration(ms: u64, english: bool) -> String {
    let seconds = ms / 1000;
    let (hours, minutes) = (seconds / 3600, seconds / 60 % 60);
    match english {
        true if hours > 0 => format!("{hours} h {minutes} min"),
        false if hours > 0 => format!("{hours} 小时 {minutes} 分钟"),
        true if minutes > 0 => format!("{minutes} min {} s", seconds % 60),
        false if minutes > 0 => format!("{minutes} 分钟 {} 秒", seconds % 60),
        true if seconds > 0 => format!("{:.1} s", ms as f64 / 1_000.0),
        false if seconds > 0 => format!("{:.1} 秒", ms as f64 / 1_000.0),
        true => format!("{ms} ms"),
        false => format!("{ms} 毫秒"),
    }
}

impl App {
    fn relative_time(&self, timestamp: Option<u64>) -> String {
        timestamp.map_or_else(
            || "—".into(),
            |at| format_elapsed_minutes(bitnet_core::now_secs().saturating_sub(at), self.english()),
        )
    }

    pub(super) fn shell(&self) -> Element<'_, Message> {
        let content = self.main_shell();
        let content = if self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot
                .capabilities
                .iter()
                .any(|item| item == "simulation")
        }) {
            column![
                container(
                    text(self.t(
                        "模拟模式：不会连接校园网，账户与流量为测试数据",
                        "Simulation: no campus connection; accounts and traffic are test data"
                    ))
                    .size(16)
                )
                .padding([14, 20])
                .width(Length::Fill)
                .style(container::danger),
                content
            ]
            .height(Length::Fill)
            .boxed()
        } else {
            content
        };
        let confirmation = self.confirmation.as_ref();
        let health_editor = self.health_editor.as_deref();
        let mut layers = stack([layer(
            content,
            self.modal.is_some()
                || self.quick_login.is_some()
                || confirmation.is_some()
                || health_editor.is_some(),
        )]);
        if let Some(page) = self.modal {
            let content = match page {
                Page::Networks => Some(self.networks_view()),
                Page::History => Some(self.history_view()),
                _ => None,
            };
            if let Some(content) = content {
                layers = layers.push(layer(
                    self.dialog(page.label(self.english()), content, Message::Cancel, 880),
                    confirmation.is_some() || health_editor.is_some(),
                ));
            }
        }
        if let Some(entries) = health_editor {
            layers = layers.push(layer(
                self.health_editor_view(entries),
                confirmation.is_some(),
            ));
        }
        if let Some(draft) = &self.quick_login {
            layers = layers.push(layer(
                self.dialog(
                    self.t("登录校园网", "Campus sign-in"),
                    self.quick_login_view(draft),
                    Message::Cancel,
                    480,
                ),
                confirmation.is_some(),
            ));
        }
        if let Some(confirmation) = confirmation {
            layers = layers.push(self.confirmation_view(confirmation));
        }
        layers.boxed()
    }

    fn dialog<'a>(
        &'a self,
        title: &'a str,
        content: Element<'a, Message>,
        close: Message,
        max_width: u32,
    ) -> Element<'a, Message> {
        let heading = row![
            text(title).size(23).width(Length::Fill),
            button(self.t("关闭", "Close"))
                .on_press(close)
                .style(action_style),
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);
        let mut dialog = column![heading].spacing(14);
        if let Some(banner) = self.notice_banner() {
            dialog = dialog.push(banner);
        }
        let body = scrollable(content).spacing(8).smooth_scroll(true);
        container(card(dialog.push(body.boxed())).width(Length::Fill.max(max_width)))
            .padding(24)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(scrim)
            .boxed()
    }

    fn notice_banner(&self) -> Option<Element<'_, Message>> {
        let (is_error, message) = self.notice.as_ref()?;
        let banner = row![
            text(message).width(Length::Fill),
            button(self.t("关闭", "Dismiss"))
                .on_press(Message::DismissNotice)
                .style(button::text)
        ]
        .spacing(12);
        Some(
            container(banner)
                .padding(12)
                .width(Length::Fill)
                .style(if *is_error {
                    container::danger
                } else {
                    container::success
                })
                .boxed(),
        )
    }

    fn main_shell(&self) -> Element<'_, Message> {
        let mut navigation = column![text("BitNet").size(32), Space::new().height(12),].spacing(7);
        for page in Page::ALL {
            let selected = page == self.page;
            navigation = navigation.push(
                button(text(page.label(self.english())).size(15))
                    .width(Length::Fill)
                    .padding([11, 13])
                    .on_press(Message::Navigate(page))
                    .style(move |theme, status| {
                        if selected {
                            button::primary(theme, status)
                        } else {
                            button::text(theme, status)
                        }
                    })
                    .boxed(),
            );
        }
        let scope = if self.connecting {
            self.t("正在连接后台", "Connecting to engine")
        } else {
            match self
                .snapshot
                .as_ref()
                .filter(|_| self.snapshot_current)
                .map(|s| s.scope)
            {
                Some(Scope::System) => self.t("已连接系统后台", "System service connected"),
                Some(Scope::User) => self.t("已连接个人引擎", "User engine connected"),
                None => self.t("未连接后台", "Engine disconnected"),
            }
        };
        navigation = navigation
            .push(Space::new().height(Length::Fill).boxed())
            .push(text(scope).size(13).boxed())
            .push(text(format!("v{APP_VERSION}")).size(11).boxed())
            .push(
                button(self.t("隐藏界面", "Hide window"))
                    .on_press_maybe(
                        self.tray
                            .as_ref()
                            .filter(|tray| tray.is_available())
                            .and(self.window)
                            .map(Message::WindowCloseRequested),
                    )
                    .padding([4, 0])
                    .style(button::text)
                    .boxed(),
            );

        let header = row![
            text(self.page.label(self.english())).size(27),
            Space::new().width(Length::Fill),
            pick_list(
                Some(self.mode),
                [Mode::User, Mode::System],
                ToString::to_string,
            )
            .on_select(Message::ModeChanged)
            .width(225),
            button(self.t("刷新", "Refresh"))
                .on_press_maybe(
                    (!self.busy && !self.connecting && !self.polling).then_some(Message::Refresh)
                )
                .style(action_style),
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);
        let mut content = column![header].spacing(14);
        if let Some(error) = &self.connection_error {
            content = content.push(
                container(
                    row![
                        text(&error.message).width(Length::Fill),
                        button(self.t("后台服务", "Background service"))
                            .on_press(Message::Navigate(Page::Service))
                            .style(button::text),
                    ]
                    .spacing(10)
                    .align_y(alignment::Vertical::Center),
                )
                .padding(12)
                .width(Length::Fill)
                .style(container::danger)
                .boxed(),
            );
        }
        if self.modal.is_none() && self.health_editor.is_none() && self.quick_login.is_none() {
            if let Some(banner) = self.notice_banner() {
                content = content.push(banner);
            }
        }
        if self.snapshot.is_some() && !self.snapshot_current && !self.connecting {
            content = content.push(
                text(self.t("显示断开前的数据", "Showing data from before disconnect"))
                    .size(13)
                    .boxed(),
            );
        }
        if self.busy || self.connecting {
            content = content.push(
                text(if self.connecting {
                    self.t("正在连接本机引擎…", "Connecting to the local engine…")
                } else {
                    self.t("正在处理，请稍候…", "Working, please wait…")
                })
                .size(13)
                .boxed(),
            );
        }
        content = content.push(match self.page {
            Page::Status => self.status_view(),
            Page::Accounts => self.accounts_view(),
            Page::Networks => self.networks_view(),
            Page::Settings => self.settings_view(),
            Page::Service => self.service_view(),
            Page::History => self.history_view(),
            Page::Statistics => self.statistics_view(),
            Page::Diagnostics => self.diagnostics_view(),
        });
        row![
            container(navigation)
                .width(180)
                .height(Length::Fill)
                .padding(16)
                .style(container::rounded_box),
            scrollable(container(content).padding(20).width(Length::Fill))
                .smooth_scroll(true)
                .height(Length::Fill)
                .width(Length::Fill)
        ]
        .height(Length::Fill)
        .boxed()
    }

    fn status_view(&self) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return card(
                column![
                    text(self.t("请先连接后台", "Connect to the engine first")).size(22),
                    row![
                        button(self.t("连接 / 重试", "Connect / retry"))
                            .on_press_maybe((!self.connecting).then_some(Message::Refresh)),
                        button(self.t("后台服务", "Background service"))
                            .on_press(Message::Navigate(Page::Service))
                            .style(action_style),
                    ]
                    .spacing(10)
                    .wrap(),
                ]
                .spacing(12),
            )
            .boxed();
        };
        let connected = self.snapshot_current && !self.busy;
        let system_auth = bitnet_runtime::engine::uses_system_authentication(snapshot);
        let portal_controls = connected && !system_auth;
        let auth = match snapshot.session.auth {
            AuthState::Authenticated => self.t("已认证", "Authenticated"),
            AuthState::Unauthenticated => self.t("尚未认证", "Not authenticated"),
            AuthState::Unknown => self.t("状态待确认", "Authentication unknown"),
        };
        let offline = snapshot.session.auth != AuthState::Authenticated;
        let state = card(
            column![
                text(self.state_label(&snapshot.state)).size(34),
                text(format!(
                            "{} · {}",
                            auth,
                            self.environment_label(&snapshot.environment)
                ))
                .size(16),
                rule::horizontal(1),
                pair(
                    self.t("当前校园账号", "Campus account"),
                    snapshot.session.username.as_deref().unwrap_or("—")
                ),
                pair(
                    self.t("认证 IP", "Session IP"),
                    snapshot
                        .session
                        .ip
                        .map(|ip| ip.to_string())
                        .unwrap_or_else(|| "—".into())
                ),
                pair(
                    self.t("网关统计流量", "Gateway data usage"),
                    snapshot
                        .session
                        .bytes_used
                        .map(format_bytes)
                        .unwrap_or_else(|| "—".into())
                ),
                pair(
                    self.t("网关累计在线时长", "Gateway accumulated online time"),
                    snapshot
                        .session
                        .online_seconds
                        .map(format_duration)
                        .unwrap_or_else(|| "—".into())
                ),
                pair(
                    self.t("账户余额", "Balance"),
                    snapshot
                        .session
                        .balance
                        .map(|value| format!("¥ {value:.2}"))
                        .unwrap_or_else(|| "—".into())
                ),
                text(self.t(
                    "流量和时长的统计周期由校方网关定义，不代表本次连续会话。",
                    "The campus gateway defines the accounting period; these totals are not the duration or usage of this connection.",
                )).size(12),
                row![
                    button(self.t("登录…", "Sign in…")).on_press_maybe(
                        (portal_controls && offline).then_some(Message::OpenQuickLogin)
                    ),
                    button(self.t("使用已保存账户", "Use saved account"))
                        .on_press_maybe(
                            (portal_controls && offline && !snapshot.accounts.is_empty())
                                .then_some(Self::login_message(snapshot))
                        )
                        .style(action_style),
                    button(self.t("注销…", "Disconnect…"))
                        .on_press_maybe(portal_controls.then_some(Message::Ask(Confirmation::Logout)))
                        .style(action_style),
                    button(self.t("立即检测", "Check now"))
                        .on_press_maybe(portal_controls.then_some(Message::Run(Command::Check)))
                        .style(action_style),
                    button(if snapshot.settings.paused || snapshot.verification_hold {
                        self.t("恢复自动认证", "Resume")
                    } else {
                        self.t("暂停自动认证", "Pause")
                    })
                    .on_press_maybe(connected.then_some(Message::Run(
                        if snapshot.settings.paused || snapshot.verification_hold {
                            Command::Resume
                        } else {
                            Command::Pause
                        }
                    )))
                    .style(action_style),
                    button(self.t("重新认证…", "Reauthenticate…"))
                        .on_press_maybe(
                            (portal_controls && !snapshot.accounts.is_empty())
                                .then_some(Message::Ask(Confirmation::ForceLogin))
                        )
                        .style(button::text),
                    button(Page::Networks.label(self.english()))
                        .on_press(Message::Navigate(Page::Networks))
                        .style(action_style),
                ]
                .spacing(10)
                .wrap(),
            ]
            .spacing(12),
        );
        let mut content = column![state].spacing(14);
        if system_auth {
            content = content.push(
                text(self.t(
                    "BIT-mobile 由系统完成认证",
                    "Your system handles BIT-mobile sign-in",
                ))
                .boxed(),
            );
        } else if snapshot.settings.automatic
            && !snapshot.settings.paused
            && !snapshot.verification_hold
            && !snapshot.network_trusted
            && snapshot.active_binding.is_some()
            && snapshot
                .capabilities
                .iter()
                .any(|capability| capability == "network_trust")
        {
            content = content.push(
                card(
                    row![
                        text(self.t(
                            "自动认证需要信任当前网络",
                            "Automatic authentication requires a trusted network"
                        ))
                        .width(Length::Fill),
                        button(self.t("查看网络", "View network"))
                            .on_press(Message::Navigate(Page::Networks))
                            .style(action_style),
                    ]
                    .spacing(10)
                    .align_y(alignment::Vertical::Center),
                )
                .boxed(),
            );
        }
        if snapshot.accounts.is_empty() {
            content = content.push(
                card(
                    column![
                        text(self.t("欢迎使用 BitNet", "Welcome to BitNet")).size(22),
                        button(self.t("添加第一个账户", "Add your first account"))
                            .on_press(Message::Navigate(Page::Accounts)),
                    ]
                    .spacing(12),
                )
                .boxed(),
            );
        }
        if let Some(error) = &snapshot.last_error {
            content = content.push(
                card(
                    column![
                        text(self.t("需要关注", "Needs attention")).size(20),
                        text(&error.message),
                        button(self.t("打开诊断", "Open diagnostics"))
                            .on_press(Message::Navigate(Page::Diagnostics))
                    ]
                    .spacing(12),
                )
                .boxed(),
            );
        }
        content = content.push(
            card(
                column![
                    text(self.t("连接健康", "Connection health")).size(20),
                    pair(
                        self.t("互联网", "Internet"),
                        self.health_label(&snapshot.health.internet)
                    ),
                    pair("DNS", self.health_label(&snapshot.health.dns)),
                    pair("IPv6", self.health_label(&snapshot.health.ipv6)),
                    pair(
                        self.t("最近成功认证", "Last successful login"),
                        self.relative_time(snapshot.last_success_at)
                    ),
                    pair(self.t("重试次数", "Retry attempts"), snapshot.retry_count),
                ]
                .spacing(10),
            )
            .boxed(),
        );
        content.push(self.statistics_view()).boxed()
    }

    fn network_trust_view(&self) -> Element<'_, Message> {
        let Some(snapshot) = self.snapshot.as_ref().filter(|s| {
            s.capabilities
                .iter()
                .any(|capability| capability == "network_trust")
        }) else {
            return Space::new().boxed();
        };
        if bitnet_runtime::engine::uses_system_authentication(snapshot) {
            return text(self.t(
                "BIT-mobile 由系统完成认证",
                "Your system handles BIT-mobile sign-in",
            ))
            .boxed();
        }
        let enabled = self.snapshot_current && !self.busy && snapshot.active_binding.is_some();
        let (label, action, button_label) = if snapshot.network_trusted {
            (
                self.t("当前网络已信任", "Current network is trusted"),
                Message::Run(Command::TrustNetwork { trusted: false }),
                self.t("取消网络信任", "Remove trust"),
            )
        } else {
            (
                self.t("当前网络尚未信任", "Current network is not trusted"),
                if snapshot.settings.automatic
                    && snapshot
                        .capabilities
                        .iter()
                        .any(|item| item == "enable_automatic")
                {
                    Message::BeginAutomatic
                } else {
                    Message::Ask(Confirmation::TrustNetwork)
                },
                self.t("信任当前网络…", "Trust this network…"),
            )
        };
        card(
            row![
                text(label),
                button(button_label)
                    .on_press_maybe(enabled.then_some(action))
                    .style(action_style),
            ]
            .spacing(10)
            .align_y(alignment::Vertical::Center),
        )
        .boxed()
    }

    fn quick_login_view<'a>(&'a self, draft: &'a AccountDraft) -> Element<'a, Message> {
        let editable = !self.busy;
        let can_submit = editable
            && self.snapshot_current
            && !draft.username.trim().is_empty()
            && !draft.password.is_empty();
        column![
            text_input(self.t("校园账号", "Campus username"), &draft.username)
                .id("quick-login-username")
                .on_input_maybe(editable.then_some(Message::UsernameChanged))
                .on_submit_maybe(editable.then_some(Message::MoveFocus(false)))
                .padding(10),
            text_input(
                self.t("校园网密码", "Campus network password"),
                &draft.password
            )
            .secure(true)
            .on_input_maybe(editable.then_some(Message::PasswordChanged))
            .on_submit_maybe(can_submit.then_some(Message::TemporaryLogin))
            .padding(10),
            row![
                button(self.t("仅登录本次…", "Sign in once…"))
                    .on_press_maybe(can_submit.then_some(Message::TemporaryLogin)),
                button(self.t("验证并保存…", "Verify and save…"))
                    .on_press_maybe(can_submit.then_some(Message::AccountSave))
                    .style(action_style),
            ]
            .spacing(10)
            .wrap(),
        ]
        .spacing(13)
        .boxed()
    }

    fn accounts_view(&self) -> Element<'_, Message> {
        let verifying = self.credential_request == Some(CredentialRequest::VerifyAccount);
        let mut content = column![text(self.t(
            "保存的密码只允许覆盖，不会显示或导出。",
            "Saved passwords can be replaced, never displayed or exported."
        ))
        .size(14)]
        .spacing(16);
        if let Some(snapshot) = &self.snapshot {
            for account in &snapshot.accounts {
                let selected = snapshot.settings.selected_account.as_deref() == Some(&account.id);
                content = content.push(
                    card(
                        column![
                            row![
                                text(&account.username).size(21),
                                Space::new().width(Length::Fill),
                                text(if selected {
                                    self.t("当前账户", "Selected")
                                } else {
                                    ""
                                })
                            ]
                            .spacing(12),
                            text(format!(
                                "{} · {}",
                                account.id,
                                if account.has_password {
                                    self.t("密码已安全保存", "Password saved")
                                } else {
                                    self.t("尚未保存密码", "No saved password")
                                }
                            ))
                            .size(12),
                            row![
                                button(self.t("设为当前", "Select")).on_press_maybe(
                                    (!self.busy && !selected).then_some(Message::Run(
                                        Command::AccountSelect {
                                            id: account.id.clone()
                                        }
                                    ))
                                ),
                                button(self.t("修改账号 / 密码", "Edit / replace password"))
                                    .on_press_maybe(
                                        (!self.busy)
                                            .then_some(Message::AccountEdit(account.id.clone()))
                                    )
                                    .style(action_style),
                                button(self.t("删除…", "Delete…"))
                                    .on_press_maybe((!self.busy).then_some(Message::Ask(
                                        Confirmation::DeleteAccount(account.id.clone())
                                    )))
                                    .style(button::danger)
                            ]
                            .spacing(10)
                            .wrap(),
                        ]
                        .spacing(10),
                    )
                    .boxed(),
                );
            }
        }
        content = content.push(
            card(
                column![
                    text(if self.account.id.is_some() {
                        self.t("覆盖修改账户", "Replace account details")
                    } else {
                        self.t("添加账户", "Add account")
                    })
                    .size(21),
                    text_input(
                        self.t("校园账号", "Campus username"),
                        &self.account.username
                    )
                    .on_input_maybe((!verifying).then_some(Message::UsernameChanged))
                    .on_submit_maybe((!verifying).then_some(Message::MoveFocus(false)))
                    .padding(12),
                    text_input(
                        self.t("输入新的密码", "Enter a new password"),
                        &self.account.password
                    )
                    .secure(true)
                    .on_input_maybe((!verifying).then_some(Message::PasswordChanged))
                    .on_submit_maybe(
                        (!self.busy && self.snapshot_current).then_some(Message::AccountSave)
                    )
                    .padding(12),
                    row![
                        button(if verifying {
                            self.t("正在验证登录…", "Verifying sign-in…")
                        } else {
                            self.t("验证并保存…", "Verify and save…")
                        })
                        .on_press_maybe(
                            (!self.busy && self.snapshot_current).then_some(Message::AccountSave)
                        ),
                        button(self.t("清空 / 取消", "Clear / cancel"))
                            .on_press_maybe((!verifying).then_some(Message::AccountCancel))
                            .style(action_style)
                    ]
                    .spacing(10),
                    text(if self.snapshot_current {
                        ""
                    } else {
                        self.t(
                            "连接后台后即可保存账户",
                            "Connect to the engine to save an account",
                        )
                    })
                    .size(13),
                    checkbox(self.draft.settings.automatic)
                        .label(self.t(
                            "启用校园网络自动认证",
                            "Authenticate automatically on campus networks"
                        ))
                        .on_toggle_maybe(
                            (!self.busy && self.snapshot_current)
                                .then_some(Message::AutomaticChanged)
                        ),
                    row![
                        button(self.t("保存设置", "Save settings"))
                            .on_press_maybe(
                                (!self.busy && self.snapshot_current && self.draft.dirty)
                                    .then_some(Message::SaveSettings)
                            )
                            .style(action_style),
                        button(Page::Networks.label(self.english()))
                            .on_press(Message::Navigate(Page::Networks))
                            .style(action_style),
                    ]
                    .spacing(10)
                    .wrap(),
                ]
                .spacing(13),
            )
            .boxed(),
        );
        content.boxed()
    }

    fn networks_view(&self) -> Element<'_, Message> {
        let mut content =
            column![
                button(self.t("恢复自动选择网卡", "Select an interface automatically"))
                    .on_press_maybe(
                        (!self.busy && self.snapshot_current && !self.draft.dirty)
                            .then_some(Message::InterfaceSelected(String::new()))
                    ),
            ]
            .spacing(10);
        if self.enabling_automatic {
            content = content.push(
                button(self.t("继续设置信任", "Continue to network trust"))
                    .on_press_maybe(
                        (!self.busy
                            && self.snapshot_current
                            && self.snapshot.as_ref().and_then(selected_binding).is_some())
                        .then_some(Message::BeginAutomatic),
                    )
                    .boxed(),
            );
        }
        let mut profiles = column![
            text(self.t("认证环境配置", "Authentication profiles")).size(22),
            text(self.t("保存当前已应用的网关、接口、账户和规则；只有明确选择时才切换。系统后台的认证目标变更仍须管理员授权。", "Capture the currently applied gateway, interface, account and rules. Profiles switch only when selected. Changing a system-service authentication endpoint still requires administrator authorization.")).size(13),
        ].spacing(12);
        if let Some(snapshot) = &self.snapshot {
            for profile in &snapshot.settings.profiles {
                let selected = snapshot.settings.active_profile.as_deref() == Some(&profile.id);
                profiles = profiles.push(
                    column![
                        row![
                            text(&profile.name).size(18),
                            text(if selected {
                                self.t("当前环境", "Active")
                            } else {
                                ""
                            }),
                            Space::new().width(Length::Fill),
                            button(self.t("选择", "Select"))
                                .on_press_maybe(
                                    (!self.busy && !self.draft.dirty && !selected)
                                        .then_some(Message::ProfileSelect(profile.id.clone()))
                                )
                                .style(button::secondary),
                            button(self.t("删除…", "Delete…"))
                                .on_press_maybe((!self.busy && !self.draft.dirty).then_some(
                                    Message::Ask(Confirmation::DeleteProfile(profile.id.clone()))
                                ))
                                .style(button::danger)
                        ]
                        .spacing(10)
                        .wrap(),
                        text(format!(
                            "{} · {} · {}",
                            profile.id,
                            profile.gateway.base_url,
                            profile.interface_id.as_deref().unwrap_or("automatic")
                        ))
                        .size(12),
                    ]
                    .spacing(6)
                    .boxed(),
                );
            }
        }
        profiles = profiles
            .push(
                row![
                    text_input(
                        self.t("环境 ID，例如 campus", "Profile ID, e.g. campus"),
                        &self.profile_id
                    )
                    .on_input(Message::ProfileIdChanged)
                    .padding(10),
                    text_input(self.t("环境名称", "Profile name"), &self.profile_name)
                        .on_input(Message::ProfileNameChanged)
                        .padding(10),
                ]
                .spacing(10)
                .boxed(),
            )
            .push(
                button(self.t(
                    "保存已应用配置为环境",
                    "Capture applied settings as a profile",
                ))
                .on_press_maybe(
                    (!self.busy && !self.draft.dirty && self.snapshot.is_some())
                        .then_some(Message::ProfileSave),
                )
                .style(button::secondary)
                .boxed(),
            );
        if self.draft.dirty {
            profiles = profiles.push(text(self.t("有未保存设置：请先到偏好页保存或放弃，再切换或保存环境。", "Pending edits: save or discard preferences before capturing or switching profiles.")).size(12).boxed());
        }
        if let Some(snapshot) = &self.snapshot {
            let mut interfaces: Vec<_> = snapshot.interfaces.iter().collect();
            interfaces.sort_by_key(|interface| {
                let saved = snapshot.settings.interface_id.as_ref() == Some(&interface.id);
                let active = snapshot
                    .active_binding
                    .as_ref()
                    .is_some_and(|binding| binding.interface_id == interface.id);
                let mut addresses = interface.ipv4.iter().copied().chain(interface.gateway);
                let campus_range = addresses.clone().any(|ip| ip.octets()[0] == 10);
                let home_range = addresses.any(|ip| ip.octets()[..2] == [192, 168]);
                (
                    !saved,
                    !active,
                    is_alternate_adapter(interface),
                    !interface.up,
                    !campus_range,
                    !home_range,
                    interface.index,
                )
            });
            let mut alternate_count = 0;
            let mut alternatives = column![].spacing(7);
            for interface in interfaces {
                let active = snapshot
                    .active_binding
                    .as_ref()
                    .is_some_and(|binding| binding.interface_id == interface.id);
                let saved = snapshot.settings.interface_id.as_ref() == Some(&interface.id);
                let alternate = is_alternate_adapter(interface);
                let source = interface
                    .ipv4
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                let badge = |label: &'static str| {
                    container(text(label).size(12))
                        .padding([3, 7])
                        .style(container::rounded_box)
                };
                let mut heading = row![
                    badge(if interface.virtual_adapter {
                        self.t("虚拟网卡", "Virtual")
                    } else if alternate {
                        self.t("备选网卡", "Alternative")
                    } else {
                        self.t("物理网卡", "Physical")
                    }),
                    badge(if interface.up {
                        self.t("● 已连接", "● Connected")
                    } else {
                        self.t("○ 未连接", "○ Disconnected")
                    }),
                    container(
                        text(&interface.name)
                            .size(17)
                            .wrapping(text::Wrapping::WordOrGlyph)
                    )
                    .width(Length::Fit.max(320)),
                    button(self.t("使用此网卡", "Use this adapter"))
                        .on_press_maybe(
                            (!self.busy
                                && self.snapshot_current
                                && !self.draft.dirty
                                && interface.up
                                && !saved)
                                .then_some(Message::InterfaceSelected(interface.id.clone()))
                        )
                        .padding([4, 8])
                        .style(action_style),
                ]
                .spacing(8)
                .align_y(iced::Alignment::Center);
                if saved {
                    heading = heading.push(badge(self.t("已记录", "Saved")).boxed());
                }
                if active {
                    heading = heading.push(badge(self.t("当前活跃", "Active")).boxed());
                }
                let mut addresses = vec![format!(
                    "IPv4 {}",
                    if source.is_empty() { "—" } else { &source }
                )];
                if let Some(gateway) = interface.gateway {
                    addresses.push(format!("{} {gateway}", self.t("网关", "Gateway")));
                }
                if let Some(ssid) = &interface.ssid {
                    addresses.push(format!("SSID {ssid}"));
                }
                let entry = container(
                    column![heading.wrap(), text(addresses.join("  ·  ")).size(13),].spacing(5),
                )
                .width(Length::Fill)
                .padding([8, 12])
                .style(container::rounded_box);
                if alternate && !saved && !active {
                    alternate_count += 1;
                    if self.alternate_adapters {
                        alternatives = alternatives.push(entry);
                    }
                } else {
                    content = content.push(entry.boxed());
                }
            }
            if alternate_count > 0 {
                content = content
                    .push(
                        button(text(format!(
                            "{} ({alternate_count})",
                            if self.alternate_adapters {
                                self.t("收起备选网卡", "Hide alternative adapters")
                            } else {
                                self.t("展开备选网卡", "Show alternative adapters")
                            }
                        )))
                        .on_press(Message::ToggleAlternateAdapters)
                        .boxed(),
                    )
                    .push(alternatives.boxed());
            }
        }
        content = content.push(self.network_trust_view()).push(
            button(self.t("高级", "Advanced"))
                .on_press(Message::ToggleAdvancedNetworks)
                .boxed(),
        );
        if !self.advanced_networks {
            return content.boxed();
        }
        content = content.push(card(profiles).boxed());
        let mut rules = column![text(self.t("网络规则", "Network rules")).size(22),
            text(self.t("高优先级规则优先。忽略规则用于禁止在指定网络自动认证。", "Higher-priority rules take precedence. Ignore rules prevent automatic authentication on matching networks.")).size(12)].spacing(12);
        for (index, rule) in self.draft.settings.rules.iter().enumerate() {
            rules = rules.push(
                row![
                    column![
                        text(format!("{} · {}", rule.name, rule.priority)).size(17),
                        text(format!(
                            "SSID={}  Gateway={}  Account={}  {}",
                            rule.ssid.as_deref().unwrap_or("*"),
                            rule.gateway
                                .map(|ip| ip.to_string())
                                .unwrap_or_else(|| "*".into()),
                            rule.account_id.as_deref().unwrap_or("default"),
                            if rule.ignore { "IGNORE" } else { "ALLOW" }
                        ))
                        .size(12)
                    ]
                    .width(Length::Fill),
                    button(self.t("编辑", "Edit"))
                        .on_press(Message::RuleEdit(index))
                        .style(button::secondary),
                    button(self.t("删除…", "Delete…"))
                        .on_press(Message::Ask(Confirmation::DeleteRule(rule.id.clone())))
                        .style(button::danger)
                ]
                .spacing(8)
                .wrap()
                .boxed(),
            );
        }
        rules = rules
            .push(rule::horizontal(1).boxed())
            .push(
                text(if self.rule.index.is_some() {
                    self.t("编辑规则", "Edit rule")
                } else {
                    self.t("添加规则", "Add rule")
                })
                .size(18)
                .boxed(),
            )
            .push(
                text_input(self.t("规则名称", "Rule name"), &self.rule.name)
                    .on_input(Message::RuleNameChanged)
                    .padding(10)
                    .boxed(),
            )
            .push(
                text_input(
                    self.t("网卡 ID（留空匹配任何网卡）", "Interface ID (empty = any)"),
                    &self.rule.interface_id,
                )
                .on_input(Message::RuleInterfaceChanged)
                .padding(10)
                .boxed(),
            )
            .push(
                row![
                    text_input("SSID", &self.rule.ssid)
                        .on_input(Message::RuleSsidChanged)
                        .padding(10),
                    text_input(
                        self.t("网关 IPv4（可选）", "Gateway IPv4 (optional)"),
                        &self.rule.gateway
                    )
                    .on_input(Message::RuleGatewayChanged)
                    .padding(10)
                ]
                .spacing(10)
                .boxed(),
            )
            .push(
                row![
                    text_input(
                        self.t(
                            "账户 ID（留空使用当前账户）",
                            "Account ID (empty = selected)"
                        ),
                        &self.rule.account_id
                    )
                    .on_input(Message::RuleAccountChanged)
                    .padding(10),
                    text_input(
                        self.t("优先级，如 100", "Priority, e.g. 100"),
                        &self.rule.priority
                    )
                    .on_input(Message::RulePriorityChanged)
                    .padding(10)
                    .width(160)
                ]
                .spacing(10)
                .boxed(),
            )
            .push(
                checkbox(self.rule.ignore)
                    .label(self.t(
                        "忽略匹配网络，不自动认证",
                        "Ignore matching networks; do not authenticate automatically",
                    ))
                    .on_toggle(Message::RuleIgnoreChanged)
                    .boxed(),
            )
            .push(
                row![
                    button(self.t("保存规则", "Save rule")).on_press_maybe(
                        (!self.busy && self.snapshot.is_some()).then_some(Message::RuleSave)
                    ),
                    button(self.t("清空 / 取消", "Clear / cancel"))
                        .on_press(Message::RuleCancel)
                        .style(button::secondary)
                ]
                .spacing(10)
                .boxed(),
            );
        if let Some(snapshot) = &self.snapshot {
            rules = rules.push(
                text(format!(
                    "{}: {}",
                    self.t("可用账户 ID", "Available account IDs"),
                    snapshot
                        .accounts
                        .iter()
                        .map(account_label)
                        .collect::<Vec<_>>()
                        .join("; ")
                ))
                .size(12)
                .boxed(),
            );
        }
        content.push(card(rules).boxed()).boxed()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let mut preferences = column![
            text(self.t("基本", "General")).size(21),
            row![
                text(self.t("日志级别", "Log level")).width(180),
                pick_list(
                    Some(self.draft.settings.log_level.as_str()),
                    ["error", "warning", "info", "debug"],
                    ToString::to_string,
                )
                .on_select(|level| Message::LogLevelChanged(level.to_string())),
            ]
            .spacing(12),
            row![
                text(self.t("界面语言", "Language")).width(180),
                pick_list(
                    Some(self.language),
                    [Language::Chinese, Language::English],
                    ToString::to_string,
                )
                .on_select(Message::LanguageChanged),
            ]
            .spacing(12),
            row![
                text(self.t("主题", "Theme")).width(180),
                pick_list(
                    Some(self.appearance),
                    [Appearance::System, Appearance::Light, Appearance::Dark],
                    ToString::to_string,
                )
                .on_select(Message::AppearanceChanged),
            ]
            .spacing(12),
        ]
        .spacing(14);
        preferences = preferences.push(
            checkbox(self.draft.settings.automatic)
                .label(self.t("自动认证", "Automatic authentication"))
                .on_toggle_maybe(
                    (!self.busy && self.snapshot_current).then_some(Message::AutomaticChanged),
                )
                .boxed(),
        );
        for (flag, value, label) in [
            (
                SettingFlag::UserAutostart,
                self.draft.settings.user_autostart,
                self.t(
                    "登录系统后保持后台运行",
                    "Start the engine after signing in to your computer",
                ),
            ),
            (
                SettingFlag::GuiAutostart,
                self.draft.settings.gui_autostart,
                self.t(
                    "登录系统后打开界面",
                    "Open the window after signing in to your computer",
                ),
            ),
            (
                SettingFlag::StartHidden,
                self.draft.settings.start_hidden,
                self.t("启动软件时隐藏主窗口", "Start with the main window hidden"),
            ),
            (
                SettingFlag::NotifyErrors,
                self.draft.settings.notify_errors,
                self.t(
                    "需要处理的异常才通知",
                    "Notify only when an error needs attention",
                ),
            ),
        ] {
            let enabled = !matches!(flag, SettingFlag::UserAutostart | SettingFlag::GuiAutostart)
                || self.mode == Mode::User;
            preferences = preferences.push(
                checkbox(value)
                    .label(label)
                    .on_toggle_maybe(
                        enabled.then_some(move |value| Message::FlagChanged(flag, value)),
                    )
                    .boxed(),
            );
        }
        if self.mode == Mode::System {
            preferences = preferences.push(text(self.t("个人登录启动项请切换到“自动选择”模式修改；该模式仍优先连接已安装的系统服务。", "Switch to Automatic mode to edit your personal login items. Automatic mode still prefers the installed system service.")).size(12).boxed());
        }
        if !self.tray.as_ref().is_some_and(tray::Tray::is_available) {
            preferences = preferences.push(text(self.t("当前桌面未提供系统托盘；隐藏启动时将保留可访问窗口。", "No system tray is available on this desktop; hidden startup keeps an accessible window.")).size(12).boxed());
        }
        let mut advanced =
            column![text(self.t("检测与保留期限", "Checks and retention")).size(21)].spacing(12);
        for (field, value, label) in [
            (
                NumericSetting::StatusInterval,
                &self.draft.status_interval,
                self.t(
                    "认证状态间隔（5–3600 秒）",
                    "Authentication interval (5–3600 s)",
                ),
            ),
            (
                NumericSetting::HealthInterval,
                &self.draft.health_interval,
                self.t(
                    "健康探测间隔（30–86400 秒）",
                    "Health interval (30–86400 s)",
                ),
            ),
            (
                NumericSetting::RetryMaximum,
                &self.draft.retry_maximum,
                self.t(
                    "最长重试等待（5–3600 秒）",
                    "Maximum retry delay (5–3600 s)",
                ),
            ),
            (
                NumericSetting::RequestTimeout,
                &self.draft.request_timeout,
                self.t("单请求超时（1–60 秒）", "Request timeout (1–60 s)"),
            ),
            (
                NumericSetting::LogRetention,
                &self.draft.log_retention,
                self.t("日志保留（1–30 天）", "Log retention (1–30 days)"),
            ),
            (
                NumericSetting::HistoryRetention,
                &self.draft.history_retention,
                self.t("历史保留（1–365 天）", "History retention (1–365 days)"),
            ),
        ] {
            advanced = advanced.push(
                row![
                    text(label).width(Length::Fill),
                    text_input("", value)
                        .on_input(move |value| Message::NumberChanged(field, value))
                        .width(130)
                        .padding(8)
                ]
                .spacing(14)
                .align_y(alignment::Vertical::Center)
                .boxed(),
            );
        }
        let mut targets = column![row![
            text(self.t("健康探测地址", "Health check URLs")).width(Length::Fill),
            button(self.t("修改…", "Edit…"))
                .on_press(Message::OpenHealthEditor)
                .style(action_style),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center)]
        .spacing(5);
        for url in self
            .draft
            .health_targets
            .split(';')
            .map(str::trim)
            .filter(|url| !url.is_empty())
        {
            targets = targets.push(text(url).size(13).style(text::secondary).boxed());
        }
        let effective_endpoint = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.settings.gateway.base_url.as_str())
            .unwrap_or("—");
        advanced = advanced.push(targets.boxed()).push(
            row![
                text(self.t("认证服务器", "Authentication endpoint")).width(Length::Fill),
                text_input("", effective_endpoint).width(230).padding(8),
            ]
            .spacing(14)
            .align_y(alignment::Vertical::Center)
            .boxed(),
        );
        let save = row![
            button(self.t("保存设置", "Save settings")).on_press_maybe(
                (self.draft.dirty && !self.busy && self.snapshot.is_some())
                    .then_some(Message::SaveSettings)
            ),
            button(self.t("放弃修改", "Discard changes"))
                .on_press_maybe(self.draft.dirty.then_some(Message::RevertSettings))
                .style(action_style),
            text(if self.draft.dirty {
                self.t("有未保存修改", "Unsaved changes")
            } else {
                self.t("设置已同步", "Settings synchronized")
            })
            .size(13)
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);
        let mut transfer = column![text(self.t("非敏感设置导入 / 导出", "Import / export non-secret settings")).size(21),
            text(self.t("导出不含密码。导入会覆盖设置，并由引擎检查权限及配置版本。", "Exports contain no passwords. Imports replace settings and are checked by the engine for permissions and compatibility.")).size(12),
            row![button(self.t("生成导出内容", "Prepare export")).on_press_maybe((!self.busy).then_some(Message::Run(Command::ExportSettings))).style(button::secondary),
                button(self.t("复制导出", "Copy export")).on_press_maybe((!self.exported.is_empty()).then_some(Message::CopyExport)).style(button::secondary)].spacing(10),
        ].spacing(12);
        if !self.exported.is_empty() {
            transfer = transfer.push(
                scrollable(text(&self.exported).size(12))
                    .smooth_scroll(true)
                    .height(150)
                    .boxed(),
            );
        }
        transfer = transfer
            .push(
                text_editor(&self.import_content)
                    .placeholder(self.t(
                        "在此粘贴 BitNet 设置 JSON",
                        "Paste BitNet settings JSON here",
                    ))
                    .height(160)
                    .on_action(Message::ImportAction)
                    .boxed(),
            )
            .push(
                button(self.t("导入并保存设置", "Import and save settings"))
                    .on_press_maybe(
                        (!self.busy && self.snapshot.is_some()).then_some(Message::ImportSettings),
                    )
                    .style(button::secondary)
                    .boxed(),
            );
        let tools = row![
            button(Page::Networks.label(self.english()))
                .on_press(Message::Navigate(Page::Networks))
                .style(action_style),
            button(Page::History.label(self.english()))
                .on_press(Message::Navigate(Page::History))
                .style(action_style),
        ]
        .spacing(10)
        .wrap();
        let mut content = column![
            self.diagnostics_view(),
            tools,
            card(preferences),
            self.updates_view(),
            button(if self.advanced_settings {
                self.t("收起高级设置", "Hide advanced settings")
            } else {
                self.t("展开高级设置", "Show advanced settings")
            })
            .on_press(Message::ToggleAdvancedSettings)
            .style(action_style),
        ]
        .spacing(14);
        if self.advanced_settings {
            content = content
                .push(card(advanced).boxed())
                .push(card(transfer).boxed());
        }
        content.push(save.boxed()).boxed()
    }

    fn health_editor_view<'a>(&'a self, entries: &'a [String]) -> Element<'a, Message> {
        let mut content = column![text(self.t(
            "每行一个地址。一次粘贴多个地址时，用分号分隔。",
            "Enter one URL per row. Separate URLs with semicolons when pasting several.",
        ))
        .size(13)]
        .spacing(9);
        for (index, entry) in entries.iter().enumerate() {
            content = content.push(
                row![
                    text_input("http://…", entry)
                        .on_input(move |value| Message::HealthEntryChanged(index, value))
                        .padding(8),
                    button(self.t("移除", "Remove"))
                        .on_press(Message::HealthEntryRemoved(index))
                        .style(action_style),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .boxed(),
            );
        }
        content = content.push(
            row![
                button(self.t("添加地址", "Add URL"))
                    .on_press_maybe((entries.len() < 8).then_some(Message::HealthEntryAdded))
                    .style(action_style),
                button(self.t("应用", "Apply")).on_press(Message::SaveHealthEditor),
            ]
            .spacing(10)
            .wrap()
            .boxed(),
        );
        self.dialog(
            self.t("编辑健康探测地址", "Edit health check URLs"),
            content.boxed(),
            Message::CancelHealthEditor,
            880,
        )
    }

    fn service_view(&self) -> Element<'_, Message> {
        let stop_hint = text(self.t("停止系统后台会持续关闭后台（Windows 同时关闭服务自动启动），直到你再次启动；仅想暂时停止认证，请使用“暂停自动认证”。", "Stopping the system service keeps it disabled until you start it again (including Windows service startup). To suspend authentication temporarily, use Pause instead.")).size(13);
        let mut content = column![card(column![text(self.t("开机无人值守联网", "Connect before signing in")).size(24),
            text(self.t("系统后台在没有用户登录时运行。关闭界面不会结束后台认证。", "The background service runs without an interactive login. Closing the window does not stop authentication.")),
        ].spacing(12))].spacing(18);
        content = content.push(stop_hint.boxed());
        if let Some(snapshot) = &self.snapshot {
            let service = &snapshot.service;
            content = content.push(
                card(
                    column![
                        pair(
                            self.t("服务安装", "Installed"),
                            self.service_value(service.installed)
                        ),
                        pair(
                            self.t("服务运行", "Running"),
                            self.service_value(service.running)
                        ),
                        pair(
                            self.t("开机启动", "Boot startup"),
                            self.service_value(service.enabled)
                        ),
                        pair(
                            self.t("当前连接范围", "Connected scope"),
                            if snapshot.scope == Scope::System {
                                self.t("共享系统服务", "Shared system service")
                            } else {
                                self.t("当前用户", "Current user")
                            }
                        ),
                        text(service.detail.as_deref().unwrap_or("")),
                        row![
                            button(self.t("重新连接后台", "Reconnect"))
                                .on_press_maybe(
                                    (!self.busy && !self.connecting).then_some(Message::Refresh)
                                )
                                .style(button::secondary),
                            button(self.t("立即检查连接", "Check connection"))
                                .on_press_maybe(
                                    (!self.busy).then_some(Message::Run(Command::Check))
                                )
                                .style(button::secondary)
                        ]
                        .spacing(10),
                    ]
                    .spacing(12),
                )
                .boxed(),
            );
            let mut actions = row![].spacing(10);
            if service.installed {
                actions = actions
                    .push(
                        button(self.t("启动后台…", "Start service…")).on_press_maybe(
                            self.can_start_service().then_some(Message::Ask(
                                Confirmation::Maintenance(MaintenanceAction::ServiceStart),
                            )),
                        ),
                    )
                    .push(
                        button(self.t("停止后台…", "Stop service…"))
                            .on_press_maybe((!self.busy && service.running).then_some(
                                Message::Ask(Confirmation::Maintenance(
                                    MaintenanceAction::ServiceStop,
                                )),
                            ))
                            .style(button::secondary),
                    )
                    .push(
                        button(self.t("卸载后台，保留数据…", "Remove service, keep data…"))
                            .on_press_maybe((!self.busy).then_some(Message::Ask(
                                Confirmation::Maintenance(MaintenanceAction::ServiceRemove {
                                    purge: false,
                                }),
                            )))
                            .style(button::danger),
                    );
            } else {
                actions = actions.push(
                    button(self.t("启用开机后台…", "Enable system service…")).on_press_maybe(
                        (!self.busy).then_some(Message::Ask(Confirmation::Maintenance(
                            MaintenanceAction::ServiceInstall,
                        ))),
                    ),
                );
            }
            actions = actions.push(
                button(self.t("修复安装…", "Repair installation…"))
                    .on_press_maybe((!self.busy).then_some(Message::Ask(
                        Confirmation::Maintenance(MaintenanceAction::Repair),
                    )))
                    .style(button::secondary),
            );
            content = content.push(actions.wrap().boxed());
            if snapshot.scope == Scope::User {
                content = content.push(
                    button(self.t("停止个人引擎…", "Stop user engine…"))
                        .on_press_maybe(
                            (!self.busy).then_some(Message::Ask(Confirmation::Shutdown)),
                        )
                        .style(button::text)
                        .boxed(),
                );
            }
        } else {
            content = content
                .push(
                    button(self.t("安装 / 修复系统后台…", "Install / repair system service…"))
                        .on_press_maybe((!self.busy && !self.connecting).then_some(Message::Ask(
                            Confirmation::Maintenance(MaintenanceAction::ServiceInstall),
                        )))
                        .boxed(),
                )
                .push(
                    button(self.t(
                        "启动已安装的系统后台…",
                        "Start an installed system service…",
                    ))
                    .on_press_maybe((!self.busy && !self.connecting).then_some(Message::Ask(
                        Confirmation::Maintenance(MaintenanceAction::ServiceStart),
                    )))
                    .style(button::secondary)
                    .boxed(),
                );
        }
        content.boxed()
    }

    pub(super) fn history_title(&self, kind: &str) -> &'static str {
        match kind {
            "reconnect" => self.t("校园网已恢复", "Campus connection restored"),
            "login_success" => self.t("校园网登录成功", "Campus sign-in succeeded"),
            "login_failure" => self.t("校园网登录失败", "Campus sign-in failed"),
            "account_verification_failed" => self.t("账户验证失败", "Account verification failed"),
            "account_saved" => self.t("账户已保存", "Account saved"),
            "environment_observed" => self.t("网络状态已更新", "Network status updated"),
            "online_segment" => self.t("在线时段", "Online period"),
            "session_lost" => self.t("校园网认证已断开", "Campus session ended"),
            "health_degraded" => self.t("互联网连接检查异常", "Internet connectivity issue"),
            _ => self.t("其他事件", "Other activity"),
        }
    }

    fn history_summary(&self, event: &HistoryEvent) -> String {
        let duration = event
            .duration_ms
            .map(|ms| format_history_duration(ms, self.english()));
        match event.kind.as_str() {
            "reconnect" => duration.map_or_else(
                || {
                    self.t(
                        "已确认恢复校园网连接。",
                        "Campus connectivity was confirmed.",
                    )
                    .into()
                },
                |duration| format!("{} {duration}", self.t("恢复用时", "Recovery took")),
            ),
            "login_success" => duration.map_or_else(
                || {
                    self.t("已向校园网确认登录。", "The gateway confirmed the sign-in.")
                        .into()
                },
                |duration| format!("{} {duration}", self.t("登录用时", "Sign-in took")),
            ),
            "environment_observed" => {
                let auth = event
                    .details
                    .as_ref()
                    .and_then(|details| details.get("auth_state"))
                    .and_then(serde_json::Value::as_str);
                match auth {
                    Some("authenticated") => {
                        self.t("已确认校园网在线。", "Campus sign-in was confirmed.")
                    }
                    Some("unauthenticated") => {
                        self.t("当前需要登录校园网。", "Campus sign-in is required.")
                    }
                    _ => self.t(
                        "已检查当前网络和认证状态。",
                        "The network and sign-in status were checked.",
                    ),
                }
                .into()
            }
            "online_segment" => duration.map_or_else(
                || {
                    self.t(
                        "记录了一段校园网在线时间。",
                        "A period of campus connectivity was recorded.",
                    )
                    .into()
                },
                |duration| {
                    format!(
                        "{} {duration}",
                        self.t("观测到连续在线", "Observed online for")
                    )
                },
            ),
            "session_lost" => self
                .t("校园网要求重新登录。", "Campus sign-in is required again.")
                .into(),
            "health_degraded" => self
                .t(
                    "校园网登录仍有效，访问互联网出现异常。",
                    "Campus sign-in is still active, but internet checks failed.",
                )
                .into(),
            _ => event.message.clone(),
        }
    }

    fn history_view(&self) -> Element<'_, Message> {
        let mut content = column![row![
            text_input(self.t("筛选记录", "Filter activity"), &self.history_filter)
                .on_input(Message::HistoryFilterChanged)
                .padding(8),
            button(self.t("刷新历史", "Refresh activity"))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current).then_some(Message::HistoryLoad(false))
                )
                .style(action_style),
            button(self.t("复制 JSON", "Copy JSON"))
                .on_press_maybe((!self.history.is_empty()).then_some(Message::CopyHistory))
                .style(action_style),
            button(self.t("保存到文件…", "Save to file…"))
                .on_press_maybe(
                    (!self.history.is_empty()
                        && !self.history_exporting
                        && self.history_export_path.is_none())
                    .then_some(Message::OpenHistoryExport)
                )
                .style(action_style),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center)
        .wrap()]
        .spacing(8);
        content = content.push(
            text(self.t(
                "按 ↑ / ↓ 选择记录，按回车展开或收起。",
                "Use ↑ / ↓ to select a record and Enter to expand or collapse it.",
            ))
            .size(12)
            .style(text::secondary)
            .boxed(),
        );
        if let Some(path) = self.history_export_path.as_deref() {
            let mut path_input = text_input(
                self.t("完整的 JSON 文件路径", "Full path to a JSON file"),
                path,
            )
            .padding(8);
            if !self.history_exporting {
                path_input = path_input
                    .on_input(Message::HistoryExportPathChanged)
                    .on_submit(Message::ExportHistory);
            }
            content = content.push(
                card(
                    column![
                        text(self.t("导出当前载入的记录", "Export loaded activity")).size(16),
                        path_input,
                        row![
                            button(self.t("保存 JSON", "Save JSON")).on_press_maybe(
                                (!self.history_exporting && !path.trim().is_empty())
                                    .then_some(Message::ExportHistory)
                            ),
                            button(self.t("取消", "Cancel"))
                                .on_press_maybe(
                                    (!self.history_exporting)
                                        .then_some(Message::CloseHistoryExport)
                                )
                                .style(action_style),
                        ]
                        .spacing(10)
                        .wrap(),
                    ]
                    .spacing(8),
                )
                .boxed(),
            );
        }
        let mut shown = 0;
        let now = bitnet_core::now_secs();
        for event in self.visible_history() {
            shown += 1;
            let expanded = self.expanded_history.contains(&event.id);
            let selected = self.history_selection == Some(event.id);
            let heading = row![
                text(if expanded { "▾" } else { "▸" }),
                text(self.history_title(&event.kind))
                    .size(16)
                    .width(Length::Fill),
                text(format_elapsed_minutes(
                    now.saturating_sub(event.at),
                    self.english()
                ))
                .size(12),
            ]
            .spacing(8)
            .align_y(alignment::Vertical::Center);
            let mut event_content = column![button(
                column![heading, text(self.history_summary(event)).size(14)]
                    .spacing(4)
                    .width(Length::Fill)
            )
            .width(Length::Fill)
            .on_press(Message::HistoryDetailToggled(event.id))
            .padding([10, 14])
            .style(move |theme, status| {
                if selected {
                    action_style(theme, status)
                } else {
                    button::background(theme, status)
                }
            }),]
            .spacing(4);
            if expanded {
                let mut details = column![text(format!(
                    "#{}  {}  {}",
                    event.id,
                    event.kind,
                    event.account.as_deref().unwrap_or("")
                ))
                .size(12)]
                .spacing(3);
                if let Some(payload) = &event.details {
                    details = details.push(
                        text(serde_json::to_string_pretty(payload).unwrap_or_default())
                            .size(12)
                            .wrapping(text::Wrapping::WordOrGlyph)
                            .boxed(),
                    );
                }
                event_content = event_content.push(
                    container(details)
                        .padding(10)
                        .width(Length::Fill)
                        .style(container::bordered_box)
                        .boxed(),
                );
            }
            content = content.push(event_content.boxed());
        }
        if shown == 0 {
            content =
                content.push(card(text(self.t("暂无匹配记录", "No matching activity"))).boxed());
        }
        if self.history.len() == self.history_limit as usize && self.history_limit < 200 {
            content = content.push(
                button(self.t("加载更早记录", "Load older activity"))
                    .on_press_maybe(
                        (!self.busy && self.snapshot_current).then_some(Message::HistoryLoad(true)),
                    )
                    .style(action_style)
                    .boxed(),
            );
        }
        content.boxed()
    }

    fn statistics_view(&self) -> Element<'_, Message> {
        let mut content = column![row![
            text(Page::Statistics.label(self.english()))
                .size(21)
                .width(Length::Fill),
            button(self.t("刷新统计", "Refresh statistics"))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current)
                        .then_some(Message::Run(Command::Statistics))
                )
                .style(action_style),
        ]
        .spacing(10)
        .align_y(alignment::Vertical::Center)]
        .spacing(9);
        if let Some(statistics) = &self.statistics {
            content = content.push(
                card(
                    column![
                        pair(
                            self.t("认证成功", "Successful logins"),
                            statistics.login_successes
                        ),
                        pair(
                            self.t("认证失败", "Failed logins"),
                            statistics.login_failures
                        ),
                        pair(self.t("恢复连接", "Reconnections"), statistics.reconnects),
                        pair(self.t("事件总数", "Activity events"), statistics.events),
                        pair(
                            self.t("观测在线总时长", "Observed online duration"),
                            format_duration(statistics.observed_online_seconds)
                        ),
                        pair(
                            self.t("最长观测在线段", "Longest observed online segment"),
                            format_duration(statistics.longest_observed_online_seconds)
                        ),
                        pair(
                            self.t("观测会话中断", "Observed session interruptions"),
                            statistics.session_interruptions
                        ),
                        pair(
                            self.t("健康异常次数", "Health anomalies"),
                            statistics.health_anomalies
                        ),
                        pair(
                            self.t("最近成功", "Last success"),
                            self.relative_time(statistics.last_success_at)
                        ),
                        pair(
                            self.t("平均恢复时间", "Average recovery time"),
                            statistics
                                .average_recovery_ms
                                .map(|ms| format!("{:.1} s", ms as f64 / 1000.0))
                                .unwrap_or_else(|| "—".into())
                        ),
                    ]
                    .spacing(7),
                )
                .boxed(),
            );
        } else {
            content = content.push(card(text(self.t("暂无统计数据", "No statistics yet"))).boxed());
        }
        content.boxed()
    }

    fn diagnostics_view(&self) -> Element<'_, Message> {
        let mut content = column![card(
            column![
                text(Page::Diagnostics.label(self.english())).size(21),
                text(self.t(
                    "检查当前网络并生成诊断报告。",
                    "Check your network and generate a diagnostic report."
                )),
                row![
                    button(self.t("开始诊断", "Run diagnostics")).on_press_maybe(
                        (!self.busy && self.snapshot_current)
                            .then_some(Message::Run(Command::Diagnose))
                    ),
                    button(self.t("复制诊断报告", "Copy report"))
                        .on_press_maybe(
                            (!self.diagnostics.is_empty()).then_some(Message::CopyDiagnostics)
                        )
                        .style(action_style)
                ]
                .spacing(10)
                .wrap(),
            ]
            .spacing(9)
        )]
        .spacing(8);
        if !self.diagnostics.is_empty() {
            content = content.push(card(text(&self.diagnostics).size(13)).boxed());
        }
        content.boxed()
    }

    fn updates_view(&self) -> Element<'_, Message> {
        let engine_version = self
            .snapshot
            .as_ref()
            .map(|s| s.app_version.as_str())
            .unwrap_or("—");
        column![card(column![text(self.t("版本与更新", "Version and updates")).size(24),
            pair(self.t("桌面客户端", "Desktop client"), APP_VERSION), pair(self.t("当前后台引擎", "Connected engine"), engine_version),
            pair(self.t("接口版本", "API version"), self.snapshot.as_ref().map(|s| s.api_version.to_string()).unwrap_or_else(|| "—".into())),
            row![button(self.t("检查更新", "Check for updates")).on_press_maybe((!self.busy && !preview_only()).then_some(Message::CheckUpdates)),
                button(self.t("发布说明与下载", "Release notes and downloads"))
                    .on_press(Message::OpenReleasePage).style(action_style),
                button(self.t("安装已验证更新…", "Install a verified update…"))
                    .on_press_maybe((!self.busy).then_some(Message::Ask(Confirmation::Maintenance(MaintenanceAction::InstallUpdate)))).style(button::secondary),
                button(self.t("恢复上一已验证版本…", "Restore the previous verified version…"))
                    .on_press_maybe((!self.busy).then_some(Message::Ask(Confirmation::Maintenance(MaintenanceAction::RollbackUpdate)))).style(button::secondary)].spacing(12).wrap(),
            text(self.t("安装前校验发布签名；系统后台每次升级或回滚都需要管理员确认。回滚仅限本机已缓存且兼容的上一已验证版本。", "Release signatures are checked before installation. Every service update or rollback requires administrator approval. Rollback requires a compatible previous verified version cached on this computer.")),
        ].spacing(14)),
        card(column![
            checkbox(self.draft.settings.check_updates).label(self.t("允许检查新版本", "Allow update checks")).on_toggle(|value| Message::FlagChanged(SettingFlag::CheckUpdates, value)),
            checkbox(self.draft.settings.auto_update).label(self.t("自动安装无需提权的个人版更新", "Automatically install personal updates that need no elevation"))
                .on_toggle(|value| Message::FlagChanged(SettingFlag::AutoUpdate, value)),
            button(self.t("保存更新偏好", "Save update preferences")).on_press_maybe((!self.busy && self.draft.dirty && self.snapshot.is_some()).then_some(Message::SaveSettings)),
            text(self.t("也可运行完整离线安装包更新；账户、网络规则和后台身份将保留。", "You can also update using the complete offline installer; accounts, rules and the service identity are preserved.")).size(13),
        ].spacing(14)),
        ].spacing(18).boxed()
    }

    fn confirmation_view(&self, confirmation: &Confirmation) -> Element<'_, Message> {
        let (title, detail) = match confirmation {
            Confirmation::DeleteProfile(_) => (self.t("删除保存的认证环境？", "Delete this authentication profile?"), self.t("删除该环境快照，不删除账户密码。实际使用的认证目标和当前会话由引擎按统一规则处理。", "Remove this saved profile without deleting account passwords. The engine applies its shared rules to the active configuration and session.")),
            Confirmation::LoginOnce => (self.t("只在当前网络登录一次？", "Connect once on this untrusted network?"), self.t("确认当前网络和认证目标由你信任。本次操作会发送认证材料，但不会保存长期网络信任；后续自动认证仍需明确“信任当前网络”。HTTP 门户无法提供加密的服务器身份保证。", "Confirm that you trust this network and authentication endpoint. This sends authentication material once without saving network trust. Automatic authentication still requires trusting the network. An HTTP portal provides no cryptographic server identity guarantee.")),
            Confirmation::TemporaryLogin { .. } => (self.t("确认仅登录本次？", "Confirm this one-time sign-in?"), self.t("请确认下方网卡和网关属于校园网。凭据只用于此次登录。", "Confirm the adapter and gateway belong to the campus network. Your credentials are used for this sign-in only.")),
            Confirmation::TrustNetwork => (self.t("信任当前网络？", "Trust this network?"), self.t("确认这是校园认证网络。信任后，BitNet 可在此网络自动使用已保存的账号登录。", "Confirm that this is the campus authentication network. Once trusted, BitNet can use your saved account to sign in automatically on this network.")),
            Confirmation::EnableAutomatic { .. } => (self.t("信任当前网络并启用自动认证？", "Trust this network and enable automatic authentication?"), self.t("确认这是校园认证网络。启用后，BitNet 会在已信任的网络自动登录。", "Confirm that this is the campus authentication network. BitNet will then sign in automatically on trusted networks.")),
            Confirmation::VerifyAccount { .. } => (self.t("验证这个校园账户？", "Verify this campus account?"), self.t("请确认这是校园认证网络。如果当前已登录，BitNet 会先退出该会话，再用新凭据尝试登录。网络可能短暂中断，验证通过后会保存并选中这个账户。", "Confirm that this network uses the campus portal. BitNet will sign out of any active session, then try the new credentials. The connection may drop briefly. A successful test saves and selects this account.")),
            Confirmation::Logout => (self.t("确认注销校园网？", "Disconnect the campus session?"), self.t("可能影响共享这个认证会话的设备。注销后自动认证保持暂停，直到你主动恢复。", "Devices sharing this session may lose connectivity. Automatic authentication stays paused until you resume it.")),
            Confirmation::ForceLogin => (self.t("强制重新认证？", "Force reauthentication?"), self.t("将执行完整的重新认证流程，可能影响共享连接。", "A full reauthentication may affect devices sharing this connection.")),
            Confirmation::DeleteAccount(_) => (self.t("删除保存的账户？", "Delete the saved account?"), self.t("账户及保存的密码将被删除。不会自动注销当前网络会话。", "The account and saved password will be removed. The current network session is not disconnected automatically.")),
            Confirmation::DeleteRule(_) => (self.t("删除网络规则？", "Delete this network rule?"), self.t("更改将立即保存，之后的自动认证使用剩余规则。", "This change is saved immediately. Future authentication uses the remaining rules.")),
            Confirmation::Shutdown => (self.t("停止个人认证引擎？", "Stop the user engine?"), self.t("自动认证和状态检测将停止。当前校园网会话不会自动注销。", "Automatic authentication and monitoring will stop. The campus session is not disconnected automatically.")),
            Confirmation::Maintenance(action) => match action {
                MaintenanceAction::ServiceInstall => (self.t("启用系统后台？", "Enable the system service?"), self.t("安装器将请求管理员权限，并启用开机无人值守运行。请在后台账户页面确认共享账户已配置。", "The installer requests administrator approval and enables unattended startup. Confirm that a shared account is configured afterwards.")),
                MaintenanceAction::ServiceStart => (self.t("启动系统后台？", "Start the system service?"), self.t("此操作可能请求管理员确认，之后将重新连接后台。", "Administrator approval may be requested before the interface reconnects to the service.")),
                MaintenanceAction::ServiceStop => (self.t("停止共享系统后台？", "Stop the shared system service?"), self.t("所有本机用户的自动认证和监测都会停止。现有校园网会话不会自动注销。", "Automatic authentication and monitoring stop for all local users. The current campus session is not disconnected automatically.")),
                MaintenanceAction::ServiceRemove { .. } => (self.t("卸载共享系统后台？", "Remove the shared system service?"), self.t("所有本机用户将失去开机无人值守能力。保留设置与受保护凭据；此操作需要管理员确认。", "Unattended startup is removed for all local users. Settings and protected credentials are retained. Administrator approval is required.")),
                MaintenanceAction::Repair => (self.t("修复 BitNet 安装？", "Repair the BitNet installation?"), self.t("使用受信任的安装包修复文件和后台注册。系统级更改需要管理员确认。", "A trusted installer repairs files and service registration. System changes require administrator approval.")),
                MaintenanceAction::InstallUpdate => (self.t("安装软件更新？", "Install the software update?"), self.t("将验证发布签名并运行更新安装。后台升级需要本次管理员确认，取消后保留当前版本。", "The release signature is verified before installation. Updating the service requires administrator approval for this update. Cancelling keeps the current version.")),
                MaintenanceAction::RollbackUpdate => (self.t("恢复上一已验证版本？", "Restore the previous verified version?"), self.t("只允许使用本机上次验签且 API/schema 兼容的缓存版本，执行前会再次验证。没有可用缓存时操作会明确失败，不从未知来源下载旧版。系统后台回滚需本次管理员确认。", "Only a previously verified local cache with a compatible API and schema can be restored, and it is verified again before use. If no usable cache exists, this operation reports unavailable. Service rollback requires administrator approval for this operation.")),
                MaintenanceAction::CheckUpdate | MaintenanceAction::SetAutostart { .. } | MaintenanceAction::SetGuiAutostart { .. } => (self.t("应用维护操作？", "Apply this maintenance operation?"), self.t("将通过统一维护流程应用更改。", "The shared maintenance workflow will apply this change.")),
            },
        };
        let mut contents = column![text(title).size(25)];
        if let Confirmation::EnableAutomatic { adapter, .. } = confirmation {
            contents = contents.push(text(adapter.clone()).size(18).boxed());
        }
        if let Confirmation::VerifyAccount {
            username, adapter, ..
        } = confirmation
        {
            contents = contents.push(text(format!("{username} · {adapter}")).size(18).boxed());
        }
        if let Confirmation::TemporaryLogin {
            username,
            adapter,
            gateway,
            ..
        } = confirmation
        {
            contents = contents
                .push(text(format!("{username} · {adapter}")).size(18).boxed())
                .push(text(gateway.clone()).boxed());
        }
        container(
            card(
                contents
                    .push(text(detail).boxed())
                    .push(
                        row![
                            button(self.t("取消", "Cancel"))
                                .on_press(Message::Cancel)
                                .style(button::secondary),
                            button(match confirmation {
                                Confirmation::VerifyAccount { .. } =>
                                    self.t("确认并验证", "Verify sign-in"),
                                Confirmation::TemporaryLogin { .. } =>
                                    self.t("确认登录", "Sign in"),
                                _ => self.t("确认继续", "Confirm"),
                            })
                            .on_press_maybe(
                                (!self.busy
                                    && (!matches!(
                                        confirmation,
                                        Confirmation::VerifyAccount { .. }
                                            | Confirmation::TemporaryLogin { .. }
                                    ) || self.snapshot_current))
                                    .then_some(Message::Confirm)
                            )
                            .style(button::danger)
                        ]
                        .spacing(14)
                        .boxed(),
                    )
                    .spacing(20),
            )
            .width(Length::Fill.max(640)),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(24)
        .style(scrim)
        .boxed()
    }

    fn yes_no(&self, value: bool) -> &'static str {
        if value {
            self.t("是", "Yes")
        } else {
            self.t("否", "No")
        }
    }
    fn service_value(&self, last: bool) -> String {
        if self.snapshot_current {
            self.yes_no(last).into()
        } else {
            format!(
                "{} ({})",
                self.t("当前未知，上次", "Unknown now; previously"),
                self.yes_no(last)
            )
        }
    }
}
