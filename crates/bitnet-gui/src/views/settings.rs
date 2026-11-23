use super::{action_style, card, pair};
use crate::design;
use crate::settings_actions::{Action, ExportTo};
use crate::{
    format_log_size, preview_only, tray, App, Appearance, Confirmation, Language, Message, Mode,
    NumericSetting, SettingFlag, SettingsTab,
};
use bitnet_core::{Command, APP_VERSION};
use bitnet_launcher::WebAction;
use iced::{
    alignment,
    widget::{
        button, checkbox, column, container, pick_list, row, text, text_editor, text_input,
        toggler, Space,
    },
    Element, Length, Size,
};

impl SettingsTab {
    const ALL: [Self; 6] = [
        Self::General,
        Self::Checks,
        Self::Trust,
        Self::Diagnostics,
        Self::Updates,
        Self::Transfer,
    ];

    fn label(self, app: &App) -> &'static str {
        match self {
            Self::General => app.t("常规", "General"),
            Self::Checks => app.t("连接检测", "Connection checks"),
            Self::Trust => app.t("网络信任", "Trusted networks"),
            Self::Diagnostics => app.t("日志", "Logs"),
            Self::Updates => app.t("更新", "Updates"),
            Self::Transfer => app.t("导入与导出", "Import & export"),
        }
    }
}

