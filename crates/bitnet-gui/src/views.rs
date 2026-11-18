mod networks;
mod settings;

use super::*;
use bitnet_core::{Health, Ipv6Status};
use iced::{
    advanced::{layout, mouse, renderer, widget::Tree, Layout, Widget},
    alignment,
    widget::{
        button, checkbox, column, container, pick_list, responsive, row, stack, text, text_input,
        Space,
    },
    Color, Length, Rectangle, Size,
};

pub(super) fn action_style(theme: &Theme, status: button::Status) -> button::Style {
    design::flat_button(theme, status)
}

fn field_row<'a>(label: &'a str, control: Element<'a, Message>) -> Element<'a, Message> {
    let d = design::get();
    row![
        text(label)
            .size(d.typography.body)
            .width(Length::Fixed(84.0))
            .style(design::text_style(design::muted)),
        control,
    ]
    .spacing(d.space.md)
    .align_y(alignment::Vertical::Center)
    .into()
}

struct EyeIcon {
    open: bool,
}

impl<Message> iced::widget::canvas::Program<Message> for EyeIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path, Stroke};

        let mut frame = Frame::new(renderer, bounds.size());
        let scale = (bounds.width.min(bounds.height) / 16.0).max(0.5);
        let pt = |x: f32, y: f32| iced::Point::new(x * scale, y * scale);
        let tone = if self.open {
            design::accent(theme)
        } else {
            design::muted(theme)
        };
        let stroke = Stroke::default().with_color(tone).with_width(1.25 * scale);

        let lens = Path::new(|p| {
            p.move_to(pt(1.5, 8.0));
            p.quadratic_curve_to(pt(8.0, 2.0), pt(14.5, 8.0));
            p.quadratic_curve_to(pt(8.0, 14.0), pt(1.5, 8.0));
        });
        frame.stroke(&lens, stroke);
        frame.fill(&Path::circle(pt(8.0, 8.0), 2.1 * scale), tone);

        if !self.open {
            let slash = Path::new(|p| {
                p.move_to(pt(2.0, 2.5));
                p.line_to(pt(14.0, 13.5));
            });
            frame.stroke(&slash, stroke);
        }

        vec![frame.into_geometry()]
    }
}

/// Square icon button: the icon is centred on both axes regardless of padding.
fn icon_button<'a>(
    icon: Element<'a, Message>,
    message: Message,
    width: f32,
) -> iced::widget::button::Button<'a, Message> {
    button(
        container(icon)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(message)
    .width(Length::Fixed(width))
    .height(Length::Fixed(26.0))
    .padding([0.0, 0.0])
}

fn eye_button<'a>(open: bool) -> Element<'a, Message> {
    icon_button(
        iced::widget::canvas(EyeIcon { open })
            .width(Length::Fixed(15.0))
            .height(Length::Fixed(15.0))
            .into(),
        Message::TogglePassword,
        30.0,
    )
    .style(design::flat_button)
    .into()
}

fn ips_eye_button<'a>(open: bool) -> Element<'a, Message> {
    icon_button(
        iced::widget::canvas(EyeIcon { open })
            .width(Length::Fixed(15.0))
            .height(Length::Fixed(15.0))
            .into(),
        Message::ToggleIps,
        30.0,
    )
    .style(design::flat_button)
    .into()
}

struct TrashIcon;

impl<Message> iced::widget::canvas::Program<Message> for TrashIcon {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &iced::Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<iced::widget::canvas::Geometry> {
        use iced::widget::canvas::{Frame, Path, Stroke};

        let mut frame = Frame::new(renderer, bounds.size());
        let scale = (bounds.width.min(bounds.height) / 16.0).max(0.5);
        let tone = design::danger(theme);
        let stroke = Stroke::default().with_color(tone).with_width(1.25 * scale);
        let pt = |x: f32, y: f32| iced::Point::new(x * scale, y * scale);

        let path = Path::new(|p| {
            p.move_to(pt(2.6, 4.4));
            p.line_to(pt(13.4, 4.4));

            p.move_to(pt(6.2, 4.4));
            p.line_to(pt(6.2, 2.2));
            p.line_to(pt(9.8, 2.2));
            p.line_to(pt(9.8, 4.4));

            p.move_to(pt(4.2, 6.0));
            p.line_to(pt(5.0, 14.0));
            p.line_to(pt(11.0, 14.0));
            p.line_to(pt(11.8, 6.0));

            p.move_to(pt(6.9, 7.6));
            p.line_to(pt(6.9, 12.4));
            p.move_to(pt(9.1, 7.6));
            p.line_to(pt(9.1, 12.4));
        });
        frame.stroke(&path, stroke);

        vec![frame.into_geometry()]
    }
}

fn trash_icon<'a>(size: f32) -> Element<'a, Message> {
    iced::widget::canvas(TrashIcon)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .into()
}

fn card<'a>(content: impl Into<Element<'a, Message>>) -> iced::widget::Container<'a, Message> {
    container(content)
        .padding(design::get().card.pad)
        .width(Length::Fill)
        .style(design::panel)
}
pub(super) fn badge<'a>(label: &'a str, role: fn(&Theme) -> Color) -> Element<'a, Message> {
    let d = design::get();
    container(
        text(label)
            .size(d.typography.meta)
            .wrapping(text::Wrapping::None),
    )
    .padding([1.0, 6.0])
    .style(design::badge(role))
    .into()
}

/// Badge stretched to a fixed width so labels of different length keep the same
/// box size.
pub(super) fn badge_cell<'a>(
    label: &'a str,
    role: fn(&Theme) -> Color,
    width: f32,
) -> Element<'a, Message> {
    let d = design::get();
    container(
        text(label)
            .size(d.typography.meta)
            .wrapping(text::Wrapping::None)
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
    )
    .width(Length::Fixed(width))
    .padding([1.0, 0.0])
    .style(design::badge(role))
    .into()
}

pub(super) fn auth_role(state: &AuthState) -> fn(&Theme) -> Color {
    match state {
        AuthState::Authenticated => design::success,
        AuthState::Unauthenticated => design::muted,
        AuthState::Unknown => design::physical,
    }
}

fn ipv6_label(state: Ipv6Status, english: bool) -> &'static str {
    match (state, english) {
        (Ipv6Status::Unknown, false) => "待确认",
        (Ipv6Status::Unknown, true) => "Unknown",
        (Ipv6Status::Disabled, false) => "未启用",
        (Ipv6Status::Disabled, true) => "Disabled",
        (Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable, false) => "无互联网",
        (Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable, true) => "No internet",
        (Ipv6Status::GlobalNotPreferred, false) => "非首选",
        (Ipv6Status::GlobalNotPreferred, true) => "Not preferred",
        (Ipv6Status::Normal, false) => "正常",
        (Ipv6Status::Normal, true) => "Normal",
    }
}

fn ipv6_role(state: Ipv6Status) -> fn(&Theme) -> Color {
    match state {
        Ipv6Status::Unknown | Ipv6Status::Disabled => design::muted,
        Ipv6Status::LinkLocalOnly | Ipv6Status::GlobalUnreachable => design::danger,
        Ipv6Status::GlobalNotPreferred => design::physical,
        Ipv6Status::Normal => design::success,
    }
}

fn health_badge_label(state: &Health, english: bool) -> &'static str {
    match (state, english) {
        (Health::Healthy, false) => "正常",
        (Health::Healthy, true) => "OK",
        (Health::Degraded, false) => "部分异常",
        (Health::Degraded, true) => "Degraded",
        (Health::Unavailable, false) => "不可用",
        (Health::Unavailable, true) => "Unavailable",
        (Health::Unknown, false) => "未知",
        (Health::Unknown, true) => "Unknown",
    }
}

fn health_badge_role(state: &Health) -> fn(&Theme) -> Color {
    match state {
        Health::Healthy => design::success,
        Health::Degraded => design::physical,
        Health::Unavailable => design::danger,
        Health::Unknown => design::muted,
    }
}

/// Centred label for fixed-width buttons. Symmetric padding does the vertical
/// centring, exactly like the header refresh button.
fn button_label<'a>(label: &'a str, size: f32) -> Element<'a, Message> {
    text(label)
        .size(size)
        .wrapping(text::Wrapping::None)
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .into()
}

fn action_button<'a>(label: &'a str, width: f32) -> iced::widget::button::Button<'a, Message> {
    let d = design::get();
    button(button_label(label, d.typography.body))
        .width(Length::Fixed(width))
        .padding([d.space.xs + 2.0, d.space.md])
}

