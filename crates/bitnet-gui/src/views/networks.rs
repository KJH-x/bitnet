use super::*;
use bitnet_core::{NetworkInterface, NetworkRule};
use bitnet_runtime::engine::fingerprint_for;
use iced::widget::{column, row, tooltip};
use std::time::Duration;

struct AdapterRow<'a> {
    interface: &'a NetworkInterface,
    ssid: Option<&'a str>,
    connected: bool,
    historical: bool,
    pinned: bool,
    active: bool,
    permission: Option<bool>,
}

fn adapter_tier(interface: &NetworkInterface) -> u8 {
    let mut addresses = interface.ipv4.iter().chain(interface.gateway.iter());
    if addresses.clone().any(|ip| ip.octets()[0] == 10) {
        0
    } else if addresses.any(|ip| ip.octets()[..2] == [192, 168]) {
        1
    } else {
        2
    }
}

fn observed_permission(
    snapshot: &Snapshot,
    interface: &NetworkInterface,
    active: bool,
) -> Option<bool> {
    let binding = if active {
        snapshot.active_binding.clone()
    } else {
        interface.binding()
    }?;
    let fingerprint = fingerprint_for(&snapshot.settings.gateway, interface, &binding)?;
    snapshot
        .settings
        .network_permissions
        .iter()
        .find(|record| record.fingerprint == fingerprint)
        .map(|record| record.allowed)
        .or_else(|| {
            snapshot
                .settings
                .trusted_networks
                .contains(&fingerprint)
                .then_some(true)
        })
}

fn adapter_rows<'a>(
    snapshot: &'a Snapshot,
    interfaces: &[&'a NetworkInterface],
) -> Vec<AdapterRow<'a>> {
    let settings = &snapshot.settings;
    let mut rows = Vec::new();
    for &interface in interfaces {
        let pinned_adapter = settings.interface_id.as_deref() == Some(interface.id.as_str());
        let pinned_ssid = settings
            .interface_ssid
            .as_deref()
            .filter(|_| pinned_adapter);
        let observed_ssid = interface.ssid.as_deref().filter(|_| interface.wireless);
        let active = interface.up
            && snapshot.active_binding.as_ref().is_some_and(|binding| {
                binding.interface_id == interface.id && binding.interface_index == interface.index
            });
        let historical = |ssid| {
            settings.network_permissions.iter().rev().find(|record| {
                record.interface_id == interface.id && record.ssid.as_deref() == ssid
            })
        };
        rows.push(AdapterRow {
            interface,
            ssid: observed_ssid,
            connected: interface.up,
            historical: false,
            pinned: pinned_adapter && pinned_ssid.is_none_or(|ssid| Some(ssid) == observed_ssid),
            active,
            permission: if interface.up {
                observed_permission(snapshot, interface, active)
            } else {
                historical(observed_ssid).map(|record| record.allowed)
            },
        });
        if !interface.wireless {
            continue;
        }
        let mut listed = std::collections::HashSet::new();
        listed.extend(observed_ssid);
        let remembered = settings
            .network_permissions
            .iter()
            .rev()
            .filter(|record| record.interface_id == interface.id)
            .filter_map(|record| record.ssid.as_deref());
        for ssid in pinned_ssid.into_iter().chain(remembered) {
            if listed.insert(ssid) {
                rows.push(AdapterRow {
                    interface,
                    ssid: Some(ssid),
                    connected: false,
                    historical: true,
                    pinned: pinned_ssid == Some(ssid),
                    active: false,
                    permission: historical(Some(ssid)).map(|record| record.allowed),
                });
            }
        }
    }
    rows.sort_by_key(|row| {
        (
            !row.pinned,
            !row.active,
            is_alternate_adapter(row.interface),
            !row.connected,
            adapter_tier(row.interface),
            row.interface.index,
            row.historical,
        )
    });
    rows
}