impl App {
    pub(super) fn settings_workspace(&self, size: Size) -> Element<'_, Message> {
        let mut tabs = row![].spacing(6);
        let compact = size.height < 420.0;
        let d = design::get();
        for tab in SettingsTab::ALL {
            let selected = self.settings_tab == tab;
            tabs = tabs.push(
                button(text(tab.label(self)).size(d.typography.body))
                    .padding([d.space.xs, d.space.sm])
                    .on_press(Message::SettingsTabChanged(tab))
                    .style(design::nav_button(selected)),
            );
        }
        let section = match self.settings_tab {
            SettingsTab::General => self.general_settings(),
            SettingsTab::Checks => self.check_settings(),
            SettingsTab::Trust => self.trust_settings(size),
            SettingsTab::Diagnostics => self.diagnostics_settings(),
            SettingsTab::Updates => self.update_settings(),
            SettingsTab::Transfer => self.transfer_settings(),
        };
        let footer = row![
            button(text(self.t("保存设置", "Save settings")).size(d.typography.body))
                .on_press_maybe(
                    (self.draft.dirty && !self.busy && self.snapshot_current)
                        .then_some(Message::SaveSettings)
                )
                .padding([d.space.xs + 1.0, d.space.md])
                .style(design::solid),
            button(text(self.t("放弃修改", "Discard changes")).size(d.typography.body))
                .on_press_maybe(self.draft.dirty.then_some(Message::RevertSettings))
                .padding([d.space.xs + 1.0, d.space.md])
                .style(design::flat_button),
            Space::new().width(Length::Fill),
            text(if self.draft.dirty {
                self.t("有未保存修改", "Unsaved changes")
            } else if self.snapshot_current {
                self.t("设置已同步", "Settings synchronized")
            } else {
                self.t("后台未连接", "Engine disconnected")
            })
            .size(12),
        ]
        .spacing(8)
        .align_y(alignment::Vertical::Center);
        column![
            tabs.wrap(),
            container(section).height(Length::Fill).width(Length::Fill),
            footer,
        ]
        .height(Length::Fill)
        .spacing(if compact { 6 } else { 9 })
        .into()
    }

    fn trust_settings(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let mut items = Vec::new();
        if let Some(snapshot) = &self.snapshot {
            let settings = &snapshot.settings;
            items.extend(settings.network_permissions.iter().map(|item| {
                let label = match &item.ssid {
                    Some(ssid) => format!("{} ({ssid})", item.adapter_name),
                    None => item.adapter_name.clone(),
                };
                (label, &item.fingerprint, item.allowed)
            }));
            items.extend(
                settings
                    .trusted_networks
                    .iter()
                    .filter(|key| {
                        !settings
                            .network_permissions
                            .iter()
                            .any(|detail| detail.fingerprint.as_str() == key.as_str())
                    })
                    .map(|key| {
                        (
                            self.t("旧版信任记录", "Legacy trusted network").to_owned(),
                            key,
                            true,
                        )
                    }),
            );
        }
        let capacity = super::page_capacity(size.height, 170.0, 38.0, 10);
        let range = super::page_window(items.len(), self.trust_offset, capacity);
        let mut rows = column![text(self.t(
            "可移除已保存的允许或拒绝记录。再次进入该网络时，BitNet 会重新询问。",
            "Remove a saved decision to let BitNet ask again the next time you use that network."
        ))
        .size(d.typography.meta)
        .style(design::text_style(design::muted)),]
        .spacing(d.space.sm);
        if items.is_empty() {
            rows = rows.push(text(self.t("尚无记录", "No saved decisions")));
        }
        let editable = self.snapshot_current
            && !self.busy
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .capabilities
                    .iter()
                    .any(|item| item == "network_permissions")
            });
        for (label, fingerprint, allowed) in &items[range.clone()] {
            rows = rows.push(
                row![
                    container(text(label.clone()).wrapping(text::Wrapping::None))
                        .clip(true)
                        .width(Length::Fill),
                    text(if *allowed {
                        self.t("已信任", "Trusted")
                    } else {
                        self.t("不再询问", "Declined")
                    })
                    .size(d.typography.meta),
                    button(text(self.t("移除记录", "Forget")).size(d.typography.meta))
                        .on_press_maybe(editable.then(|| Message::Run(
                            Command::ForgetNetworkPermission {
                                fingerprint: (*fingerprint).clone(),
                            }
                        )))
                        .style(design::flat_button),
                ]
                .align_y(alignment::Vertical::Center)
                .spacing(d.space.md),
            );
        }
        if items.len() > capacity {
            rows = rows.push(self.page_controls(items.len(), range, capacity, Message::TrustPage));
        }
        card(rows).into()
    }

    fn general_settings(&self) -> Element<'_, Message> {
        let english = self.english();
        let d = design::get();
        let can_edit = self.mode == Mode::User && self.snapshot_current && !self.busy;
        let autorun = self.t("系统登录时启动 BitNet", "Start BitNet at sign-in");
        let hidden_unavailable = !self.draft.settings.gui_autostart;
        let resume = self.snapshot.as_ref().is_some_and(|snapshot| {
            snapshot.settings.automatic && (snapshot.settings.paused || snapshot.verification_hold)
        });
        let mut body = column![
            setting_row(
                self.t("界面语言", "Language").to_string(),
                setting_control(
                    pick_list(
                        Some(self.language),
                        [Language::Chinese, Language::English],
                        ToString::to_string,
                    )
                    .on_select(Message::LanguageChanged)
                    .width(Length::Fill)
                    .into(),
                ),
            ),
            setting_row(
                self.t("主题", "Theme").to_string(),
                setting_control(
                    pick_list(
                        Some(self.appearance),
                        [Appearance::System, Appearance::Light, Appearance::Dark],
                        move |value: &Appearance| value.label(english).to_string(),
                    )
                    .on_select(Message::AppearanceChanged)
                    .width(Length::Fill)
                    .into(),
                ),
            ),
            checkbox(self.draft.settings.automatic)
                .label(self.t("自动认证", "Automatic authentication"))
                .on_toggle_maybe(
                    (!self.busy && self.snapshot_current).then_some(Message::AutomaticChanged),
                ),
        ]
        .spacing(d.space.xs);
        if resume {
            body = body.push(
                button(text(
                    self.t("恢复自动认证", "Resume automatic authentication"),
                ))
                .on_press_maybe(
                    (self.snapshot_current && !self.busy && !self.draft.dirty)
                        .then_some(Message::Run(Command::Resume)),
                )
                .style(design::flat_button),
            );
        }
        body =
            body.push(Self::rule(false))
                .push(
                    text(self.t("启动与窗口", "Startup and window")).font(design::family_medium()),
                )
                .push(
                    checkbox(self.draft.settings.gui_autostart)
                        .label(autorun)
                        .on_toggle_maybe(can_edit.then_some(|value| {
                            Message::FlagChanged(SettingFlag::GuiAutostart, value)
                        })),
                )
                .push(
                    checkbox(self.draft.settings.start_hidden)
                        .label(self.t(
                            "自动启动时隐藏主窗口",
                            "Hide the window when started automatically",
                        ))
                        .on_toggle_maybe((can_edit && !hidden_unavailable).then_some(|value| {
                            Message::FlagChanged(SettingFlag::StartHidden, value)
                        })),
                );
        if hidden_unavailable {
            body = body.push(setting_hint(self.t(
                "先开启登录时启动 BitNet，才能选择隐藏窗口。",
                "Enable sign-in startup to choose whether the window stays hidden.",
            )));
        }
        body = body.push(
            checkbox(self.draft.settings.user_autostart)
                .label(self.t(
                    "系统登录时启动个人后台",
                    "Start the personal engine at sign-in",
                ))
                .on_toggle_maybe(
                    can_edit
                        .then_some(|value| Message::FlagChanged(SettingFlag::UserAutostart, value)),
                ),
        );
        if self.mode == Mode::System {
            body = body.push(setting_hint(self.t(
                "切回个人后台后可修改这些启动选项。",
                "Switch to the personal engine to edit these startup options.",
            )));
        }
        if !self.tray.as_ref().is_some_and(tray::Tray::is_available) {
            body = body.push(setting_hint(self.t(
                "系统托盘不可用，自动启动时也会显示窗口。",
                "Automatic startup opens the window while the system tray is unavailable.",
            )));
        }
        card(
            body.push(Self::rule(false))
                .push(text(self.t("通知", "Notifications")).font(design::family_medium()))
                .push(
                    checkbox(self.draft.settings.notify_errors)
                        .label(self.t("需要处理时才通知", "Notify when an error needs attention"))
                        .on_toggle(|value| Message::FlagChanged(SettingFlag::NotifyErrors, value)),
                ),
        )
        .padding(10)
        .into()
    }

    fn check_settings(&self) -> Element<'_, Message> {
        let d = design::get();
        let targets = &self.draft.settings.health_targets;
        let target_label = if self.english() {
            format!("Health check URLs ({})", targets.len())
        } else {
            format!("健康探测地址（{} 个）", targets.len())
        };
        let endpoint = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.settings.gateway.base_url.clone())
            .unwrap_or_else(|| "—".into());

        let mut body = column![
            setting_number(
                NumericSetting::StatusInterval,
                &self.draft.status_interval,
                self.t(
                    "认证状态间隔（5–3600 秒）",
                    "Authentication interval (5–3600 s)"
                ),
            ),
            setting_row(
                target_label,
                setting_control(
                    button(text(self.t("修改…", "Edit…")).size(d.typography.body))
                        .on_press(Message::OpenHealthEditor)
                        .padding([d.space.xs + 2.0, d.space.md])
                        .width(Length::Fill)
                        .style(design::flat_button)
                        .into(),
                ),
            ),
        ]
        .spacing(d.card.gap);

        for target in targets.iter().take(2) {
            body = body.push(sub_item(target));
        }

        let ipv6_target_count = self.draft.settings.ipv6_targets.len();
        body = body.push(setting_row(
            if self.english() {
                format!("Public IPv6 check URLs ({ipv6_target_count})")
            } else {
                format!("公网 IPv6 检测地址（{ipv6_target_count} 个）")
            },
            setting_control(
                button(text(self.t("修改…", "Edit…")).size(d.typography.body))
                    .on_press(Message::SettingsAction(Action::OpenIpv6Editor))
                    .padding([d.space.xs + 2.0, d.space.md])
                    .width(Length::Fill)
                    .style(design::flat_button)
                    .into(),
            ),
        ));
        body = body.push(setting_row(
            self.t("认证服务器", "Authentication endpoint").to_string(),
            container(text(endpoint).size(d.typography.body))
                .width(Length::Fill)
                .padding([d.space.xs, d.space.sm])
                .style(design::field)
                .into(),
        ));
        let mut advanced = row![
            toggler(self.settings_checks_advanced)
                .label(self.t("高级参数", "Advanced parameters"))
                .text_size(d.typography.body)
                .on_toggle(|_| Message::ToggleCheckAdvanced),
            Space::new().width(Length::Fill),
        ]
        .align_y(alignment::Vertical::Center);
        if self.settings_checks_advanced {
            advanced = advanced.push(
                button(text(self.t("恢复默认参数", "Restore defaults")).size(d.typography.body))
                    .on_press_maybe(
                        (self.snapshot_current && !self.busy)
                            .then_some(Message::RestoreCheckDefaults),
                    )
                    .style(design::flat_button),
            );
        }
        body = body.push(Self::rule(false)).push(advanced);
        if self.settings_checks_advanced {
            body = body
                .push(setting_number(
                    NumericSetting::HealthInterval,
                    &self.draft.health_interval,
                    self.t(
                        "健康探测间隔（30–86400 秒）",
                        "Health interval (30–86400 s)",
                    ),
                ))
                .push(setting_number(
                    NumericSetting::RetryMaximum,
                    &self.draft.retry_maximum,
                    self.t(
                        "最长重试等待（5–3600 秒）",
                        "Maximum retry delay (5–3600 s)",
                    ),
                ))
                .push(setting_number(
                    NumericSetting::RequestTimeout,
                    &self.draft.request_timeout,
                    self.t("单请求超时（1–60 秒）", "Request timeout (1–60 s)"),
                ));
        }

        card(body).padding(10).into()
    }

    fn diagnostics_settings(&self) -> Element<'_, Message> {
        let d = design::get();
        let english = self.english();
        let log_size = self.current_log_size();
        let clear = log_size
            .filter(|(_, bytes)| self.can_read_log_size() && *bytes > 0)
            .map(|(scope, bytes)| Message::Ask(Confirmation::ClearLogs { scope, bytes }));
        let clear_label = log_size.map_or_else(
            || self.t("清除诊断日志…", "Clear diagnostic logs…").to_owned(),
            |(_, bytes)| {
                format!(
                    "{} ({})",
                    self.t("清除诊断日志…", "Clear diagnostic logs…"),
                    format_log_size(bytes)
                )
            },
        );
        card(
            column![
                row![
                    button(
                        text(self.t("打开日志 Web 界面", "Open log view")).size(d.typography.meta)
                    )
                    .on_press_maybe(
                        (!self.log_viewer_busy).then_some(Message::WebLogs(WebAction::Open))
                    )
                    .padding([d.space.xs, d.space.md])
                    .style(design::flat_button),
                    button(
                        text(self.t("停止 Web 服务", "Stop web service")).size(d.typography.meta)
                    )
                    .on_press_maybe(
                        (!self.log_viewer_busy).then_some(Message::WebLogs(WebAction::Stop))
                    )
                    .padding([d.space.xs, d.space.md])
                    .style(design::flat_button),
                    Space::new().width(Length::Fill),
                    button(text(clear_label).size(d.typography.meta))
                        .on_press_maybe(clear)
                        .padding([d.space.xs, d.space.md])
                        .style(design::danger_outline),
                ]
                .spacing(d.space.sm)
                .align_y(alignment::Vertical::Center),
                setting_row(
                    self.t("日志级别", "Log level").to_string(),
                    setting_control(
                        pick_list(
                            Some(self.draft.settings.log_level.as_str()),
                            ["error", "warning", "info", "debug"],
                            move |level| match (*level, english) {
                                ("error", false) => "错误",
                                ("warning", false) => "警告",
                                ("info", false) => "信息",
                                ("debug", false) => "调试",
                                ("error", true) => "Error",
                                ("warning", true) => "Warning",
                                ("info", true) => "Information",
                                ("debug", true) => "Debug",
                                (label, _) => label,
                            }
                            .to_owned(),
                        )
                        .on_select(|level| Message::LogLevelChanged(level.to_owned()))
                        .width(Length::Fill)
                        .into(),
                    ),
                ),
                setting_number(
                    NumericSetting::LogRetention,
                    &self.draft.log_retention,
                    self.t("日志保留（1–30 天）", "Log retention (1–30 days)"),
                ),
                setting_number(
                    NumericSetting::HistoryRetention,
                    &self.draft.history_retention,
                    self.t("历史保留（1–365 天）", "History retention (1–365 days)"),
                ),
            ]
            .spacing(7),
        )
        .padding(10)
        .into()
    }

    fn update_settings(&self) -> Element<'_, Message> {
        let engine = self.snapshot.as_ref();
        let available = self.settings_actions.update_release.as_deref();
        let mut details = column![
            pair(self.t("桌面客户端", "Desktop client"), APP_VERSION),
            pair(
                self.t("当前后台引擎", "Connected engine"),
                engine.map_or("—", |s| s.app_version.as_str()),
            ),
            pair(
                self.t("接口版本", "API version"),
                engine.map_or_else(|| "—".into(), |s| s.api_version.to_string()),
            ),
            checkbox(self.draft.settings.check_updates)
                .label(self.t("允许检查新版本", "Allow update checks"))
                .on_toggle(|value| Message::FlagChanged(SettingFlag::CheckUpdates, value)),
            checkbox(self.draft.settings.auto_update)
                .label(self.t(
                    "自动安装无需提权的个人版更新",
                    "Install personal updates without elevation"
                ))
                .on_toggle(|value| Message::FlagChanged(SettingFlag::AutoUpdate, value)),
        ]
        .spacing(7);
        if let Some(version) = available {
            details = details.push(pair(self.t("可用版本", "Available version"), version));
        }
        let (label, action) = match available {
            Some(_) => (
                self.t("安装更新", "Install update"),
                Message::SettingsAction(Action::OpenInstaller),
            ),
            None => (
                self.t("检查更新", "Check for updates"),
                Message::CheckUpdates,
            ),
        };
        let checking = self.settings_actions.checking_update();
        column![
            card(details).padding(10),
            container(
                button(if checking {
                    self.t("正在检查…", "Checking…")
                } else {
                    label
                })
                .on_press_maybe(
                    (!self.busy && !self.connecting && !preview_only() && !checking)
                        .then_some(action),
                )
                .style(action_style),
            )
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Right),
        ]
        .spacing(8)
        .into()
    }

    fn transfer_settings(&self) -> Element<'_, Message> {
        let export_enabled =
            !self.busy && self.snapshot_current && !self.settings_actions.export_pending;
        card(
            column![
                text(self.t(
                    "导出不含密码。导入将覆盖当前设置。",
                    "Exports contain no passwords. Imports replace the current settings.",
                ))
                .size(12),
                row![
                    button(self.t("复制导出", "Copy export"))
                        .on_press_maybe(export_enabled.then_some(Message::SettingsAction(
                            Action::Export(ExportTo::Clipboard),
                        )))
                        .style(action_style),
                    button(self.t("导出到文件…", "Export to file…"))
                        .on_press_maybe(
                            export_enabled.then_some(Message::SettingsAction(Action::Export(
                                ExportTo::File
                            ),))
                        )
                        .style(action_style),
                ]
                .spacing(8)
                .wrap(),
                text_editor(&self.import_content)
                    .placeholder(self.t("在此粘贴设置 JSON", "Paste settings JSON"))
                    .height(Length::Fill)
                    .on_action(Message::ImportAction),
                button(self.t("导入并保存设置", "Import and save settings"))
                    .on_press_maybe(
                        (!self.busy && self.snapshot_current).then_some(Message::ImportSettings)
                    )
                    .style(action_style),
            ]
            .height(Length::Fill)
            .spacing(7),
        )
        .height(Length::Fill)
        .padding(10)
        .into()
    }

    pub(super) fn settings_extra_dialog(&self) -> Option<Element<'_, Message>> {
        let d = design::get();
        if let Some(targets) = &self.settings_actions.ipv6_editor {
            let mut content = column![text(self.t(
                "网址可填写带方括号的 IPv6 地址，直接连接检测站点。",
                "Use an IPv6 address in brackets to reach a probe site without DNS."
            ))
            .size(d.typography.meta)
            .style(design::text_style(design::muted))]
            .spacing(d.space.sm);
            for (index, value) in targets.iter().enumerate() {
                content = content.push(
                    row![
                        text_input("http://…", value)
                            .on_input(move |value| Message::SettingsAction(
                                Action::Ipv6TargetChanged(index, value)
                            ))
                            .width(Length::Fill),
                        button(self.t("移除", "Remove"))
                            .on_press(Message::SettingsAction(Action::RemoveIpv6Target(index)))
                            .style(design::flat_button),
                    ]
                    .spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center),
                );
            }
            content = content.push(
                row![
                    button(self.t("添加地址", "Add address"))
                        .on_press_maybe(
                            (targets.len() < bitnet_core::MAX_IPV6_TARGETS)
                                .then_some(Message::SettingsAction(Action::AddIpv6Target))
                        )
                        .style(design::flat_button),
                    Space::new().width(Length::Fill),
                    button(self.t("保存", "Save"))
                        .on_press(Message::SettingsAction(Action::SaveIpv6Targets))
                        .style(design::solid),
                ]
                .spacing(d.space.sm),
            );
            return Some(self.dialog(
                self.t("公网 IPv6 检测地址", "Public IPv6 check URLs"),
                content.into(),
                Message::SettingsAction(Action::CloseIpv6Editor),
                660,
            ));
        }
        let install = self.settings_actions.update_install.as_ref()?;
        let (detail, verified) = match &install.result {
            None => (
                self.t(
                    "正在检查新版本和安装包签名…",
                    "Checking the release and installer signature…",
                )
                .to_owned(),
                false,
            ),
            Some(Ok(Some(update))) => (
                format!(
                    "{} {}",
                    self.t("可下载安装的版本", "Verified update"),
                    update.version
                ),
                true,
            ),
            Some(Ok(None)) => (
                self.t(
                    "当前没有新版可安装",
                    "No newer update is available to install",
                )
                .to_owned(),
                false,
            ),
            Some(Err(error)) => (
                format!(
                    "{} · {}",
                    self.t("无法确认安装包", "Could not verify the installer"),
                    error.message
                ),
                false,
            ),
        };
        let content = column![
            text(detail).size(d.typography.body),
            container(
                button(self.t("下载并安装", "Download and install"))
                    .style(|theme, status| {
                        let mut style = design::solid(theme, status);
                        if status != button::Status::Disabled {
                            style.background = Some(design::success(theme).into());
                            style.text_color = iced::Color::BLACK;
                        }
                        style
                    })
                    .on_press_maybe(
                        (verified && !self.busy && !self.connecting)
                            .then_some(Message::SettingsAction(Action::InstallVerifiedUpdate))
                    )
            )
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Right),
        ]
        .spacing(d.card.gap);
        Some(self.dialog(
            self.t("安装更新", "Install update"),
            content.into(),
            Message::SettingsAction(Action::CloseInstaller),
            560,
        ))
    }
}