fn labelled_row<'a>(
    label: &'a str,
    value: Element<'a, Message>,
    label_width: f32,
) -> Element<'a, Message> {
    let d = design::get();
    row![
        text(label)
            .size(d.typography.meta)
            .style(design::text_style(design::muted))
            .width(Length::Fixed(label_width)),
        value,
    ]
    .spacing(d.space.sm)
    .align_y(alignment::Vertical::Center)
    .into()
}

/// Monospaced chip for IP addresses and other code-like values. Masking keeps
/// the character count so the layout does not jump when toggled.
pub(super) fn ip_chip<'a>(value: &str, revealed: bool) -> Element<'a, Message> {
    let d = design::get();
    let shown = if revealed || value == "—" {
        value.to_string()
    } else {
        "•".repeat(value.chars().count())
    };
    container(
        text(shown)
            .size(d.typography.meta)
            .font(Font::MONOSPACE)
            .style(design::text_style(design::code_text)),
    )
    .padding([1.0, 6.0])
    .style(design::code)
    .into()
}

fn pair<'a>(label: &'a str, value: impl ToString) -> iced::widget::Row<'a, Message> {
    let d = design::get();
    row![
        text(label)
            .size(d.typography.body)
            .width(Length::Fixed(150.0))
            .style(design::text_style(design::muted)),
        text(value.to_string())
            .size(d.typography.body)
            .width(Length::Fill)
    ]
    .spacing(d.space.md)
}

// Preserve the underlying page and its widget state while a modal owns input.
struct Inactive<'a> {
    content: Element<'a, Message>,
    inactive: bool,
}