fn fitted_text<'a>(value: impl ToString, size: f32) -> Element<'a, Message> {
    container(
        text(value.to_string())
            .size(size)
            .width(Length::Fill)
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End),
    )
    .width(Length::Fill)
    .clip(true)
    .into()
}

fn section_label(section: NetworkSection, english: bool) -> &'static str {
    match (section, english) {
        (NetworkSection::Adapters, false) => "网络",
        (NetworkSection::Adapters, true) => "Network",
        (NetworkSection::Profiles, false) => "认证环境",
        (NetworkSection::Profiles, true) => "Profiles",
        (NetworkSection::Rules, false) => "网络规则",
        (NetworkSection::Rules, true) => "Rules",
        (NetworkSection::RuleEditor, false) => "规则编辑",
        (NetworkSection::RuleEditor, true) => "Edit rule",
    }
}

fn choice_mark<'a>(selected: bool, enabled: bool) -> Element<'a, Message> {
    let ring = if selected {
        design::accent
    } else if enabled {
        design::muted
    } else {
        design::line
    };
    container(
        Space::new()
            .width(Length::Fixed(11.0))
            .height(Length::Fixed(11.0)),
    )
    .padding(0)
    .style(move |theme| iced::widget::container::Style {
        border: iced::Border {
            color: ring(theme),
            width: 1.0,
            radius: 6.0.into(),
        },
        background: if selected {
            Some(iced::Background::Color(design::accent(theme)))
        } else {
            None
        },
        ..iced::widget::container::Style::default()
    })
    .into()
}

fn section_row<'a>(label: &'a str) -> Element<'a, Message> {
    let d = design::get();
    container(
        text(label)
            .size(d.typography.meta)
            .style(design::text_style(design::muted)),
    )
    .width(Length::Fill)
    .padding([d.space.md, d.space.md])
    .style(design::tint(design::muted, 0.06))
    .into()
}

fn column_label<'a>(label: &'a str, width: f32) -> Element<'a, Message> {
    let d = design::get();
    let cell = text(label)
        .size(d.typography.meta)
        .style(design::text_style(design::muted));
    if width > 0.0 {
        container(cell)
            .width(Length::Fixed(width))
            .align_x(alignment::Horizontal::Center)
            .into()
    } else {
        container(cell).width(Length::Fill).into()
    }
}

impl App {
    pub(super) fn networks_workspace(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let english = self.english();
        let section = match self.network_section {
            NetworkSection::RuleEditor => NetworkSection::Rules,
            other => other,
        };
        let picker = pick_list(
            Some(section),
            [
                NetworkSection::Adapters,
                NetworkSection::Profiles,
                NetworkSection::Rules,
            ],
            move |value: &NetworkSection| section_label(*value, english).to_string(),
        )
        .on_select(Message::NetworkTab)
        .width(Length::Fixed(140.0));
        let header = row![
            container(
                text(
                    if self.enabling_automatic && section == NetworkSection::Adapters {
                        self.t(
                            "选择用于自动登录的网卡，然后继续。",
                            "Select the adapter for automatic sign-in, then continue.",
                        )
                    } else {
                        self.section_hint(section)
                    }
                )
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
            )
            .padding(iced::padding::left(d.space.md))
            .width(Length::Fill),
            text(self.t("高级", "Advanced"))
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
            picker,
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center);

        let body = match self.network_section {
            NetworkSection::Adapters => self.adapter_page(size),
            NetworkSection::Profiles => self.profile_page(size),
            NetworkSection::Rules => self.rule_page(size),
            NetworkSection::RuleEditor => self.rule_editor(),
        };
        column![header, Self::rule(false), body]
            .spacing(d.content.gap)
            .into()
    }