const SETTING_LABEL_WIDTH: f32 = 220.0;
const SETTING_CONTROL_WIDTH: f32 = 148.0;

fn setting_control(control: Element<'_, Message>) -> Element<'_, Message> {
    container(container(control).width(Length::Fill.max(SETTING_CONTROL_WIDTH)))
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Right)
        .into()
}

/// Two-column settings grid: every row shares the same label / control columns
/// so inputs, pickers and buttons line up.
fn setting_row<'a>(label: String, control: Element<'a, Message>) -> Element<'a, Message> {
    let d = design::get();
    row![
        text(label)
            .size(d.typography.body)
            .width(Length::Fixed(SETTING_LABEL_WIDTH)),
        container(control).width(Length::Fill),
    ]
    .spacing(d.space.md)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Indented child rows used when a value is not edited in place.
fn sub_item<'a>(value: &'a str) -> Element<'a, Message> {
    let d = design::get();
    row![
        Space::new().width(Length::Fixed(SETTING_LABEL_WIDTH + d.space.md)),
        text(value)
            .size(d.typography.meta)
            .font(iced::Font::MONOSPACE)
            .style(design::text_style(design::muted)),
    ]
    .into()
}

fn setting_hint(label: &str) -> Element<'_, Message> {
    text(label)
        .size(design::get().typography.meta)
        .style(design::text_style(design::muted))
        .into()
}

fn setting_number<'a>(
    field: NumericSetting,
    value: &'a str,
    label: &'static str,
) -> Element<'a, Message> {
    setting_row(
        label.to_string(),
        setting_control(
            text_input("", value)
                .on_input(move |value| Message::NumberChanged(field, value))
                .align_x(alignment::Horizontal::Center)
                .padding(5)
                .width(Length::Fill)
                .into(),
        ),
    )
}