impl Widget<Message, Theme, iced::Renderer> for Inactive<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
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
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        shell: &mut iced::advanced::Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if !self.inactive {
            self.content.as_widget_mut().update(
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
        layout: Layout<'_>,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
    ) {
        if !self.inactive {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                layout,
                viewport,
                renderer,
                operation,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        if self.inactive {
            mouse::Interaction::None
        } else {
            self.content.as_widget().mouse_interaction(
                &tree.children[0],
                layout,
                cursor,
                viewport,
                renderer,
            )
        }
    }

    fn overlay<'a>(
        &'a mut self,
        tree: &'a mut Tree,
        layout: Layout<'a>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: iced::Vector,
        window: Size,
    ) -> Vec<iced::advanced::overlay::Element<'a, Message, Theme, iced::Renderer>> {
        if self.inactive {
            Vec::new()
        } else {
            self.content.as_widget_mut().overlay(
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
    Element::new(Inactive { content, inactive })
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

fn page_capacity(height: f32, reserved: f32, row_height: f32, maximum: usize) -> usize {
    (((height - reserved) / row_height).floor() as usize).clamp(1, maximum)
}

fn page_window(total: usize, offset: usize, capacity: usize) -> std::ops::Range<usize> {
    let start = (offset.min(total.saturating_sub(1)) / capacity) * capacity;
    start..(start + capacity).min(total)
}

impl App {
    fn page_controls(
        &self,
        total: usize,
        shown: std::ops::Range<usize>,
        capacity: usize,
        navigate: impl Fn(usize) -> Message,
    ) -> Element<'_, Message> {
        row![
            button(self.t("上一页", "Previous"))
                .on_press_maybe(
                    (shown.start > 0).then(|| navigate(shown.start.saturating_sub(capacity)))
                )
                .style(action_style),
            text(format!(
                "{} / {}",
                shown.start / capacity + 1,
                total.div_ceil(capacity).max(1)
            ))
            .size(12),
            button(self.t("下一页", "Next"))
                .on_press_maybe((shown.end < total).then(|| navigate(shown.end)))
                .style(action_style),
        ]
        .spacing(6)
        .align_y(alignment::Vertical::Center)
        .into()
    }

    fn relative_time(&self, timestamp: Option<u64>) -> String {
        timestamp.map_or_else(
            || "—".into(),
            |at| format_elapsed_minutes(bitnet_core::now_secs().saturating_sub(at), self.english()),
        )
    }

    fn network_prompt_dialog(&self, prompt: &NetworkPrompt) -> Element<'_, Message> {
        let d = design::get();
        let decision = |allow| Message::NetworkDecision {
            fingerprint: prompt.fingerprint.clone(),
            allow,
        };
        let body = column![
            text(self.t(
                "请确认这是校园网。信任后，BitNet 会在此网络用已保存的账户登录。",
                "Make sure this is your campus network. Once trusted, BitNet can sign in here with a saved account.",
            ))
            .size(d.typography.body)
            .style(design::text_style(design::muted)),
            text(prompt.label.clone()).size(d.typography.section_title),
            self.network_facts_line(),
            row![
                button(
                    text(self.t("信任并登录", "Trust and sign in")).size(d.typography.body)
                )
                .on_press_maybe((!self.busy).then(|| decision(Some(true))))
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::solid),
                button(
                    text(self.t("不再询问", "Never ask for this network"))
                        .size(d.typography.body)
                )
                .on_press_maybe((!self.busy).then(|| decision(Some(false))))
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::danger_outline),
                button(text(self.t("本次忽略", "Ignore once")).size(d.typography.body))
                    .on_press(decision(None))
                    .padding([d.space.xs + 2.0, d.space.md])
                    .style(design::flat_button),
            ]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center),
        ]
        .spacing(d.card.gap);
        self.dialog(
            self.t(
                "对此网络采用自动登录吗？",
                "Automatic sign-in for this network?",
            ),
            body.into(),
            decision(None),
            680,
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
            .into()
        } else {
            content
        };
        let confirmation = self.confirmation.as_ref();
        let health_editor = self.health_editor.as_deref();
        let prompt = self.network_prompt.as_ref().filter(|_| {
            self.snapshot_current
                && !self.enabling_automatic
                && health_editor.is_none()
                && !self.account_editing
                && self.quick_login.is_none()
        });
        let mut layers = stack([layer(
            content,
            self.quick_login.is_some()
                || confirmation.is_some()
                || health_editor.is_some()
                || self.account_editing
                || prompt.is_some(),
        )]);
        if let Some(entries) = health_editor {
            layers = layers.push(layer(
                self.health_editor_view(entries),
                confirmation.is_some(),
            ));
        }
        if self.account_editing {
            layers = layers.push(layer(self.add_account_dialog(), confirmation.is_some()));
        }
        if let Some(prompt) = prompt {
            layers = layers.push(layer(
                self.network_prompt_dialog(prompt),
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
        layers.into()
    }

    fn dialog<'a>(
        &'a self,
        title: &'a str,
        content: Element<'a, Message>,
        close: Message,
        max_width: u32,
    ) -> Element<'a, Message> {
        let d = design::get();
        let heading = row![
            text(title)
                .size(d.typography.page_title)
                .font(design::family_medium())
                .width(Length::Fill),
            button(text(self.t("关闭", "Close")).size(d.typography.meta))
                .on_press_maybe((!self.busy).then_some(close))
                .style(design::bare_button),
        ]
        .align_y(alignment::Vertical::Center);
        let mut dialog = column![heading].spacing(d.card.gap);
        if let Some(banner) = self.notice_banner() {
            dialog = dialog.push(banner);
        }
        container(
            container(dialog.push(content))
                .width(Length::Fill.max(max_width))
                .padding(d.card.pad)
                .style(design::elevated),
        )
        .padding(d.space.xl)
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .style(scrim)
        .into()
    }

    fn notice_banner(&self) -> Option<Element<'_, Message>> {
        let (is_error, message) = self.notice.as_ref()?;
        let banner = row![
            text(message).width(Length::Fill),
            button(self.t("关闭", "Dismiss"))
                .on_press(Message::DismissNotice)
                .style(button::text)
        ]
        .spacing(12)
        .align_y(alignment::Vertical::Center);
        Some(
            container(banner)
                .padding(12)
                .width(Length::Fill)
                .style(if *is_error {
                    container::danger
                } else {
                    container::success
                })
                .into(),
        )
    }

    fn area_page(&self) -> Page {
        match self.page {
            Page::Health | Page::Statistics | Page::Diagnostics => Page::Status,
            other => other,
        }
    }

    fn nav_selected(&self, page: Page) -> bool {
        page == self.page
            || (page == Page::Status
                && matches!(
                    self.page,
                    Page::Health | Page::Statistics | Page::Diagnostics
                ))
    }

    fn scope_label(&self) -> &'static str {
        if self.connecting {
            return self.t("正在连接后台", "Connecting to engine");
        }
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
    }

    fn hide_button(&self) -> Element<'_, Message> {
        button(self.t("隐藏界面", "Hide window"))
            .on_press_maybe(
                self.tray
                    .as_ref()
                    .filter(|tray| tray.is_available())
                    .and(self.window)
                    .map(Message::WindowCloseRequested),
            )
            .padding(2)
            .style(design::nav_button(false))
            .into()
    }

    fn nav_items(&self, d: &design::Design, inline: bool) -> Vec<Element<'_, Message>> {
        let mut items: Vec<Element<'_, Message>> = Vec::new();
        for page in Page::ALL {
            let selected = self.nav_selected(page);
            let label = container(
                text(page.label(self.english()))
                    .size(d.nav.label_size)
                    .width(Length::Fill)
                    .align_x(alignment::Horizontal::Center),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(alignment::Vertical::Center);
            let mut item = button(label)
                .on_press(Message::Navigate(page))
                .style(design::nav_button(selected));
            item = if inline {
                item.padding([d.nav.item_pad_y, d.nav.item_pad_x])
            } else {
                item.width(Length::Fill)
                    .height(Length::Fixed(d.nav.item_height))
                    .padding([0.0, d.space.xs])
            };
            if !inline && d.nav.selected == "bar" {
                let bar = container(
                    Space::new()
                        .width(Length::Fixed(d.nav.bar_width))
                        .height(Length::Fixed(d.nav.item_height)),
                )
                .style(if selected {
                    design::accent_bar
                } else {
                    design::bare
                });
                items.push(
                    container(
                        row![
                            bar,
                            item,
                            Space::new().width(Length::Fixed(d.nav.bar_width))
                        ]
                        .spacing(0)
                        .height(Length::Fixed(d.nav.item_height)),
                    )
                    .width(Length::Fill)
                    .style(if selected { design::band } else { design::bare })
                    .into(),
                );
            } else {
                items.push(item.into());
            }
        }
        items
    }

    fn sidebar(&self, d: &design::Design) -> Element<'_, Message> {
        let header_band = 30.0;
        let brand = container(
            text("BitNet")
                .size(d.nav.brand_size)
                .font(design::family_medium())
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
        )
        .width(Length::Fill)
        .height(Length::Fixed(header_band))
        .align_y(alignment::Vertical::Center);
        let mut items = column![].spacing(d.nav.item_gap).width(Length::Fill);
        for item in self.nav_items(d, false) {
            items = items.push(item);
        }
        let mut rail = column![brand, Self::rule(false)]
            .spacing(d.space.sm)
            .width(Length::Fill);
        rail = rail
            .push(Space::new().height(Length::Fixed(d.space.md)))
            .push(items);
        if d.nav.valign == "center" {
            rail = column![
                Space::new().height(Length::Fill),
                rail,
                Space::new().height(Length::Fill)
            ]
            .width(Length::Fill);
        }
        let mut footer = column![
            text(self.scope_label())
                .size(d.typography.meta)
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
            text(format!("v{APP_VERSION}"))
                .size((d.typography.meta - 1.0).max(9.0))
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
        ]
        .spacing(d.space.xs)
        .width(Length::Fill);
        if self.tray.as_ref().is_some_and(tray::Tray::is_available) {
            footer = footer.push(container(self.hide_button()).center_x(Length::Fill));
        }
        let rail = rail.push(Space::new().height(Length::Fill)).push(footer);
        container(rail)
            .width(Length::Fixed(d.nav.width))
            .height(Length::Fill)
            .padding([d.content.pad, d.space.sm])
            .style(design::chrome)
            .into()
    }

    fn topbar(&self, d: &design::Design) -> Element<'_, Message> {
        let mut bar = row![
            text("BitNet").size(d.nav.brand_size),
            Space::new().width(Length::Fixed(d.space.md))
        ]
        .spacing(d.space.xs)
        .align_y(alignment::Vertical::Center);
        for item in self.nav_items(d, true) {
            bar = bar.push(item);
        }
        bar = bar
            .push(Space::new().width(Length::Fill))
            .push(text(self.scope_label()).size(d.typography.meta))
            .push(self.hide_button());
        container(bar)
            .height(Length::Fixed(d.header.height))
            .width(Length::Fill)
            .padding([0.0, d.space.lg])
            .style(design::chrome)
            .into()
    }

    fn rule<'a>(vertical: bool) -> Element<'a, Message> {
        let space = if vertical {
            Space::new().width(Length::Fixed(1.0)).height(Length::Fill)
        } else {
            Space::new().width(Length::Fill).height(Length::Fixed(1.0))
        };
        container(space).style(design::hairline).into()
    }

    fn page_header(&self, d: &design::Design) -> Element<'_, Message> {
        let english = self.english();
        let title = container(
            text(self.area_page().label(self.english()))
                .size(d.header.title_size)
                .font(design::family_medium()),
        )
        .height(Length::Fixed(30.0))
        .align_y(alignment::Vertical::Center);
        let mut header = row![title]
            .spacing(d.space.md)
            .align_y(alignment::Vertical::Center)
            .push(Space::new().width(Length::Fill));
        if d.header.mode_picker {
            let picker = pick_list(
                Some(self.mode),
                [Mode::User, Mode::System],
                move |value: &Mode| value.label(english).to_string(),
            )
            .on_select(Message::ModeChanged)
            .text_size(d.typography.body)
            .padding([d.space.xs + 1.0, d.space.sm])
            .width(Length::Fixed(d.header.picker_width));
            header = header.push(picker);
        }
        if d.header.refresh {
            header = header.push(
                button(text(self.t("刷新", "Refresh")).size(d.typography.body))
                    .on_press_maybe(
                        (!self.busy && !self.connecting && !self.polling)
                            .then_some(Message::Refresh),
                    )
                    .padding([d.space.xs + 1.0, d.space.md])
                    .style(design::flat_button),
            );
        }
        let mut result = column![header].spacing(d.space.sm);
        if d.header.border {
            result = result.push(Self::rule(false));
        }
        result.into()
    }

    fn workspace_body(&self, d: &design::Design) -> Element<'_, Message> {
        let mut content = column![self.page_header(d)].spacing(d.content.gap);
        if let Some(error) = &self.connection_error {
            content = content.push(
                container(
                    row![
                        container(text(&error.message).size(d.typography.body))
                            .width(Length::Fill)
                            .height(Length::Fixed(34.0))
                            .clip(true),
                        button(self.t("后台服务", "Background service"))
                            .on_press(Message::Navigate(Page::Service))
                            .style(button::text),
                    ]
                    .spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center),
                )
                .padding(d.space.md)
                .width(Length::Fill)
                .style(container::danger),
            );
        }
        if self.connection_error.is_none()
            && self.health_editor.is_none()
            && self.quick_login.is_none()
        {
            if let Some(banner) = self.notice_banner() {
                content = content.push(banner);
            }
        }
        if self.connection_error.is_none()
            && self.snapshot.is_some()
            && !self.snapshot_current
            && !self.connecting
        {
            content = content.push(
                text(self.t("显示断开前的数据", "Showing data from before disconnect"))
                    .size(d.typography.meta),
            );
        }
        if self.busy || self.connecting {
            content = content.push(
                text(if self.connecting {
                    self.t("正在连接本机引擎…", "Connecting to the local engine…")
                } else {
                    self.t("正在处理，请稍候…", "Working, please wait…")
                })
                .size(d.typography.meta),
            );
        }
        content = content.push(match self.page {
            Page::Status | Page::Health | Page::Statistics | Page::Diagnostics => {
                responsive(|size| self.connection_workspace(size)).into()
            }
            Page::Accounts => responsive(|size| self.accounts_view(size)).into(),
            Page::Networks => responsive(|size| self.networks_workspace(size)).into(),
            Page::Settings => responsive(|size| self.settings_workspace(size)).into(),
            Page::Service => self.service_view(),
        });
        let workspace = container(content)
            .padding(d.content.pad)
            .width(Length::Fill)
            .height(Length::Fill);
        if d.content.max_width > 0.0 {
            container(workspace)
                .width(Length::Fixed(d.content.max_width))
                .center_x(Length::Fill)
                .height(Length::Fill)
                .into()
        } else {
            workspace.into()
        }
    }

    fn main_shell(&self) -> Element<'_, Message> {
        let d = design::get();
        let body = self.workspace_body(d);
        if d.nav.mode == "topbar" {
            column![self.topbar(d), Self::rule(false), body]
                .height(Length::Fill)
                .into()
        } else {
            row![self.sidebar(d), Self::rule(true), body]
                .height(Length::Fill)
                .into()
        }
    }

    fn connection_workspace(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let mut navigation = row![].spacing(d.space.xs);
        for page in [
            Page::Status,
            Page::Health,
            Page::Statistics,
            Page::Diagnostics,
        ] {
            let selected = page == self.page;
            navigation = navigation.push(
                button(text(page.tab_label(self.english())).size(d.typography.body))
                    .on_press_maybe((!selected).then_some(Message::Navigate(page)))
                    .padding([d.space.xs, d.space.sm])
                    .style(design::nav_button(selected)),
            );
        }
        let body = match self.page {
            Page::Health => self.connection_details(),
            Page::Statistics => self.statistics_view(),
            Page::Diagnostics => self.diagnostics_view(),
            _ => self.status_view(size),
        };
        column![navigation.wrap(), body].spacing(12).into()
    }

    fn status_view(&self, size: Size) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return card(
                column![
                    text(self.t("请先连接后台", "Connect to the engine first")).size(21),
                    button(self.t("连接 / 重试", "Connect / retry"))
                        .on_press_maybe((!self.connecting).then_some(Message::Refresh)),
                ]
                .spacing(10),
            )
            .into();
        };
        let connected = self.snapshot_current && !self.busy;
        let system_auth = bitnet_runtime::engine::uses_system_authentication(snapshot);
        let portal_controls = connected && !system_auth;
        let offline = snapshot.session.auth != AuthState::Authenticated;
        let auth = match snapshot.session.auth {
            AuthState::Authenticated => self.t("已认证", "Authenticated"),
            AuthState::Unauthenticated => self.t("尚未认证", "Not authenticated"),
            AuthState::Unknown => self.t("状态待确认", "Authentication unknown"),
        };
        let d = design::get();
        let english = self.english();
        let label_width = if english { 116.0 } else { 100.0 };
        let health_label_width = if english { 52.0 } else { 56.0 };
        let badge_width = if english { 108.0 } else { 84.0 };
        let revealed = self.ips_revealed;

        let authenticated_ip = snapshot
            .session
            .ip
            .map_or_else(|| "—".to_string(), |ip| ip.to_string());
        let local_ip = snapshot
            .active_binding
            .as_ref()
            .map(|binding| binding.source_ip.to_string())
            .unwrap_or_else(|| "—".into());

        let left = column![
            labelled_row(
                self.t("当前校园账号", "Campus account"),
                text(
                    snapshot
                        .session
                        .username
                        .clone()
                        .unwrap_or_else(|| "—".into())
                )
                .size(d.typography.body)
                .width(Length::Fill)
                .into(),
                label_width,
            ),
            labelled_row(
                self.t("认证 IP", "Authenticated IP"),
                ip_chip(&authenticated_ip, revealed),
                label_width,
            ),
            labelled_row(
                self.t("本机 IP", "Local IP"),
                ip_chip(&local_ip, revealed),
                label_width,
            ),
        ]
        .spacing(d.card.gap)
        .width(Length::Fixed(280.0));

        let ipv6 = snapshot.health.ipv6_status;
        let adapter = ipv6_adapter(snapshot);
        let ipv6_address = adapter
            .filter(|_| matches!(ipv6, Ipv6Status::GlobalNotPreferred | Ipv6Status::Normal))
            .and_then(|adapter| {
                adapter
                    .ipv6
                    .iter()
                    .copied()
                    .find(|ip| bitnet_core::is_public_ipv6(*ip))
            })
            .map(|ip| ip.to_string());

        let right = column![
            labelled_row(
                self.t("互联网", "Internet"),
                badge_cell(
                    health_badge_label(&snapshot.health.internet, english),
                    health_badge_role(&snapshot.health.internet),
                    badge_width,
                ),
                health_label_width,
            ),
            labelled_row(
                "DNS",
                badge_cell(
                    health_badge_label(&snapshot.health.dns, english),
                    health_badge_role(&snapshot.health.dns),
                    badge_width,
                ),
                health_label_width,
            ),
        ]
        .spacing(d.card.gap)
        .width(Length::Fill);

        let mut summary = column![
            row![
                text(self.state_label(&snapshot.state))
                    .font(design::family_medium())
                    .size(if size.height < 390.0 {
                        d.typography.page_title
                    } else {
                        d.typography.page_title * 1.35
                    }),
                badge(auth, auth_role(&snapshot.session.auth)),
                Space::new().width(Length::Fill),
                ips_eye_button(revealed),
            ]
            .spacing(d.space.md)
            .align_y(alignment::Vertical::Center),
            row![left, right].spacing(d.space.xl),
        ]
        .spacing(d.card.gap);
        let ipv6_caption = if ipv6 == Ipv6Status::Unknown && snapshot.health.checked_at.is_some() {
            self.t("未能确认", "Inconclusive")
        } else {
            ipv6_label(ipv6, english)
        };
        let status_badge = badge_cell(ipv6_caption, ipv6_role(ipv6), badge_width);
        let status_badge = if let Some(detail) = &snapshot.health.ipv6_detail {
            iced::widget::tooltip(
                status_badge,
                container(text(detail).size(d.typography.meta)).padding(d.space.sm),
                iced::widget::tooltip::Position::Top,
            )
            .style(design::panel)
            .into()
        } else {
            status_badge
        };
        let ipv6_status = row![
            text("IPv6")
                .size(d.typography.meta)
                .style(design::text_style(design::muted))
                .width(Length::Fixed(health_label_width)),
            status_badge,
            container(ip_chip(ipv6_address.as_deref().unwrap_or("—"), revealed))
                .width(Length::Fixed(280.0)),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center);
        let mut ipv6_row = row![ipv6_status, Space::new().width(Length::Fill)]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center);

        if system_auth {
            summary = summary.push(
                text(self.t(
                    "BIT-mobile 由系统完成认证",
                    "Your system handles BIT-mobile sign-in",
                ))
                .size(d.typography.meta),
            );
        } else if snapshot.settings.automatic
            && !snapshot.settings.paused
            && !snapshot.verification_hold
            && !snapshot.network_trusted
            && snapshot.active_binding.is_some()
            && snapshot.environment != Environment::NonCampus
            && snapshot.session.auth != AuthState::Unknown
            && !bitnet_runtime::engine::network_permission_denied(snapshot)
            && snapshot
                .capabilities
                .iter()
                .any(|capability| capability == "network_trust")
        {
            summary = summary.push(
                text(self.t(
                    "自动认证需要先信任当前网络",
                    "Automatic authentication needs a trusted network",
                ))
                .size(d.typography.meta)
                .style(design::text_style(design::physical)),
            );
        }

        let width = if english {
            136.0
        } else {
            d.actions.button_width
        };
        let mut actions = row![]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center);
        if offline {
            actions = actions.push(
                action_button(self.t("登录", "Sign in"), width)
                    .on_press_maybe(portal_controls.then_some(Message::OpenQuickLogin))
                    .style(design::solid),
            );
            if !snapshot.accounts.is_empty() {
                actions = actions.push(
                    action_button(self.t("使用已保存账户", "Use saved account"), width)
                        .on_press_maybe(portal_controls.then_some(Self::login_message(snapshot)))
                        .style(design::flat_button),
                );
            }
        } else {
            actions = actions.push(
                action_button(self.t("注销", "Disconnect"), width)
                    .on_press_maybe(portal_controls.then_some(Message::Ask(Confirmation::Logout)))
                    .style(design::danger_outline),
            );
        }
        actions = actions.push(
            action_button(self.t("立即检测", "Check now"), width)
                .on_press_maybe(portal_controls.then_some(Message::Run(Command::Check)))
                .style(design::flat_button),
        );
        if !offline {
            actions = actions.push(
                action_button(self.t("重新认证…", "Reauthenticate…"), width)
                    .on_press_maybe(
                        (portal_controls && !snapshot.accounts.is_empty())
                            .then_some(Message::Ask(Confirmation::ForceLogin)),
                    )
                    .style(design::flat_button),
            );
        }
        let paused = snapshot.settings.paused || snapshot.verification_hold;
        actions = actions
            .push(
                container(
                    Space::new()
                        .width(Length::Fixed(1.0))
                        .height(Length::Fixed(16.0)),
                )
                .style(design::hairline),
            )
            .push(
                action_button(
                    if paused {
                        self.t("恢复自动认证", "Resume automatic")
                    } else {
                        self.t("暂停自动认证", "Pause automatic")
                    },
                    width,
                )
                .on_press_maybe(connected.then_some(Message::Run(if paused {
                    Command::Resume
                } else {
                    Command::Pause
                })))
                .style(design::nav_button(false)),
            );

        if let Some((interface, enabled)) = ipv6_maintenance_target(snapshot)
            .filter(|_| !preview_only())
            .zip(adapter.and_then(|adapter| adapter.ipv6_enabled))
        {
            let protocol_width = 124.0;
            let (caption, toggle) = if enabled {
                (
                    self.t("关闭 IPv6", "Disable IPv6"),
                    MaintenanceAction::DisableIpv6 {
                        interface: interface.clone(),
                    },
                )
            } else {
                (
                    self.t("启用 IPv6", "Enable IPv6"),
                    MaintenanceAction::EnableIpv6 {
                        interface: interface.clone(),
                    },
                )
            };
            let reset = MaintenanceAction::ResetIpv6 { interface };
            ipv6_row = ipv6_row.push(
                row![
                    action_button(caption, protocol_width)
                        .on_press_maybe(
                            connected.then_some(Message::Ask(Confirmation::Maintenance(toggle))),
                        )
                        .style(design::flat_button),
                    action_button(self.t("重新启用", "Re-enable"), protocol_width)
                        .on_press_maybe(
                            (connected && enabled)
                                .then_some(Message::Ask(Confirmation::Maintenance(reset))),
                        )
                        .style(design::flat_button),
                ]
                .spacing(d.space.sm)
                .align_y(alignment::Vertical::Center),
            );
        }

        card(
            summary
                .push(Self::rule(false))
                .push(ipv6_row.wrap())
                .push(Self::rule(false))
                .push(actions.wrap()),
        )
        .into()
    }

    fn connection_details(&self) -> Element<'_, Message> {
        let Some(snapshot) = &self.snapshot else {
            return card(text(self.t("尚未连接后台", "Engine disconnected"))).into();
        };
        let mut details = column![
            pair(
                self.t("网关统计流量", "Gateway usage"),
                snapshot
                    .session
                    .bytes_used
                    .map(format_bytes)
                    .unwrap_or_else(|| "—".into())
            ),
            pair(
                self.t("网关累计在线时长", "Gateway online total"),
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
                "网关统计周期由校方定义。",
                "The campus gateway defines the accounting period."
            ))
            .size(12),
            pair(
                self.t("最近成功认证", "Last successful login"),
                self.relative_time(snapshot.last_success_at)
            ),
            pair(self.t("重试次数", "Retry attempts"), snapshot.retry_count),
            row![
                button(self.t("打开诊断", "Open diagnostics"))
                    .on_press(Message::Navigate(Page::Diagnostics))
                    .style(action_style),
                button(self.t("账户管理", "Manage accounts"))
                    .on_press(Message::Navigate(Page::Accounts))
                    .style(action_style),
            ]
            .spacing(8)
            .wrap(),
        ]
        .spacing(9);
        if let Some(error) = &snapshot.last_error {
            details = details.push(
                container(text(&error.message).size(12))
                    .clip(true)
                    .height(42)
                    .width(Length::Fill),
            );
        }
        card(details).into()
    }

    fn quick_login_view<'a>(&'a self, draft: &'a AccountDraft) -> Element<'a, Message> {
        let editable = !self.busy;
        let can_submit = editable
            && self.snapshot_current
            && !draft.username.trim().is_empty()
            && !draft.password.is_empty();
        let verify = can_submit
            .then(|| self.snapshot.as_ref().and_then(confirmation_target))
            .flatten()
            .map(|(fingerprint, _)| Message::AccountSave(fingerprint));
        column![
            text_input(self.t("校园账号", "Campus username"), &draft.username)
                .id("quick-login-username")
                .on_input_maybe(editable.then_some(Message::UsernameChanged))
                .on_submit_maybe(editable.then_some(Message::MoveFocus(false)))
                .padding(10),
            row![
                text_input(
                    self.t("校园网密码", "Campus network password"),
                    &draft.password
                )
                .secure(!self.password_visible())
                .on_input_maybe(editable.then_some(Message::PasswordChanged))
                .on_submit_maybe(can_submit.then_some(Message::TemporaryLogin))
                .padding(10)
                .width(Length::Fill),
                eye_button(self.password_visible()),
            ]
            .spacing(design::get().space.xs)
            .align_y(alignment::Vertical::Center),
            text(self.t(
                "验证并保存会先退出当前登录。账号或密码有误可能导致远程桌面断开。",
                "Verify and save signs out of the current account first. An incorrect username or password could disconnect Remote Desktop.",
            )).size(12),
            row![
                action_button(self.t("单次登录", "Sign in once"), 128.0)
                    .on_press_maybe(can_submit.then_some(Message::TemporaryLogin))
                    .style(design::flat_button),
                action_button(self.t("验证并保存", "Verify and save"), 128.0)
                    .on_press_maybe(verify)
                    .style(design::solid),
            ]
            .spacing(10)
            .wrap(),
        ]
        .spacing(13)
        .into()
    }

    fn accounts_view(&self, size: Size) -> Element<'_, Message> {
        let d = design::get();
        let accounts = self
            .snapshot
            .as_ref()
            .map_or(&[][..], |s| s.accounts.as_slice());
        let selected = self
            .snapshot
            .as_ref()
            .and_then(|s| s.settings.selected_account.as_deref());
        let available = !self.busy && self.snapshot_current;

        let mut rows = column![].spacing(0.0);
        if accounts.is_empty() {
            rows = rows.push(
                container(
                    text(self.t("尚未保存账户", "No saved accounts"))
                        .size(d.typography.body)
                        .style(design::text_style(design::muted)),
                )
                .padding([d.space.lg, 0.0]),
            );
        }
        let capacity = page_capacity(size.height, 160.0, 44.0, 12);
        let range = page_window(accounts.len(), self.account_offset, capacity);
        for item in &accounts[range.clone()] {
            let is_current = selected == Some(item.id.as_str());
            let target = self
                .snapshot
                .as_ref()
                .filter(|_| available)
                .and_then(|snapshot| SavedAccountTarget::capture(snapshot, &item.id));
            let is_online = self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.session.auth == AuthState::Authenticated
                    && snapshot.session.username.as_deref() == Some(item.username.as_str())
            });
            let cell_width = 96.0;
            let state: Element<'_, Message> = if is_current && (is_online || target.is_none()) {
                container(
                    text(self.t("当前账号", "Current"))
                        .size(d.typography.meta)
                        .style(design::text_style(design::success))
                        .width(Length::Fill)
                        .align_x(alignment::Horizontal::Center),
                )
                .width(Length::Fixed(cell_width))
                .padding([3.0, 0.0])
                .style(design::tint(design::success, 0.14))
                .into()
            } else {
                let label = if is_current {
                    self.t("登录此账户", "Sign in")
                } else {
                    self.t("设为当前", "Set current")
                };
                button(button_label(label, d.typography.meta))
                    .on_press_maybe(target.map(Message::SwitchAccount))
                    .width(Length::Fixed(cell_width))
                    .padding([3.0, 0.0])
                    .style(design::flat_button)
                    .into()
            };

            rows = rows.push(
                row![
                    container(
                        text(&item.username)
                            .size(d.typography.body)
                            .wrapping(text::Wrapping::None)
                    )
                    .width(Length::Fill)
                    .clip(true),
                    state,
                    icon_button(
                        trash_icon(14.0),
                        Message::Ask(Confirmation::DeleteAccount(item.id.clone())),
                        30.0,
                    )
                    .style(design::bare_button),
                ]
                .spacing(d.space.sm)
                .align_y(alignment::Vertical::Center)
                .padding([d.space.sm, d.space.md]),
            );
            rows = rows.push(Self::rule(false));
        }

        let heading = row![
            text(self.t("已保存账户", "Saved accounts"))
                .size(d.typography.section_title)
                .font(design::family_medium())
                .width(Length::Fill),
            button(text(self.t("添加一个账号", "Add account")).size(d.typography.body))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current).then_some(Message::AccountAdd)
                )
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::solid),
        ]
        .spacing(d.space.md)
        .align_y(alignment::Vertical::Center);

        let mut content = column![heading, Self::rule(false), rows].spacing(d.card.gap);
        if accounts.len() > capacity {
            content = content.push(self.page_controls(
                accounts.len(),
                range,
                capacity,
                Message::AccountPage,
            ));
        }
        card(content).into()
    }

    fn add_account_dialog(&self) -> Element<'_, Message> {
        let d = design::get();
        let mut adapters = Vec::new();
        let mut current = None;
        if let Some(snapshot) = &self.snapshot {
            adapters.extend(snapshot.interfaces.iter().filter(|interface| {
                interface.binding().is_some() && !is_alternate_adapter(interface)
            }));
            current = selected_binding(snapshot).and_then(|binding| {
                adapters
                    .iter()
                    .copied()
                    .find(|interface| interface.id == binding.interface_id)
            });
            if current.is_none() {
                current = selected_binding(snapshot).and_then(|binding| {
                    snapshot
                        .interfaces
                        .iter()
                        .find(|interface| interface.id == binding.interface_id)
                });
                if let Some(interface) = current {
                    adapters.push(interface);
                }
            }
        }
        let label = |interface: &&bitnet_core::NetworkInterface| match &interface.ssid {
            Some(ssid) => format!("{} ({ssid})", interface.name),
            None => interface.name.clone(),
        };
        let ready = !self.busy
            && self.snapshot_current
            && current.is_some()
            && !self.account.username.trim().is_empty()
            && !self.account.password.is_empty();
        let verify = ready
            .then(|| self.snapshot.as_ref().and_then(confirmation_target))
            .flatten()
            .map(|(fingerprint, _)| Message::AccountSave(fingerprint));
        let verifying = self.credential_request == Some(CredentialRequest::VerifyAccount);
        let mut body = column![
            field_row(
                self.t("校园账号", "Campus account"),
                text_input(self.t("学号或账号", "Student ID or username"), &self.account.username)
                    .id("add-account-username")
                    .on_input_maybe((!verifying).then_some(Message::UsernameChanged))
                    .on_submit_maybe((!verifying).then_some(Message::MoveFocus(false)))
                    .padding([d.space.sm, d.space.md])
                    .into(),
            ),
            field_row(
                self.t("密码", "Password"),
                row![
                    text_input(self.t("校园网密码", "Campus password"), &self.account.password)
                        .secure(!self.password_visible())
                        .on_input_maybe((!verifying).then_some(Message::PasswordChanged))
                        .padding([d.space.sm, d.space.md])
                        .width(Length::Fill),
                    eye_button(self.password_visible()),
                ]
                .spacing(d.space.xs)
                .align_y(alignment::Vertical::Center)
                .into(),
            ),
            field_row(
                self.t("认证网络", "Network"),
                pick_list(current, adapters, label)
                    .on_select(move |interface| Message::InterfaceSelected(Some((
                        interface.id.clone(),
                        interface.ssid.clone(),
                    ))))
                    .width(Length::Fill)
                    .into(),
            ),
            Self::rule(false),
            column![
                text(self.t(
                    "我们会先退出当前登录，再用新账号试一次。登录成功后才会保存并选为当前账号。",
                    "BitNet will sign out of the current account and try the new one. The new account is saved and selected only if sign-in succeeds.",
                ))
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
                text(self.t(
                    "网络会短暂中断。若账号或密码有误，远程桌面可能断开。",
                    "Your connection will briefly drop. An incorrect username or password could disconnect Remote Desktop.",
                ))
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
            ]
            .spacing(2),
        ]
        .spacing(d.card.gap);

        if current.is_some() {
            body = body.push(self.network_facts_line());
        }
        body = body.push(
            row![
                Space::new().width(Length::Fill),
                button(text(self.t("取消", "Cancel")).size(d.typography.body))
                    .on_press_maybe((!verifying).then_some(Message::AccountCancel))
                    .padding([d.space.xs + 2.0, d.space.md])
                    .style(design::flat_button),
                button(
                    text(if verifying {
                        self.t("正在验证登录…", "Verifying sign-in…")
                    } else {
                        self.t("确认并验证", "Verify and save")
                    })
                    .size(d.typography.body)
                )
                .on_press_maybe(verify)
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::solid),
            ]
            .spacing(d.space.sm),
        );

        self.dialog(
            self.t("添加校园账户", "Add campus account"),
            body.into(),
            Message::AccountCancel,
            560,
        )
    }

    fn health_editor_view<'a>(&'a self, entries: &'a [HealthTarget]) -> Element<'a, Message> {
        let d = design::get();
        let conditions = [
            HealthCondition::AnySuccess,
            HealthCondition::Status,
            HealthCondition::BodyEquals,
            HealthCondition::BodyContains,
        ];
        let mut content = column![text(self.t(
            "设置每个地址的成功条件，最多添加 8 个地址。",
            "Set a success condition for each URL. You can add up to 8 URLs.",
        ))
        .size(d.typography.meta)
        .style(design::text_style(design::muted)),]
        .spacing(d.space.sm);

        for (index, entry) in entries.iter().enumerate() {
            let placeholder = match entry.condition {
                HealthCondition::AnySuccess => None,
                HealthCondition::Status => Some(self.t("例如 204", "e.g. 204")),
                HealthCondition::BodyEquals | HealthCondition::BodyContains => {
                    Some(self.t("输入文字", "Expected text"))
                }
            };
            let value: Element<'_, Message> = match placeholder {
                Some(placeholder) => text_input(placeholder, &entry.value)
                    .on_input(move |value| Message::HealthEntryValueChanged(index, value))
                    .size(d.typography.meta)
                    .padding([d.space.xs, d.space.sm])
                    .width(158)
                    .into(),
                None => Space::new().width(158).into(),
            };
            content = content.push(
                row![
                    text_input("https://…", &entry.url)
                        .on_input(move |value| Message::HealthEntryChanged(index, value))
                        .size(d.typography.meta)
                        .padding([d.space.xs, d.space.sm])
                        .width(Length::Fill),
                    pick_list(
                        Some(entry.condition),
                        conditions,
                        move |condition: &HealthCondition| {
                            match condition {
                                HealthCondition::AnySuccess => self.t("任意 2xx", "Any 2xx"),
                                HealthCondition::Status => {
                                    self.t("指定状态码", "Exact status code")
                                }
                                HealthCondition::BodyEquals => {
                                    self.t("正文完全匹配", "Body equals text")
                                }
                                HealthCondition::BodyContains => {
                                    self.t("正文包含文字", "Body contains text")
                                }
                            }
                            .to_owned()
                        }
                    )
                    .on_select(move |condition| Message::HealthEntryExpectChanged(index, condition))
                    .text_size(d.typography.meta)
                    .padding([d.space.xs, d.space.sm])
                    .width(Length::Fixed(if self.english() { 170.0 } else { 132.0 })),
                    value,
                    icon_button(trash_icon(14.0), Message::HealthEntryRemoved(index), 30.0,)
                        .style(design::bare_button),
                ]
                .spacing(d.space.sm)
                .align_y(alignment::Vertical::Center),
            );
        }

        content = content.push(
            row![
                button(text(self.t("添加地址", "Add URL")).size(d.typography.body))
                    .on_press_maybe(
                        (entries.len() < MAX_HEALTH_TARGETS).then_some(Message::HealthEntryAdded)
                    )
                    .padding([d.space.xs + 2.0, d.space.md])
                    .style(design::flat_button),
                Space::new().width(Length::Fill),
                button(text(self.t("取消", "Cancel")).size(d.typography.body))
                    .on_press(Message::CancelHealthEditor)
                    .padding([d.space.xs + 2.0, d.space.md])
                    .style(design::flat_button),
                button(text(self.t("完成", "Done")).size(d.typography.body))
                    .on_press(Message::SaveHealthEditor)
                    .padding([d.space.xs + 2.0, d.space.md])
                    .style(design::solid),
            ]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center),
        );

        self.dialog(
            self.t("编辑健康探测地址", "Edit health check URLs"),
            content.into(),
            Message::CancelHealthEditor,
            880,
        )
    }

    fn service_view(&self) -> Element<'_, Message> {
        let d = design::get();
        let mut content = column![text(self.t(
            "系统后台可在登录系统前连接校园网。",
            "The system service can connect before you sign in to the computer."
        ))
        .size(14)]
        .spacing(10);
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
                    container(text(service.detail.as_deref().unwrap_or("")).size(12))
                        .height(34)
                        .width(Length::Fill)
                        .clip(true),
                    row![
                        button(text(self.t("重新连接后台", "Reconnect")).size(d.typography.body))
                            .on_press_maybe(
                                (!self.busy && !self.connecting).then_some(Message::Refresh)
                            )
                            .padding([d.space.xs, d.space.md])
                            .style(design::flat_button),
                        button(
                            text(self.t("立即检查连接", "Check connection"))
                                .size(d.typography.body)
                        )
                        .on_press_maybe((!self.busy).then_some(Message::Run(Command::Check)))
                        .padding([d.space.xs, d.space.md])
                        .style(design::flat_button)
                    ]
                    .spacing(10),
                ]
                .spacing(8),
            ));
            let mut actions = row![].spacing(10);
            if service.installed {
                actions = actions
                    .push(
                        button(text(self.t("启动后台", "Start service")).size(d.typography.body))
                            .padding([d.space.xs, d.space.md])
                            .style(design::solid)
                            .on_press_maybe(self.can_start_service().then_some(Message::Ask(
                                Confirmation::Maintenance(MaintenanceAction::ServiceStart),
                            ))),
                    )
                    .push(
                        button(text(self.t("停止后台", "Stop service")).size(d.typography.body))
                            .on_press_maybe((!self.busy && service.running).then_some(
                                Message::Ask(Confirmation::Maintenance(
                                    MaintenanceAction::ServiceStop,
                                )),
                            ))
                            .padding([d.space.xs, d.space.md])
                            .style(design::flat_button),
                    )
                    .push(
                        button(
                            text(self.t("卸载后台，保留数据", "Remove service, keep data"))
                                .size(d.typography.body),
                        )
                        .on_press_maybe((!self.busy).then_some(Message::Ask(
                            Confirmation::Maintenance(MaintenanceAction::ServiceRemove {
                                purge: false,
                            }),
                        )))
                        .padding([d.space.xs, d.space.md])
                        .style(design::danger_text),
                    );
            } else {
                actions = actions.push(
                    button(self.t("启用开机后台", "Enable system service")).on_press_maybe(
                        (!self.busy).then_some(Message::Ask(Confirmation::Maintenance(
                            MaintenanceAction::ServiceInstall,
                        ))),
                    ),
                );
            }
            actions = actions.push(
                button(text(self.t("修复安装", "Repair installation")).size(d.typography.body))
                    .on_press_maybe((!self.busy).then_some(Message::Ask(
                        Confirmation::Maintenance(MaintenanceAction::Repair),
                    )))
                    .padding([d.space.xs, d.space.md])
                    .style(design::flat_button),
            );
            if snapshot.scope == Scope::User {
                actions = actions.push(
                    button(
                        text(self.t("停止个人引擎", "Stop user engine")).size(d.typography.body),
                    )
                    .on_press_maybe((!self.busy).then_some(Message::Ask(Confirmation::Shutdown)))
                    .padding([d.space.xs, d.space.md])
                    .style(design::flat_button),
                );
            }
            content = content.push(actions.wrap());
            if service.installed {
                content = content.push(
                    text(self.t(
                        "停止后台会关闭开机启动，直到再次启动服务。",
                        "Stopping the service disables automatic startup until you start it again.",
                    ))
                    .size(12),
                );
            }
        } else {
            content = content
                .push(
                    button(self.t("安装或修复系统后台", "Install or repair service"))
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
        content.into()
    }

    fn statistics_view(&self) -> Element<'_, Message> {
        let d = design::get();
        let mut content = column![].spacing(d.content.gap);
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
            ));
        } else {
            content = content.push(card(text(self.t("暂无统计数据", "No statistics yet"))));
        }

        let footer = row![
            text(self.t(
                "刷新会重新读取本机统计数据",
                "Refreshing re-reads local statistics"
            ))
            .size(d.typography.meta)
            .style(design::text_style(design::muted))
            .width(Length::Fill),
            button(text(self.t("刷新统计", "Refresh statistics")).size(d.typography.body))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current)
                        .then_some(Message::Run(Command::Statistics))
                )
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::flat_button),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center);

        content = content.push(Self::rule(false)).push(footer);
        content.into()
    }

    fn diagnostics_view(&self) -> Element<'_, Message> {
        let d = design::get();
        let actions = row![
            button(text(self.t("诊断网络", "Diagnose network")).size(d.typography.body))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current)
                        .then_some(Message::Run(Command::Diagnose))
                )
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::solid),
            button(text(self.t("复制报告", "Copy report")).size(d.typography.body))
                .on_press_maybe((!self.diagnostics.is_empty()).then_some(Message::CopyDiagnostics))
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::flat_button),
            Space::new().width(Length::Fill),
            text(if self.diagnostics.is_empty() {
                self.t("尚未生成报告", "No report yet")
            } else {
                self.t("报告已生成", "Report ready")
            })
            .size(d.typography.meta)
            .style(design::text_style(design::muted)),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center);

        let report: Element<'_, Message> = if self.diagnostics.is_empty() {
            text(self.t(
                "点击「诊断网络」生成本机脱敏报告。不会注销、改 DNS 或修改路由。",
                "Run the diagnosis to collect a redacted local report. It never signs out, changes DNS, or edits routes.",
            ))
            .size(d.typography.meta)
            .style(design::text_style(design::muted))
            .into()
        } else {
            container(
                text(self.diagnostics.clone())
                    .size(d.typography.meta)
                    .font(Font::MONOSPACE),
            )
            .width(Length::Fill)
            .padding(d.space.sm)
            .style(design::tint(design::muted, 0.06))
            .into()
        };

        card(column![actions, Self::rule(false), report].spacing(d.card.gap)).into()
    }

    fn network_facts_line(&self) -> Element<'_, Message> {
        let d = design::get();
        let Some(snapshot) = &self.snapshot else {
            return Space::new().into();
        };
        let Some(binding) = &snapshot.active_binding else {
            return Space::new().into();
        };
        let Some(interface) = snapshot
            .interfaces
            .iter()
            .find(|interface| interface.id == binding.interface_id)
        else {
            return Space::new().into();
        };
        let virtual_adapter = interface.virtual_adapter || is_alternate_adapter(interface);
        let name = match &interface.ssid {
            Some(ssid) => format!("{}-{ssid}", interface.name),
            None => interface.name.clone(),
        };
        let (assignment, color) = match interface.ipv4_mode {
            Some(bitnet_core::Ipv4Assignment::Dhcp) => (
                self.t("自动", "Automatic"),
                design::success as fn(&Theme) -> iced::Color,
            ),
            Some(bitnet_core::Ipv4Assignment::Manual) => (
                self.t("手动", "Manual"),
                design::physical as fn(&Theme) -> iced::Color,
            ),
            Some(bitnet_core::Ipv4Assignment::AutoPending) => (
                self.t("自动（未获地址）", "Automatic (no address)"),
                design::muted as fn(&Theme) -> iced::Color,
            ),
            None => (
                self.t("分配方式未知", "Assignment unknown"),
                design::muted as fn(&Theme) -> iced::Color,
            ),
        };
        row![
            text(name).size(d.typography.body),
            ip_chip(&binding.source_ip.to_string(), true),
            badge(assignment, color),
            badge(
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
            ),
            row![
                text(self.t("网关", "Gateway")).size(d.typography.meta),
                ip_chip(
                    &interface
                        .gateway
                        .map_or_else(|| "—".into(), |ip| ip.to_string()),
                    true
                ),
            ]
            .spacing(d.space.xs)
            .align_y(alignment::Vertical::Center),
            row![
                text("DNS").size(d.typography.meta),
                ip_chip(
                    &interface
                        .dns
                        .first()
                        .map_or_else(|| "—".into(), ToString::to_string),
                    true
                ),
            ]
            .spacing(d.space.xs)
            .align_y(alignment::Vertical::Center),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center)
        .wrap()
        .into()
    }

    fn confirmation_view(&self, confirmation: &Confirmation) -> Element<'_, Message> {
        let (title, detail) = match confirmation {
            Confirmation::DeleteProfile(_) => (self.t("删除保存的认证环境？", "Delete this authentication profile?"), self.t("删除该环境快照，不删除账户密码。实际使用的认证目标和当前会话由引擎按统一规则处理。", "Remove this saved profile without deleting account passwords. The engine applies its shared rules to the active configuration and session.")),
            Confirmation::LoginOnce => (self.t("只在当前网络登录一次？", "Connect once on this untrusted network?"), self.t("确认当前网络和认证目标由你信任。本次操作会发送认证材料，但不会保存长期网络信任；后续自动认证仍需明确“信任当前网络”。HTTP 门户无法提供加密的服务器身份保证。", "Confirm that you trust this network and authentication endpoint. This sends authentication material once without saving network trust. Automatic authentication still requires trusting the network. An HTTP portal provides no cryptographic server identity guarantee.")),
            Confirmation::TemporaryLogin { .. } => (self.t("确认仅登录本次？", "Confirm this one-time sign-in?"), self.t("请确认下方网卡和网关属于校园网。凭据只用于此次登录。", "Confirm the adapter and gateway belong to the campus network. Your credentials are used for this sign-in only.")),
            Confirmation::EnableAutomatic { .. } => (self.t("信任当前网络并启用自动认证？", "Trust this network and enable automatic authentication?"), self.t("确认这是校园认证网络。启用后，BitNet 会在已信任的网络自动登录。", "Confirm that this is the campus authentication network. BitNet will then sign in automatically on trusted networks.")),
            Confirmation::SwitchAccount(_) => (
                self.t("切换校园网账户？", "Switch campus account?"),
                self.t(
                    "将退出当前账户并使用下方已保存的账户登录。远程桌面可能因此断开。",
                    "Sign out of the current account and connect with the saved account below. Remote Desktop may disconnect.",
                ),
            ),
            Confirmation::Logout => (self.t("确认注销校园网？", "Disconnect the campus session?"), self.t("可能影响共享这个认证会话的设备。注销后自动认证保持暂停，直到你主动恢复。", "Devices sharing this session may lose connectivity. Automatic authentication stays paused until you resume it.")),
            Confirmation::ForceLogin => (self.t("强制重新认证？", "Force reauthentication?"), self.t("将执行完整的重新认证流程，可能影响共享连接。", "A full reauthentication may affect devices sharing this connection.")),
            Confirmation::DeleteAccount(_) => (self.t("删除保存的账户？", "Delete the saved account?"), self.t("账户及保存的密码将被删除。不会自动注销当前网络会话。", "The account and saved password will be removed. The current network session is not disconnected automatically.")),
            Confirmation::DeleteRule(_) => (self.t("删除网络规则？", "Delete this network rule?"), self.t("更改将立即保存，之后的自动认证使用剩余规则。", "This change is saved immediately. Future authentication uses the remaining rules.")),
            Confirmation::Shutdown => (self.t("停止个人认证引擎？", "Stop the user engine?"), self.t("自动认证和状态检测将停止。当前校园网会话不会自动注销。", "Automatic authentication and monitoring will stop. The campus session is not disconnected automatically.")),
            Confirmation::ClearLogs { .. } => (self.t("清除诊断日志？", "Clear diagnostic logs?"), self.t("清除已写入本机的诊断日志。连接历史和运行统计仍可查看。", "Clear the diagnostic log files on this computer. Connection history and statistics stay available.")),
            Confirmation::Maintenance(action) => match action {
                MaintenanceAction::ServiceInstall => (self.t("启用系统后台？", "Enable the system service?"), self.t("安装器将请求管理员权限，并启用开机无人值守运行。", "The installer requests administrator approval and enables unattended startup.")),
                MaintenanceAction::ServiceStart => (self.t("启动系统后台？", "Start the system service?"), self.t("此操作可能请求管理员确认，之后将重新连接后台。", "Administrator approval may be requested before the interface reconnects to the service.")),
                MaintenanceAction::ServiceStop => (self.t("停止共享系统后台？", "Stop the shared system service?"), self.t("所有本机用户的自动认证和监测都会停止。现有校园网会话不会自动注销。", "Automatic authentication and monitoring stop for all local users. The current campus session is not disconnected automatically.")),
                MaintenanceAction::ServiceRemove { .. } => (self.t("卸载共享系统后台？", "Remove the shared system service?"), self.t("所有本机用户将失去开机无人值守能力。保留设置与受保护凭据；此操作需要管理员确认。", "Unattended startup is removed for all local users. Settings and protected credentials are retained. Administrator approval is required.")),
                MaintenanceAction::Repair => (self.t("修复 BitNet 安装？", "Repair the BitNet installation?"), self.t("使用受信任的安装包修复文件和后台注册。系统级更改需要管理员确认。", "A trusted installer repairs files and service registration. System changes require administrator approval.")),
                MaintenanceAction::InstallUpdate => (self.t("安装软件更新？", "Install the software update?"), self.t("将验证发布签名并运行更新安装。后台升级需要本次管理员确认，取消后保留当前版本。", "The release signature is verified before installation. Updating the service requires administrator approval for this update. Cancelling keeps the current version.")),
                MaintenanceAction::RollbackUpdate => (self.t("恢复上一已验证版本？", "Restore the previous verified version?"), self.t("只允许使用本机上次验签且 API/schema 兼容的缓存版本，执行前会再次验证。没有可用缓存时操作会明确失败，不从未知来源下载旧版。系统后台回滚需本次管理员确认。", "Only a previously verified local cache with a compatible API and schema can be restored, and it is verified again before use. If no usable cache exists, this operation reports unavailable. Service rollback requires administrator approval for this operation.")),
                MaintenanceAction::EnableIpv6 { .. }
                | MaintenanceAction::DisableIpv6 { .. }
                | MaintenanceAction::ResetIpv6 { .. } => {
                    let title = match action {
                        MaintenanceAction::EnableIpv6 { .. } => self.t("启用 IPv6？", "Enable IPv6?"),
                        MaintenanceAction::DisableIpv6 { .. } => self.t("关闭 IPv6？", "Disable IPv6?"),
                        _ => self.t("重新启用 IPv6？", "Re-enable IPv6?"),
                    };
                    (title, self.t("Windows 会请求管理员确认。更改期间，远程桌面或正在使用的网络连接可能会断开。", "Windows will ask for administrator approval. Remote Desktop or other active connections may drop."))
                },
                MaintenanceAction::CheckUpdate | MaintenanceAction::SetAutostart { .. } | MaintenanceAction::SetGuiAutostart { .. } => (self.t("应用维护操作？", "Apply this maintenance operation?"), self.t("将通过统一维护流程应用更改。", "The shared maintenance workflow will apply this change.")),
            },
        };
        let d = design::get();
        let mut contents = column![text(title)
            .size(d.typography.page_title)
            .font(design::family_medium())];
        if let Confirmation::SwitchAccount(target) = confirmation {
            contents = contents.push(text(target.username.clone()).size(d.typography.body));
        }
        if let Confirmation::ClearLogs { scope, bytes } = confirmation {
            let source = match scope {
                Scope::System => self.t("系统后台", "System service"),
                Scope::User => self.t("个人后台", "Personal engine"),
            };
            contents = contents.push(
                text(format!("{} · {}", source, format_log_size(*bytes))).size(d.typography.body),
            );
        }
        if let Confirmation::Maintenance(action) = confirmation {
            if let Some(target) = action.ipv6_target() {
                if let Some(adapter) = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot.interfaces.iter().find(|adapter| {
                        InterfaceTarget::from_interface(adapter)
                            .as_ref()
                            .is_ok_and(|candidate| candidate == target)
                    })
                }) {
                    contents = contents.push(text(&adapter.name).size(d.typography.body));
                }
            }
        }
        if matches!(
            confirmation,
            Confirmation::TemporaryLogin { .. }
                | Confirmation::EnableAutomatic { .. }
                | Confirmation::SwitchAccount(_)
                | Confirmation::LoginOnce
        ) {
            contents = contents.push(self.network_facts_line());
        }

        container(
            container(
                contents
                    .push(text(detail).size(d.typography.body))
                    .push(
                        row![
                            button(text(
                                if matches!(
                                    confirmation,
                                    Confirmation::TemporaryLogin { .. }
                                        | Confirmation::EnableAutomatic { .. }
                                        | Confirmation::LoginOnce
                                ) {
                                    self.t("上一步", "Back")
                                } else {
                                    self.t("取消", "Cancel")
                                }
                            ))
                            .on_press(Message::Cancel)
                            .padding([d.space.xs + 2.0, d.space.md])
                            .style(design::flat_button),
                            button(match confirmation {
                                Confirmation::TemporaryLogin { .. } =>
                                    self.t("确认登录", "Sign in"),
                                Confirmation::ClearLogs { .. } => self.t("清除日志", "Clear logs"),
                                Confirmation::SwitchAccount(_) =>
                                    self.t("切换并登录", "Switch and sign in"),
                                _ => self.t("确认继续", "Confirm"),
                            })
                            .on_press_maybe(
                                (!self.busy
                                    && (!matches!(
                                        confirmation,
                                        Confirmation::TemporaryLogin { .. }
                                    ) || self.snapshot_current))
                                    .then_some(Message::Confirm)
                            )
                            .padding([d.space.xs + 2.0, d.space.md])
                            .style(design::danger_solid)
                        ]
                        .spacing(14),
                    )
                    .spacing(20),
            )
            .padding(design::get().card.pad)
            .style(design::elevated)
            .width(Length::Fill.max(640)),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill)
        .padding(24)
        .style(scrim)
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
