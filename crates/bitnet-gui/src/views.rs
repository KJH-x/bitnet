use super::*;
use iced::{
    alignment,
    widget::{
        button, checkbox, column, container, pick_list, row, rule, scrollable, text, text_editor,
        text_input, Space,
    },
    Length,
};

fn card<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .padding(20)
        .width(Length::Fill)
        .style(container::rounded_box)
}
fn pair<'a>(label: &'a str, value: impl ToString) -> iced::widget::Row<'a, Message> {
    row![
        text(label).width(180),
        text(value.to_string()).width(Length::Fill)
    ]
    .spacing(12)
}

impl App {
    pub(super) fn shell(&self) -> Element<'_, Message> {
        let content = if let Some(confirmation) = &self.confirmation {
            self.confirmation_view(confirmation)
        } else {
            self.main_shell()
        };
        if self.snapshot.as_ref().is_some_and(|snapshot| {
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
            .into()
        } else {
            content
        }
    }

    fn main_shell(&self) -> Element<'_, Message> {
        let mut navigation = column![
            text("BitNet").size(32),
            text(self.t("校园网连接管理器", "Campus connection manager")).size(12),
            Space::new().height(18),
        ]
        .spacing(7);
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
                    }),
            );
        }
        navigation = navigation
            .push(Space::new().height(Length::Fill))
            .push(text(format!("v{APP_VERSION}")).size(11))
            .push(
                button(self.t("隐藏界面", "Hide window"))
                    .on_press_maybe(
                        self.tray
                            .as_ref()
                            .and(self.window)
                            .map(Message::WindowCloseRequested),
                    )
                    .style(button::text),
            );

        let actual_scope = self.snapshot.as_ref().map(|s| s.scope);
        let scope = match actual_scope {
            Some(Scope::System) => self.t("系统共享后台", "Shared system service"),
            Some(Scope::User) => self.t("个人桌面引擎", "User engine"),
            None => self.t("尚未连接", "Not connected"),
        };
        let header = row![
            column![
                text(self.page.label(self.english())).size(27),
                text(scope).size(13)
            ]
            .spacing(4),
            Space::new().width(Length::Fill),
            pick_list(
                [Mode::User, Mode::System],
                Some(self.mode),
                Message::ModeChanged
            )
            .width(225),
            button(self.t("刷新", "Refresh")).on_press_maybe(
                (!self.busy && !self.connecting && !self.polling).then_some(Message::Refresh)
            ),
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);
        let mut content = column![header].spacing(18);
        if let Some((is_error, message)) = &self.notice {
            let banner = row![
                text(message).width(Length::Fill),
                button(self.t("关闭", "Dismiss"))
                    .on_press(Message::DismissNotice)
                    .style(button::text)
            ]
            .spacing(12);
            content = content.push(container(banner).padding(12).width(Length::Fill).style(
                if *is_error {
                    container::danger
                } else {
                    container::success
                },
            ));
        }
        if self.snapshot.is_some() && !self.snapshot_current && !self.connecting {
            content = content.push(text(self.t("后台连接已断开，以下保留最后成功快照；请重新连接或使用服务维护。", "The engine is unavailable. The last successful snapshot is shown below; reconnect or use service maintenance.")).size(13));
        }
        if self.busy || self.connecting {
            content = content.push(
                text(if self.connecting {
                    self.t("正在连接本机引擎…", "Connecting to the local engine…")
                } else {
                    self.t("正在处理，请稍候…", "Working, please wait…")
                })
                .size(13),
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
            Page::Updates => self.updates_view(),
        });
        row![
            container(navigation)
                .width(210)
                .height(Length::Fill)
                .padding(20)
                .style(container::rounded_box),
            scrollable(container(content).padding(26).width(Length::Fill))
                .height(Length::Fill)
                .width(Length::Fill)
        ]
        .height(Length::Fill)
        .into()
    }

    fn status_view(&self) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return card(column![text(self.t("先连接本机引擎", "Connect to the local engine first")).size(22),
                text(self.t("桌面模式会启动当前用户的引擎。已安装系统服务时，将连接该服务。", "Desktop mode starts your user engine. If a system service is installed, BitNet connects to it.")),
                button(self.t("连接 / 重试", "Connect / retry")).on_press_maybe((!self.connecting).then_some(Message::Refresh))].spacing(16)).into();
        };
        let auth = match snapshot.session.auth {
            AuthState::Authenticated => self.t("已认证", "Authenticated"),
            AuthState::Unauthenticated => self.t("尚未认证", "Not authenticated"),
            AuthState::Unknown => self.t("状态待确认", "Authentication unknown"),
        };
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
                    button(self.t("登录", "Connect")).on_press_maybe(
                        (!self.busy && !snapshot.accounts.is_empty())
                            .then_some(Self::login_message(snapshot))
                    ),
                    button(self.t("注销…", "Disconnect…"))
                        .on_press_maybe((!self.busy).then_some(Message::Ask(Confirmation::Logout)))
                        .style(button::secondary),
                    button(self.t("立即检测", "Check now"))
                        .on_press_maybe((!self.busy).then_some(Message::Run(Command::Check)))
                        .style(button::secondary),
                    button(if snapshot.settings.paused {
                        self.t("恢复自动认证", "Resume")
                    } else {
                        self.t("暂停自动认证", "Pause")
                    })
                    .on_press_maybe((!self.busy).then_some(Message::Run(
                        if snapshot.settings.paused {
                            Command::Resume
                        } else {
                            Command::Pause
                        }
                    )))
                    .style(button::secondary),
                    button(self.t("重新认证…", "Reauthenticate…"))
                        .on_press_maybe(
                            (!self.busy && !snapshot.accounts.is_empty())
                                .then_some(Message::Ask(Confirmation::ForceLogin))
                        )
                        .style(button::text),
                ]
                .spacing(10)
                .wrap(),
            ]
            .spacing(12),
        );
        let mut content = column![state].spacing(18);
        if snapshot
            .capabilities
            .iter()
            .any(|capability| capability == "network_trust")
        {
            content = content.push(card(column![
                text(if snapshot.network_trusted {
                    self.t("当前网络已信任", "Current network is trusted")
                } else {
                    self.t("当前网络尚未信任", "Current network is not trusted")
                }).size(20),
                text(self.t(
                    "未知网络仅查询认证状态，不自动发送账户凭据。信任按网卡、SSID、网关、来源网段及认证目标识别；HTTP 校园门户不提供加密的服务器身份保证。",
                    "Unknown networks are checked without sending account credentials automatically. Trust uses the interface, SSID, gateway, source subnet and authentication endpoint. An HTTP campus portal provides no cryptographic server identity guarantee."
                )).size(13),
                if snapshot.network_trusted {
                    button(self.t("取消当前网络信任", "Stop trusting this network"))
                        .on_press_maybe((!self.busy).then_some(Message::Run(Command::TrustNetwork { trusted: false })))
                        .style(button::secondary)
                } else {
                    button(self.t("信任当前网络…", "Trust this network…"))
                        .on_press_maybe((!self.busy && snapshot.active_binding.is_some()).then_some(Message::Ask(Confirmation::TrustNetwork)))
                        .style(button::secondary)
                },
            ].spacing(12)));
        }
        if snapshot.accounts.is_empty() {
            content = content.push(card(column![
                text(self.t("欢迎使用 BitNet", "Welcome to BitNet")).size(22),
                text(self.t("1. 添加校园网账号和密码。\n2. 选择是否自动认证。\n3. 需要在 Windows/macOS/Linux 登录前联网时，再启用系统后台。", "1. Add your campus account and password.\n2. Choose whether to authenticate automatically.\n3. Enable the system service when you need connectivity before signing in.")),
                button(self.t("添加第一个账户", "Add your first account")).on_press(Message::Navigate(Page::Accounts)),
            ].spacing(12)));
        }
        if let Some(error) = &snapshot.last_error {
            content = content.push(card(
                column![
                    text(self.t("需要关注", "Needs attention")).size(20),
                    text(&error.message),
                    button(self.t("打开诊断", "Open diagnostics"))
                        .on_press(Message::Navigate(Page::Diagnostics))
                ]
                .spacing(12),
            ));
        }
        content = content.push(card(column![text(self.t("连接健康", "Connection health")).size(20),
            pair(self.t("互联网", "Internet"), self.health_label(&snapshot.health.internet)),
            pair("DNS", self.health_label(&snapshot.health.dns)), pair("IPv6", self.health_label(&snapshot.health.ipv6)),
            pair(self.t("最近成功认证", "Last successful login"), format_timestamp(snapshot.last_success_at)),
            pair(self.t("重试次数", "Retry attempts"), snapshot.retry_count),
            text(self.t("认证状态与互联网可用性分开判断。DNS 或外网异常不会自动注销校园网。", "Authentication and internet health are separate. DNS or internet failures do not disconnect a campus session.")).size(12),
        ].spacing(10)));
        content.into()
    }

    fn accounts_view(&self) -> Element<'_, Message> {
        let mut content = column![text(self.t(
            "保存的密码只允许覆盖，不会显示或导出。",
            "Saved passwords can be replaced, never displayed or exported."
        ))
        .size(14)]
        .spacing(16);
        if let Some(snapshot) = &self.snapshot {
            for account in &snapshot.accounts {
                let selected = snapshot.settings.selected_account.as_deref() == Some(&account.id);
                content = content.push(card(
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
                                .style(button::secondary),
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
                ));
            }
        }
        content = content.push(card(column![
            text(if self.account.id.is_some() { self.t("覆盖修改账户", "Replace account details") } else { self.t("添加账户", "Add account") }).size(21),
            text_input(self.t("校园账号", "Campus username"), &self.account.username).on_input(Message::UsernameChanged).padding(12),
            text_input(self.t("输入新的密码", "Enter a new password"), &self.account.password).secure(true).on_input(Message::PasswordChanged).padding(12),
            row![button(self.t("安全保存账户", "Save account securely")).on_press_maybe((!self.busy && self.snapshot.is_some()).then_some(Message::AccountSave)),
                button(self.t("清空 / 取消", "Clear / cancel")).on_press(Message::AccountCancel).style(button::secondary)].spacing(10),
            checkbox(self.draft.settings.automatic).label(self.t("启用校园网络自动认证", "Authenticate automatically on campus networks"))
                .on_toggle(|value| Message::FlagChanged(SettingFlag::Automatic, value)),
            button(self.t("保存自动认证设置", "Save automatic authentication preference"))
                .on_press_maybe((!self.busy && self.snapshot.is_some() && self.draft.dirty).then_some(Message::SaveSettings)).style(button::secondary),
            text(self.t("共享后台中的账户可由本机用户管理。认证服务器与软件更新权限由管理员控制。", "Local users can manage shared accounts. Authentication endpoints and software updates remain administrator-controlled.")).size(12),
        ].spacing(13)));
        content.into()
    }

    fn networks_view(&self) -> Element<'_, Message> {
        let mut content = column![text(self.t("网卡选择与网络规则由统一引擎执行，不改变系统路由。", "Interface selection and network rules are enforced by the engine without changing system routes.")).size(14)].spacing(16);
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
                    .spacing(6),
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
                .spacing(10),
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
                .style(button::secondary),
            );
        if self.draft.dirty {
            profiles = profiles.push(text(self.t("有未保存设置：请先到偏好页保存或放弃，再切换或保存环境。", "Pending edits: save or discard preferences before capturing or switching profiles.")).size(12));
        }
        content = content.push(card(profiles));
        if let Some(snapshot) = &self.snapshot {
            for interface in &snapshot.interfaces {
                let active = snapshot
                    .active_binding
                    .as_ref()
                    .is_some_and(|binding| binding.interface_id == interface.id);
                let source = interface
                    .ipv4
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                content = content.push(card(column![
                    row![text(&interface.name).size(20), Space::new().width(Length::Fill), text(if active { self.t("当前出口", "Active interface") } else if interface.up { self.t("已连接", "Connected") } else { self.t("未连接", "Disconnected") })],
                    pair("ID", &interface.id), pair("IPv4", if source.is_empty() { "—".into() } else { source }),
                    pair(self.t("网关", "Gateway"), interface.gateway.map(|ip| ip.to_string()).unwrap_or_else(|| "—".into())),
                    pair("SSID", interface.ssid.as_deref().unwrap_or("—")),
                    text(if interface.virtual_adapter { self.t("虚拟网卡：需要明确的校园网归属，不能仅凭默认路由认证。", "Virtual adapter: campus ownership must be confirmed before authentication.") } else { self.t("物理网络接口", "Physical network interface") }).size(12),
                    button(self.t("使用此网卡", "Use this interface")).on_press_maybe((!self.busy && interface.up).then_some(Message::InterfaceSelected(interface.id.clone()))).style(button::secondary),
                ].spacing(9)));
            }
        }
        content = content.push(
            button(self.t("恢复自动选择网卡", "Select an interface automatically"))
                .on_press_maybe(
                    (!self.busy && self.snapshot.is_some())
                        .then_some(Message::InterfaceSelected(String::new())),
                )
                .style(button::secondary),
        );
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
                .wrap(),
            );
        }
        rules = rules
            .push(rule::horizontal(1))
            .push(
                text(if self.rule.index.is_some() {
                    self.t("编辑规则", "Edit rule")
                } else {
                    self.t("添加规则", "Add rule")
                })
                .size(18),
            )
            .push(
                text_input(self.t("规则名称", "Rule name"), &self.rule.name)
                    .on_input(Message::RuleNameChanged)
                    .padding(10),
            )
            .push(
                text_input(
                    self.t("网卡 ID（留空匹配任何网卡）", "Interface ID (empty = any)"),
                    &self.rule.interface_id,
                )
                .on_input(Message::RuleInterfaceChanged)
                .padding(10),
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
                .spacing(10),
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
                .spacing(10),
            )
            .push(
                checkbox(self.rule.ignore)
                    .label(self.t(
                        "忽略匹配网络，不自动认证",
                        "Ignore matching networks; do not authenticate automatically",
                    ))
                    .on_toggle(Message::RuleIgnoreChanged),
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
                .spacing(10),
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
                .size(12),
            );
        }
        content.push(card(rules)).into()
    }

    fn settings_view(&self) -> Element<'_, Message> {
        let mut preferences = column![
            row![
                text(self.t("日志级别", "Log level")).width(180),
                pick_list(
                    ["error", "warning", "info", "debug"],
                    Some(self.draft.settings.log_level.as_str()),
                    |level| Message::LogLevelChanged(level.to_string())
                )
            ]
            .spacing(12),
            text(self.t("日常偏好", "Preferences")).size(21),
            row![
                text(self.t("界面语言", "Language")).width(180),
                pick_list(
                    [Language::Chinese, Language::English],
                    Some(self.language),
                    Message::LanguageChanged
                )
            ]
            .spacing(12),
            row![
                text(self.t("主题", "Theme")).width(180),
                pick_list(
                    [Appearance::System, Appearance::Light, Appearance::Dark],
                    Some(self.appearance),
                    Message::AppearanceChanged
                )
            ]
            .spacing(12),
        ]
        .spacing(14);
        for (flag, value, label) in [
            (
                SettingFlag::Automatic,
                self.draft.settings.automatic,
                self.t("自动认证", "Automatic authentication"),
            ),
            (
                SettingFlag::UserAutostart,
                self.draft.settings.user_autostart,
                self.t(
                    "登录后保持后台运行",
                    "Start the background engine after signing in",
                ),
            ),
            (
                SettingFlag::GuiAutostart,
                self.draft.settings.gui_autostart,
                self.t("登录后打开界面", "Open the window after signing in"),
            ),
            (
                SettingFlag::StartHidden,
                self.draft.settings.start_hidden,
                self.t("启动时隐藏主窗口", "Start with the main window hidden"),
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
            preferences = preferences.push(checkbox(value).label(label).on_toggle_maybe(
                enabled.then_some(move |value| Message::FlagChanged(flag, value)),
            ));
        }
        if self.mode == Mode::System {
            preferences = preferences.push(text(self.t("个人登录启动项请切换到“自动选择”模式修改；该模式仍优先连接已安装的系统服务。", "Switch to Automatic mode to edit your personal login items. Automatic mode still prefers the installed system service.")).size(12));
        }
        if self.tray.is_none() {
            preferences = preferences.push(text(self.t("当前桌面未提供系统托盘；隐藏启动时将保留可访问窗口。", "No system tray is available on this desktop; hidden startup keeps an accessible window.")).size(12));
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
                .align_y(alignment::Vertical::Center),
            );
        }
        advanced = advanced.push(text(self.t("健康探测地址（用分号分隔，最多 8 个）", "Health check URLs (semicolon-separated, up to 8)")).size(13))
            .push(text_input("http://…; http://…", &self.draft.health_targets).on_input(Message::HealthTargetsChanged).padding(10))
            .push(pair(self.t("认证服务器", "Authentication endpoint"), &self.draft.settings.gateway.base_url))
            .push(text(self.t("系统后台的认证地址、安全参数及更新执行权限由管理员控制。", "System authentication endpoints, security policy and update execution are administrator-controlled.")).size(12));
        let save = row![
            button(self.t("保存设置", "Save settings")).on_press_maybe(
                (self.draft.dirty && !self.busy && self.snapshot.is_some())
                    .then_some(Message::SaveSettings)
            ),
            button(self.t("放弃修改", "Discard changes"))
                .on_press_maybe(self.draft.dirty.then_some(Message::RevertSettings))
                .style(button::secondary),
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
            transfer = transfer.push(scrollable(text(&self.exported).size(12)).height(150));
        }
        transfer = transfer
            .push(
                text_editor(&self.import_content)
                    .placeholder(self.t(
                        "在此粘贴 BitNet 设置 JSON",
                        "Paste BitNet settings JSON here",
                    ))
                    .height(160)
                    .on_action(Message::ImportAction),
            )
            .push(
                button(self.t("导入并保存设置", "Import and save settings"))
                    .on_press_maybe(
                        (!self.busy && self.snapshot.is_some()).then_some(Message::ImportSettings),
                    )
                    .style(button::secondary),
            );
        column![card(preferences), card(advanced), save, card(transfer)]
            .spacing(18)
            .into()
    }

    fn service_view(&self) -> Element<'_, Message> {
        let stop_hint = text(self.t("停止系统后台会持续关闭后台（Windows 同时关闭服务自动启动），直到你再次启动；仅想暂时停止认证，请使用“暂停自动认证”。", "Stopping the system service keeps it disabled until you start it again (including Windows service startup). To suspend authentication temporarily, use Pause instead.")).size(13);
        let mut content = column![card(column![text(self.t("开机无人值守联网", "Connect before signing in")).size(24),
            text(self.t("系统后台在没有用户登录时运行。关闭界面不会结束后台认证。", "The background service runs without an interactive login. Closing the window does not stop authentication.")),
        ].spacing(12))].spacing(18);
        content = content.push(stop_hint);
        if let Some(snapshot) = &self.snapshot {
            let service = &snapshot.service;
            content = content.push(card(
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
                            .on_press_maybe((!self.busy).then_some(Message::Run(Command::Check)))
                            .style(button::secondary)
                    ]
                    .spacing(10),
                ]
                .spacing(12),
            ));
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
            content = content.push(actions.wrap());
            if snapshot.scope == Scope::User {
                content = content.push(
                    button(self.t("停止个人引擎…", "Stop user engine…"))
                        .on_press_maybe(
                            (!self.busy).then_some(Message::Ask(Confirmation::Shutdown)),
                        )
                        .style(button::text),
                );
            }
        } else {
            content = content
                .push(
                    button(self.t("安装 / 修复系统后台…", "Install / repair system service…"))
                        .on_press_maybe((!self.busy && !self.connecting).then_some(Message::Ask(
                            Confirmation::Maintenance(MaintenanceAction::ServiceInstall),
                        ))),
                )
                .push(
                    button(self.t(
                        "启动已安装的系统后台…",
                        "Start an installed system service…",
                    ))
                    .on_press_maybe((!self.busy && !self.connecting).then_some(Message::Ask(
                        Confirmation::Maintenance(MaintenanceAction::ServiceStart),
                    )))
                    .style(button::secondary),
                );
        }
        content.push(card(column![text(self.t("管理说明", "Management")).size(21),
            text(self.t("安装、卸载和更新系统后台需要管理员确认。修改校园账号、暂停和恢复自动认证不需要管理员权限。", "Installing, removing or updating the system service requires administrator approval. Managing campus accounts and pausing or resuming authentication does not.")),
            text(self.t("服务未运行时不会自动改用第二个桌面控制器。可先使用安装包的后台服务维护选项。", "An unavailable installed service does not trigger a second desktop controller. Use the installer's service maintenance option when needed.")).size(13),
            text(self.t("系统磁盘与网络需已解锁；软件不会绕过磁盘启动密码或 VPN 网络策略。", "The system disk and network must be unlocked. BitNet does not bypass disk startup passwords or VPN policy.")).size(12),
        ].spacing(13))).into()
    }

    fn history_view(&self) -> Element<'_, Message> {
        let mut content = column![row![text_input(self.t("按类型或内容筛选", "Filter by event type or message"), &self.history_filter).on_input(Message::HistoryFilterChanged).padding(10),
            button(self.t("刷新历史", "Refresh activity")).on_press_maybe((!self.busy).then_some(Message::Run(Command::History { limit: 200 })))].spacing(12),
            text(self.t("记录只包含脱敏事件。正常周期检查不会逐次写入日志。", "Only redacted activity is shown. Routine periodic checks are not logged individually.")).size(12),
        ].spacing(14);
        let filter = self.history_filter.to_lowercase();
        let mut shown = 0;
        for event in self.history.iter().rev().filter(|event| {
            filter.is_empty()
                || event.kind.to_lowercase().contains(&filter)
                || event.message.to_lowercase().contains(&filter)
        }) {
            shown += 1;
            let mut event_content = column![
                row![
                    text(&event.kind).size(16),
                    Space::new().width(Length::Fill),
                    text(format_timestamp(Some(event.at))).size(12),
                    button(self.t("详情", "Details"))
                        .on_press(Message::HistoryDetailToggled(event.id))
                        .style(button::text)
                ],
                text(&event.message),
                text(format!(
                    "#{}  {}",
                    event.id,
                    event.account.as_deref().unwrap_or("")
                ))
                .size(11)
            ]
            .spacing(8);
            if self.expanded_history == Some(event.id) {
                event_content = event_content.push(
                    text(serde_json::to_string_pretty(&event.details).unwrap_or_default()).size(12),
                );
            }
            content = content.push(card(event_content));
        }
        if shown == 0 {
            content = content.push(card(text(self.t("暂无匹配记录", "No matching activity"))));
        }
        content.into()
    }

    fn statistics_view(&self) -> Element<'_, Message> {
        let mut content = column![button(self.t("刷新统计", "Refresh statistics"))
            .on_press_maybe((!self.busy).then_some(Message::Run(Command::Statistics)))]
        .spacing(18);
        if let Some(statistics) = &self.statistics {
            content = content.push(card(
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
                    pair(self.t("观测在线总时长", "Observed online duration"), format_duration(statistics.observed_online_seconds)),
                    pair(self.t("最长观测在线段", "Longest observed online segment"), format_duration(statistics.longest_observed_online_seconds)),
                    pair(self.t("观测会话中断", "Observed session interruptions"), statistics.session_interruptions),
                    pair(self.t("健康异常次数", "Health anomalies"), statistics.health_anomalies),
                    text(self.t("观测时长来自本地监测，不等同于校方计费时间。", "Observed durations come from local monitoring and are not campus billing time.")).size(12),
                    pair(
                        self.t("最近成功", "Last success"),
                        format_timestamp(statistics.last_success_at)
                    ),
                    pair(
                        self.t("平均恢复时间", "Average recovery time"),
                        statistics
                            .average_recovery_ms
                            .map(|ms| format!("{:.1} s", ms as f64 / 1000.0))
                            .unwrap_or_else(|| "—".into())
                    ),
                ]
                .spacing(14),
            ));
        } else {
            content = content.push(card(text(self.t(
                "正在获取统计或尚无记录",
                "Loading statistics, or no activity is available yet",
            ))));
        }
        if let Some(snapshot) = &self.snapshot {
            content = content.push(card(
                column![
                    text(self.t("校方网关统计", "Campus gateway totals")).size(21),
                    pair(
                        self.t("网关累计在线时长", "Gateway accumulated online time"),
                        snapshot
                            .session
                            .online_seconds
                            .map(format_duration)
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
                    text(self.t(
                        "统计周期和计费口径以校方网关为准，不代表本次连续会话。",
                        "The gateway defines the accounting period and billing rules; these are not totals for this connection.",
                    )).size(12),
                    pair(
                        self.t("网络变化代次", "Network generation"),
                        snapshot.network_generation
                    ),
                ]
                .spacing(12),
            ));
        }
        content.into()
    }

    fn diagnostics_view(&self) -> Element<'_, Message> {
        let mut content = column![card(column![text(self.t("按需诊断", "Diagnostics on demand")).size(24),
            text(self.t("诊断由统一引擎执行，结果已脱敏；不会主动注销、重置网卡或修改 DNS。", "The engine produces redacted diagnostics without logging out, resetting adapters or changing DNS.")),
            row![button(self.t("开始诊断", "Run diagnostics")).on_press_maybe((!self.busy).then_some(Message::Run(Command::Diagnose))),
                button(self.t("复制诊断报告", "Copy report")).on_press_maybe((!self.diagnostics.is_empty()).then_some(Message::CopyDiagnostics)).style(button::secondary)].spacing(12),
        ].spacing(13))].spacing(18);
        if !self.diagnostics.is_empty() {
            content = content.push(card(text(&self.diagnostics).size(13)));
        }
        content.into()
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
            row![button(self.t("检查更新", "Check for updates")).on_press_maybe((!self.busy).then_some(Message::Maintain(MaintenanceAction::CheckUpdate))),
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
        ].spacing(18).into()
    }

    fn confirmation_view(&self, confirmation: &Confirmation) -> Element<'_, Message> {
        let (title, detail) = match confirmation {
            Confirmation::DeleteProfile(_) => (self.t("删除保存的认证环境？", "Delete this authentication profile?"), self.t("删除该环境快照，不删除账户密码。实际使用的认证目标和当前会话由引擎按统一规则处理。", "Remove this saved profile without deleting account passwords. The engine applies its shared rules to the active configuration and session.")),
            Confirmation::LoginOnce => (self.t("只在当前网络登录一次？", "Connect once on this untrusted network?"), self.t("确认当前网络和认证目标由你信任。本次操作会发送认证材料，但不会保存长期网络信任；后续自动认证仍需明确“信任当前网络”。HTTP 门户无法提供加密的服务器身份保证。", "Confirm that you trust this network and authentication endpoint. This sends authentication material once without saving network trust. Automatic authentication still requires trusting the network. An HTTP portal provides no cryptographic server identity guarantee.")),
            Confirmation::TrustNetwork => (self.t("信任当前网络以允许自动认证？", "Trust this network for automatic authentication?"), self.t("仅在你已确认这是可用的校园认证环境时继续。引擎将记录当前接口、SSID、网关、来源网段和认证目标的指纹。此确认不是服务器证书验证，HTTP 门户无法抵御仿冒网络。", "Continue only after confirming this is a campus authentication environment. The engine records a fingerprint of the interface, SSID, gateway, source subnet and authentication endpoint. This is not certificate verification; an HTTP portal cannot protect against an impersonated network.")),
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
        container(
            card(
                column![
                    text(title).size(25),
                    text(detail),
                    row![
                        button(self.t("取消", "Cancel"))
                            .on_press(Message::Cancel)
                            .style(button::secondary),
                        button(self.t("确认继续", "Confirm"))
                            .on_press_maybe((!self.busy).then_some(Message::Confirm))
                            .style(button::danger)
                    ]
                    .spacing(14)
                ]
                .spacing(20),
            )
            .max_width(640),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(40)
        .into()
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