    fn section_hint(&self, section: NetworkSection) -> &'static str {
        match section {
            NetworkSection::Adapters => self.t(
                "选择校园网使用的网卡。",
                "Choose the adapter used for the campus network.",
            ),
            NetworkSection::Profiles => self.t(
                "保存已应用的网关、网卡、账户与规则。",
                "Capture the applied gateway, adapter, account and rules.",
            ),
            _ => self.t(
                "优先级高的规则先匹配。",
                "Higher-priority rules match first.",
            ),
        }
    }

    fn network_columns(&self, width: f32) -> (f32, f32, f32) {
        if self.english() && width < 700.0 {
            (44.0, 60.0, 86.0)
        } else if self.english() {
            (52.0, 72.0, 98.0)
        } else {
            (44.0, 44.0, 64.0)
        }
    }

    fn trust_column_width(&self) -> f32 {
        if self.english() {
            76.0
        } else {
            56.0
        }
    }

    fn adapter_page(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let Some(snapshot) = &self.snapshot else {
            return card(text(self.t("尚未连接后台", "Engine disconnected"))).into();
        };
        let saved = snapshot.settings.interface_id.as_deref();
        let active = snapshot
            .active_binding
            .as_ref()
            .map(|binding| binding.interface_id.as_str());
        let mut interfaces: Vec<_> = snapshot.interfaces.iter().collect();
        let alternate_count = interfaces
            .iter()
            .filter(|candidate| is_alternate_adapter(candidate))
            .filter(|candidate| {
                Some(candidate.id.as_str()) != saved && Some(candidate.id.as_str()) != active
            })
            .count();
        if !self.alternate_adapters {
            interfaces.retain(|candidate| {
                !is_alternate_adapter(candidate)
                    || Some(candidate.id.as_str()) == saved
                    || Some(candidate.id.as_str()) == active
            });
        }
        let widths = self.network_columns(size.width);
        let (use_width, type_width, state_width) = widths;
        let header = row![
            column_label(self.t("使用", "Use"), use_width),
            column_label(self.t("类型", "Type"), type_width),
            column_label(self.t("状态", "State"), state_width),
            column_label(self.t("网卡 · 网络", "Adapter · network"), 0.0),
            column_label("DNS", d.networks.dns_width),
            column_label(self.t("信任", "Trust"), self.trust_column_width()),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center)
        .padding([0.0, d.space.md]);

        let networks = adapter_rows(snapshot, &interfaces);
        let total = networks.len();
        let grouped = d.networks.group_by_kind;
        let row_height = d.networks.row_pad_y * 2.0 + 22.0;
        let name_width = size.width
            - 2.0 * (d.card.pad + d.space.md)
            - 5.0 * d.space.sm
            - use_width
            - type_width
            - state_width
            - d.networks.dns_width
            - self.trust_column_width();
        let maximum_section_heads = 4.0;
        let section_head_height = d.space.md * 2.0 + d.typography.meta + 4.0;
        let reserved = 198.0
            + if grouped {
                maximum_section_heads * section_head_height
            } else {
                0.0
            };
        let capacity = page_capacity(size.height, reserved, row_height, 12);
        let range = page_window(total, self.network_offset, capacity);
        let mut rows = column![].spacing(0.0);
        let mut previous_group = None;
        for network in &networks[range.clone()] {
            let alternate = is_alternate_adapter(network.interface);
            if grouped && previous_group != Some(alternate) {
                previous_group = Some(alternate);
                rows = rows.push(section_row(if alternate {
                    self.t("备选网卡", "Other adapters")
                } else {
                    self.t("常用网卡", "Primary adapters")
                }));
            }
            rows = rows.push(self.network_row(network, widths, name_width));
        }
        if networks.is_empty() {
            rows = rows.push(
                container(
                    text(self.t("没有可显示的网卡", "No adapters to show")).size(d.typography.body),
                )
                .padding(d.space.lg),
            );
        }

        let editable = self.snapshot_current && !self.busy && !self.draft.dirty;
        let mut toolbar = row![Space::new().width(Length::Fill)]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center);
        toolbar = toolbar.push(super::ips_eye_button(self.ips_revealed));
        if alternate_count > 0 {
            let label = if self.alternate_adapters {
                self.t("隐藏备选网卡", "Hide other adapters")
            } else {
                self.t("显示备选网卡", "Show other adapters")
            };
            toolbar = toolbar.push(
                button(text(format!("{label} ({alternate_count})")).size(d.typography.meta))
                    .on_press(Message::ToggleAlternateAdapters)
                    .padding([2.0, 8.0])
                    .style(design::flat_button),
            );
        }
        if self.enabling_automatic {
            let ready = editable
                && selected_binding(snapshot).is_some()
                && snapshot.environment != Environment::NonCampus
                && !bitnet_runtime::engine::uses_system_authentication(snapshot);
            toolbar = toolbar
                .push(
                    button(text(self.t("取消", "Cancel")).size(d.typography.meta))
                        .on_press(Message::NetworkBack)
                        .style(design::flat_button),
                )
                .push(
                    button(text(self.t("继续", "Continue")).size(d.typography.meta))
                        .on_press_maybe(ready.then_some(Message::BeginAutomatic)),
                );
        }

        let mut content = column![toolbar, header, Self::rule(false), rows].spacing(d.card.gap);
        if total > capacity {
            content =
                content.push(self.page_controls(total, range, capacity, Message::NetworkPage));
        }
        card(content).into()
    }

    fn network_row<'a>(
        &'a self,
        network: &AdapterRow<'_>,
        widths: (f32, f32, f32),
        name_width: f32,
    ) -> Element<'a, Message> {
        let d = design::get();
        let (use_width, type_width, state_width) = widths;
        let item = network.interface;
        let virtual_adapter = item.virtual_adapter;
        let initial_binding = item.binding();
        let active_binding = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.active_binding.as_ref())
            .filter(|_| network.active);
        let pinned_adapter = self
            .snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.settings.interface_id.is_some());
        let marked = network.pinned || (network.active && !pinned_adapter);
        let selectable = network.connected
            && initial_binding.is_some()
            && !network.pinned
            && self.snapshot_current
            && !self.busy
            && !self.draft.dirty;
        let label = match network.ssid {
            Some(ssid) => format!("{}({ssid})", item.name),
            None => item.name.clone(),
        };
        let ip = network
            .connected
            .then(|| {
                active_binding
                    .or(initial_binding.as_ref())
                    .map(|binding| binding.source_ip)
                    .or_else(|| item.ipv4.first().copied())
            })
            .flatten();
        let dns = item.dns.first().filter(|_| network.connected);
        let ip = ip.map(|ip| ip.to_string());
        let ip_width = ip.as_ref().map_or(0.0, |ip| {
            ip.len() as f32 * (d.typography.meta * 0.7) + 12.0 + d.space.sm
        });
        let name_text = tooltip(
            container(
                text(label.clone())
                    .size(d.typography.body)
                    .wrapping(text::Wrapping::None)
                    .ellipsis(text::Ellipsis::End),
            )
            .width(Length::Shrink.max((name_width - ip_width).max(34.0)))
            .clip(true),
            container(text(label).size(d.typography.meta)).padding([d.space.xs, d.space.md]),
            tooltip::Position::Top,
        )
        .gap(d.space.xs)
        .delay(Duration::from_millis(250))
        .style(design::panel);
        let mut name = row![name_text]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center);
        if let Some(ip) = ip {
            name = name.push(ip_chip(&ip, self.ips_revealed));
        }
        let state = badge_cell(
            if network.connected {
                self.t("已连接", "Connected")
            } else {
                self.t("未连接", "Disconnected")
            },
            if network.connected {
                design::success
            } else {
                design::muted
            },
            state_width,
        );
        let trust = match network.permission {
            Some(true) => badge_cell(
                self.t("已信任", "Trusted"),
                design::success,
                self.trust_column_width(),
            ),
            Some(false) => badge_cell(
                self.t("已忽略", "Ignored"),
                design::muted,
                self.trust_column_width(),
            ),
            None => column_label("—", self.trust_column_width()),
        };
        let cells = row![
            button(container(choice_mark(marked, selectable)).center_x(Length::Fill))
                .on_press_maybe(selectable.then(|| {
                    Message::InterfaceSelected(Some((item.id.clone(), item.ssid.clone())))
                }))
                .padding([3.0, 0.0])
                .width(Length::Fixed(use_width))
                .style(|theme, status| iced::widget::button::Style {
                    border: iced::Border::default(),
                    ..design::flat_button(theme, status)
                }),
            badge_cell(
                if virtual_adapter {
                    self.t("虚拟", "Virtual")
                } else {
                    self.t("物理", "Physical")
                },
                if virtual_adapter {
                    design::muted
                } else {
                    design::physical
                },
                type_width,
            ),
            state,
            container(name)
                .height(Length::Fixed(22.0))
                .width(Length::Fill)
                .align_y(alignment::Vertical::Center)
                .clip(true),
            container(ip_chip(
                &dns.map_or_else(|| "—".to_owned(), ToString::to_string),
                self.ips_revealed,
            ))
            .width(Length::Fixed(d.networks.dns_width))
            .align_x(alignment::Horizontal::Center),
            trust,
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center)
        .padding([d.networks.row_pad_y, d.space.md]);
        let highlight = if network.pinned {
            0.15
        } else if network.active {
            0.08
        } else {
            0.0
        };
        container(cells)
            .width(Length::Fill)
            .style(design::tint(design::accent, highlight))
            .into()
    }

    fn profile_page(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let Some(snapshot) = &self.snapshot else {
            return card(text(self.t("尚未连接后台", "Engine disconnected"))).into();
        };
        let profiles = &snapshot.settings.profiles;
        let capacity = page_capacity(size.height, 190.0, 66.0, 8);
        let range = page_window(profiles.len(), self.network_offset, capacity);
        let enabled = self.snapshot_current && !self.busy && !self.draft.dirty;
        let mut entries = column![].spacing(5);
        for profile in &profiles[range.clone()] {
            let active = snapshot.settings.active_profile.as_deref() == Some(profile.id.as_str());
            let summary = format!(
                "{}{} · {}",
                if active {
                    self.t("当前环境 · ", "Active · ")
                } else {
                    ""
                },
                profile.id,
                profile
                    .interface_id
                    .as_deref()
                    .unwrap_or_else(|| self.t("自动选择", "Automatic")),
            );
            entries = entries.push(
                container(
                    row![
                        column![
                            fitted_text(&profile.name, 16.0),
                            row![
                                fitted_text(summary, 12.0),
                                container(ip_chip(&profile.gateway.base_url, self.ips_revealed))
                                    .width(Length::Fixed(d.networks.dns_width))
                                    .clip(true),
                            ]
                            .spacing(d.space.sm)
                            .align_y(alignment::Vertical::Center),
                        ]
                        .spacing(3)
                        .width(Length::Fill),
                        button(self.t("选择", "Select"))
                            .on_press_maybe(
                                (enabled && !active)
                                    .then(|| Message::ProfileSelect(profile.id.clone()))
                            )
                            .style(action_style),
                        button(trash_icon(14.0))
                            .on_press_maybe(enabled.then(|| Message::Ask(
                                Confirmation::DeleteProfile(profile.id.clone())
                            )))
                            .width(Length::Fixed(28.0))
                            .padding([d.space.xs, 0.0])
                            .style(design::bare_button),
                    ]
                    .spacing(8)
                    .align_y(alignment::Vertical::Center),
                )
                .padding([6, 10])
                .width(Length::Fill)
                .style(design::panel),
            );
        }
        if profiles.is_empty() {
            entries = entries.push(text(self.t("还没有认证环境", "No authentication profiles")));
        }
        let fields = row![
            text_input(self.t("环境 ID", "Profile ID"), &self.profile_id)
                .on_input(Message::ProfileIdChanged),
            text_input(self.t("环境名称", "Profile name"), &self.profile_name)
                .on_input(Message::ProfileNameChanged),
        ]
        .spacing(8);
        let note = if self.draft.dirty {
            self.t(
                "请先保存或放弃未保存的设置。",
                "Save or discard your pending settings first.",
            )
        } else {
            self.t(
                "将当前已应用的网关、网卡、账户和规则保存为环境。",
                "Capture the applied gateway, adapter, account, and rules.",
            )
        };
        column![
            entries,
            self.page_controls(profiles.len(), range, capacity, Message::NetworkPage),
            card(
                column![
                    text(note).size(12),
                    fields,
                    button(self.t("保存环境", "Capture applied settings")).on_press_maybe(
                        (enabled
                            && !self.profile_id.trim().is_empty()
                            && !self.profile_name.trim().is_empty())
                        .then_some(Message::ProfileSave)
                    ),
                ]
                .spacing(7)
            ),
        ]
        .spacing(9)
        .into()
    }

    fn rule_page(&self, size: Size) -> Element<'_, Message> {
        let rules = &self.draft.settings.rules;
        let capacity = page_capacity(size.height, 153.0, 62.0, 10);
        let range = page_window(rules.len(), self.network_offset, capacity);
        let enabled = self.snapshot_current && !self.busy;
        let mut entries = column![].spacing(5);
        for i in range.clone() {
            entries = entries.push(self.rule_entry(&rules[i], i, enabled));
        }
        if rules.is_empty() {
            entries = entries.push(text(self.t("还没有网络规则", "No network rules")));
        }
        column![
            row![
                button(self.t("添加规则", "Add rule"))
                    .on_press_maybe(enabled.then_some(Message::BeginRule)),
                text(self.t(
                    "优先级高的规则先匹配。",
                    "Higher-priority rules match first."
                ))
                .size(12),
            ]
            .spacing(9)
            .align_y(alignment::Vertical::Center),
            entries,
            self.page_controls(rules.len(), range, capacity, Message::NetworkPage),
        ]
        .spacing(9)
        .into()
    }

    fn rule_entry<'a>(
        &'a self,
        item: &'a NetworkRule,
        index: usize,
        enabled: bool,
    ) -> Element<'a, Message> {
        let d = design::get();
        let action = if item.ignore {
            self.t("忽略", "Ignore")
        } else {
            self.t("认证", "Authenticate")
        };
        let detail = format!(
            "{} · SSID {} · {} {}",
            item.priority,
            item.ssid.as_deref().unwrap_or("*"),
            self.t("账户", "Account"),
            item.account_id
                .as_deref()
                .unwrap_or_else(|| self.t("当前", "Selected")),
        );
        container(
            row![
                column![
                    fitted_text(format!("{} · {action}", item.name), 16.0),
                    row![
                        fitted_text(detail, 12.0),
                        text(self.t("网关", "Gateway"))
                            .size(d.typography.meta)
                            .style(design::text_style(design::muted)),
                        ip_chip(
                            &item.gateway.map_or_else(|| "—".into(), |ip| ip.to_string()),
                            self.ips_revealed,
                        ),
                    ]
                    .spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center),
                ]
                .spacing(3)
                .width(Length::Fill),
                button(self.t("编辑", "Edit"))
                    .on_press_maybe(enabled.then_some(Message::RuleEdit(index)))
                    .style(action_style),
                button(trash_icon(14.0))
                    .on_press_maybe(
                        enabled.then(|| Message::Ask(Confirmation::DeleteRule(item.id.clone())))
                    )
                    .width(Length::Fixed(28.0))
                    .padding([d.space.xs, 0.0])
                    .style(design::bare_button),
            ]
            .align_y(alignment::Vertical::Center)
            .spacing(8),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(design::panel)
        .into()
    }

    fn rule_editor(&self) -> Element<'_, Message> {
        let editing = self.rule.index.is_some();
        let enabled = self.snapshot_current && !self.busy;
        let account_options: Vec<String> =
            self.snapshot.as_ref().map_or_else(Vec::new, |snapshot| {
                snapshot
                    .accounts
                    .iter()
                    .map(|account| account.id.clone())
                    .collect()
            });
        let account_label = |id: &String| {
            self.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.accounts.iter().find(|account| &account.id == id))
                .map_or_else(|| id.clone(), super::account_label)
        };
        card(
            column![
                text(if editing {
                    self.t("编辑规则", "Edit rule")
                } else {
                    self.t("添加规则", "Add rule")
                })
                .size(18),
                row![
                    text_input(self.t("规则名称", "Rule name"), &self.rule.name)
                        .on_input(Message::RuleNameChanged),
                    text_input(self.t("优先级", "Priority"), &self.rule.priority)
                        .on_input(Message::RulePriorityChanged)
                        .width(125),
                ]
                .spacing(8),
                row![
                    text_input(
                        self.t("网卡 ID（可空）", "Adapter ID (optional)"),
                        &self.rule.interface_id
                    )
                    .on_input(Message::RuleInterfaceChanged),
                    text_input("SSID", &self.rule.ssid).on_input(Message::RuleSsidChanged),
                ]
                .spacing(8),
                row![
                    text_input(
                        self.t("网关 IPv4（可空）", "Gateway IPv4 (optional)"),
                        &self.rule.gateway
                    )
                    .on_input(Message::RuleGatewayChanged),
                    text_input(
                        self.t("账户 ID（可空）", "Account ID (optional)"),
                        &self.rule.account_id
                    )
                    .on_input(Message::RuleAccountChanged),
                ]
                .spacing(8),
                row![
                    checkbox(self.rule.ignore)
                        .label(self.t("忽略此网络", "Ignore this network"))
                        .on_toggle(Message::RuleIgnoreChanged),
                    pick_list(
                        account_options
                            .iter()
                            .find(|id| *id == &self.rule.account_id)
                            .cloned(),
                        account_options,
                        account_label,
                    )
                    .placeholder(self.t("选择已保存账户", "Choose saved account"))
                    .on_select(Message::RuleAccountChanged),
                ]
                .spacing(8)
                .align_y(alignment::Vertical::Center)
                .wrap(),
                row![
                    button(self.t("保存规则", "Save rule"))
                        .on_press_maybe(enabled.then_some(Message::RuleSave)),
                    button(self.t("取消", "Cancel"))
                        .on_press(Message::RuleCancel)
                        .style(action_style),
                ]
                .spacing(8),
            ]
            .spacing(7),
        )
        .into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitnet_core::{NetworkPermission, Scope};

    #[test]
    fn remembered_ssid_keeps_its_pin_and_permission_without_trusting_the_live_network() {
        let mut snapshot = Snapshot::new(Scope::User);
        let wlan: NetworkInterface = serde_json::from_value(serde_json::json!({
            "id": "wifi",
            "name": "WLAN",
            "index": 15,
            "ipv4": ["10.1.3.4"],
            "gateway": "10.1.3.1",
            "up": true,
            "wireless": true,
            "ssid": "Current",
            "virtual_adapter": false
        }))
        .unwrap();
        let mut remembered = wlan.clone();
        remembered.ssid = Some("Remembered".into());
        let fingerprint = fingerprint_for(
            &snapshot.settings.gateway,
            &remembered,
            &remembered.binding().unwrap(),
        )
        .unwrap();
        snapshot.settings.interface_id = Some(wlan.id.clone());
        snapshot.settings.interface_ssid = remembered.ssid.clone();
        snapshot
            .settings
            .network_permissions
            .push(NetworkPermission {
                fingerprint,
                interface_id: remembered.id,
                adapter_name: remembered.name,
                ssid: remembered.ssid,
                allowed: true,
            });
        let rows = adapter_rows(&snapshot, &[&wlan]);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].ssid, Some("Remembered"));
        assert_eq!(rows[0].permission, Some(true));
        assert!(rows[0].pinned);
        assert!(!rows[0].connected);
        assert_eq!(rows[1].ssid, Some("Current"));
        assert_eq!(rows[1].permission, None);
        assert!(!rows[1].pinned);
        assert!(rows[1].connected);
    }
}
