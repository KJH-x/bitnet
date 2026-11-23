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
        32.0,
    )
    .height(Length::Fixed(32.0))
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
        (Ipv6Status::LinkLocalOnly, false) => "仅限本地",
        (Ipv6Status::LinkLocalOnly, true) => "Local only",
        (Ipv6Status::GlobalUnreachable, false) => "探测失败",
        (Ipv6Status::GlobalUnreachable, true) => "Check failed",
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
fn button_label(label: &str, size: f32) -> iced::widget::Text<'_> {
    text(label)
        .size(size)
        .wrapping(text::Wrapping::None)
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Center)
}

fn action_button<'a>(label: &'a str, width: f32) -> iced::widget::button::Button<'a, Message> {
    let d = design::get();
    button(button_label(label, d.typography.body))
        .width(Length::Fixed(width))
        .padding([d.space.xs + 2.0, d.space.md])
}

fn service_status(
    label: &str,
    width: Length,
    size: f32,
    padding: [f32; 2],
) -> Element<'_, Message> {
    container(button_label(label, size).style(design::text_style(design::success)))
        .width(width)
        .padding(padding)
        .into()
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
            .wrapping(text::Wrapping::None)
            .ellipsis(text::Ellipsis::End)
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
        let settings_dialog = self.settings_extra_dialog();
        let detail_dialog = self
            .health_detail
            .map(|detail| self.health_detail_dialog(detail));
        let confirmation = self.confirmation.as_ref();
        let health_editor = self.health_editor.as_deref();
        let prompt = self.network_prompt.as_ref().filter(|_| {
            self.snapshot_current
                && !self.enabling_automatic
                && health_editor.is_none()
                && !self.account_editing
                && settings_dialog.is_none()
                && detail_dialog.is_none()
                && self.quick_login.is_none()
        });
        let mut layers = stack([layer(
            content,
            self.quick_login.is_some()
                || confirmation.is_some()
                || health_editor.is_some()
                || self.account_editing
                || settings_dialog.is_some()
                || detail_dialog.is_some()
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
        if let Some(dialog) = settings_dialog {
            layers = layers.push(layer(dialog, confirmation.is_some()));
        }
        if let Some(dialog) = detail_dialog {
            layers = layers.push(layer(dialog, confirmation.is_some()));
        }
        if let Some(confirmation) = confirmation {
            layers = layers.push(self.confirmation_view(confirmation));
        }
        if let Some(banner) = self.notice_banner() {
            let d = design::get();
            layers = layers.push(
                container(iced::widget::opaque(banner))
                    .center_x(Length::Fill)
                    .align_bottom(Length::Fill)
                    .padding(iced::Padding {
                        top: 0.0,
                        right: d.content.pad,
                        bottom: 46.0,
                        left: if d.nav.mode == "topbar" {
                            d.content.pad
                        } else {
                            d.nav.width + d.content.pad
                        },
                    }),
            );
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
        let dialog = column![heading].spacing(d.card.gap);
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
        let activity = if self.connecting {
            Some(self.t("正在连接本机引擎…", "Connecting to the local engine…"))
        } else if self.busy {
            Some(self.t("正在处理…", "Working…"))
        } else if preview_only() {
            Some(self.t("模拟模式 · 仅使用测试数据", "Simulation · test data only"))
        } else {
            None
        };
        let (is_error, message, action) = if let Some((error, message)) = &self.notice {
            (
                *error,
                message.as_str(),
                Some((self.t("关闭", "Dismiss"), Message::DismissNotice)),
            )
        } else if let Some(error) = &self.connection_error {
            (
                true,
                error.message.as_str(),
                Some((
                    self.t("后台服务", "Background service"),
                    Message::Navigate(Page::Service),
                )),
            )
        } else {
            let activity = activity?;
            return Some(
                container(text(activity).size(design::get().typography.meta))
                    .padding([6, 12])
                    .style(design::panel)
                    .into(),
            );
        };
        let mut banner = row![text(message).width(Length::Fill)]
            .spacing(12)
            .align_y(alignment::Vertical::Center);
        if let Some((label, message)) = action {
            banner = banner.push(button(label).on_press(message).style(button::text));
        }
        Some(
            container(banner)
                .padding([8, 12])
                .width(Length::Fill.max(660.0))
                .style(if is_error {
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

    fn hide_button(&self, width: Length) -> Element<'_, Message> {
        button(button_label(
            self.t("隐藏到托盘", "Hide to tray"),
            design::get().typography.body,
        ))
        .on_press_maybe(
            self.tray
                .as_ref()
                .filter(|tray| tray.is_available())
                .and(self.window)
                .map(|_| Message::HideWindow),
        )
        .width(width)
        .padding([6, 4])
        .style(design::flat_button)
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
        let mut footer = column![].spacing(d.space.xs).width(Length::Fill);
        if let Some(entry) = self.service_entry(Length::Fill) {
            footer = footer.push(entry);
        }
        if self.tray.as_ref().is_some_and(tray::Tray::is_available) {
            footer = footer.push(self.hide_button(Length::Fill));
        }
        footer = footer.push(
            text(format!("v{APP_VERSION}"))
                .size(d.typography.meta)
                .width(Length::Fill)
                .align_x(alignment::Horizontal::Center),
        );
        let rail = rail.push(Space::new().height(Length::Fill)).push(footer);
        container(rail)
            .width(Length::Fixed(d.nav.width))
            .height(Length::Fill)
            .padding([d.content.pad, d.space.sm])
            .style(design::chrome)
            .into()
    }

    fn service_entry(&self, width: Length) -> Option<Element<'_, Message>> {
        let d = design::get();
        let system_connected = self.snapshot_current
            && self
                .snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.scope == Scope::System)
            && self
                .native_service
                .as_ref()
                .is_none_or(|service| service.running || service.detail.is_some());
        if system_connected {
            Some(service_status(
                self.t("系统后台运行中", "Service running"),
                width,
                d.typography.body,
                [6.0, 4.0],
            ))
        } else if self
            .observed_service()
            .is_some_and(|service| service.running)
        {
            Some(
                button(button_label(
                    self.t("连接到系统后台", "Connect to service"),
                    d.typography.body,
                ))
                .width(width)
                .padding([6, 4])
                .style(design::flat_button)
                .on_press_maybe(
                    (!self.busy && !self.connecting).then_some(Message::ModeChanged(Mode::System)),
                )
                .into(),
            )
        } else {
            None
        }
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
        bar = bar.push(Space::new().width(Length::Fill));
        if let Some(entry) = self.service_entry(Length::Fixed(140.0)) {
            bar = bar.push(entry);
        }
        if self.tray.as_ref().is_some_and(tray::Tray::is_available) {
            bar = bar.push(self.hide_button(Length::Fixed(100.0)));
        }
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
        let title = container(
            text(self.area_page().label(self.english()))
                .size(d.header.title_size)
                .font(design::family_medium()),
        )
        .height(Length::Fixed(30.0))
        .align_y(alignment::Vertical::Center);
        let header = row![title].align_y(alignment::Vertical::Center);
        let mut result = column![header].spacing(d.space.sm);
        if d.header.border {
            result = result.push(Self::rule(false));
        }
        result.into()
    }

    fn workspace_body(&self, d: &design::Design) -> Element<'_, Message> {
        let mut content = column![self.page_header(d)].spacing(d.content.gap);
        content = content.push(match self.page {
            Page::Status | Page::Health | Page::Statistics | Page::Diagnostics => {
                responsive(|size| self.connection_workspace(size)).into()
            }
            Page::Accounts => responsive(|size| {
                self.single_tab(
                    self.t("账户列表", "Saved accounts"),
                    self.accounts_view(Size::new(size.width, (size.height - 38.0).max(0.0))),
                )
            })
            .into(),
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

        let account = if let Some(username) = &snapshot.session.username {
            button(text(username).size(d.typography.body))
                .padding(0)
                .style(design::nav_button(true))
                .on_press(Message::Navigate(Page::Accounts))
                .into()
        } else {
            text("—").size(d.typography.body).into()
        };
        let left = column![
            labelled_row(
                self.t("当前校园账号", "Campus account"),
                account,
                label_width
            ),
            labelled_row(
                self.t("网关所见 IPv4", "Gateway IPv4"),
                ip_chip(&authenticated_ip, revealed),
                label_width,
            ),
            labelled_row(
                self.t("网卡 IPv4", "Adapter IPv4"),
                ip_chip(&local_ip, revealed),
                label_width
            ),
        ]
        .spacing(d.card.gap)
        .width(Length::Fixed(280.0));

        let ipv6 = snapshot.health.observed_ipv6_status();
        let ipv6_address = snapshot
            .health
            .local_ipv6
            .or_else(|| snapshot.health.confirmed_ipv6())
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "—".into());

        let right = column![
            labelled_row(
                self.t("互联网", "Internet"),
                self.health_badge(
                    &snapshot.health.internet,
                    HealthDetail::Internet,
                    badge_width
                ),
                health_label_width
            ),
            labelled_row(
                "DNS",
                self.health_badge(&snapshot.health.dns, HealthDetail::Dns, badge_width),
                health_label_width
            ),
        ]
        .spacing(d.card.gap)
        .width(Length::Fill);
        let compact_title = if english || size.height < 390.0 {
            d.typography.section_title
        } else {
            d.typography.page_title
        };
        let mut summary = column![
            row![
                text(self.state_label(&snapshot.state))
                    .font(design::family_medium())
                    .size(compact_title)
                    .width(Length::Fixed(label_width)),
                badge(auth, auth_role(&snapshot.session.auth)),
                ips_eye_button(revealed),
            ]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center),
            row![left, right].spacing(d.space.xl),
        ]
        .spacing(d.card.gap);
        let ipv6_caption =
            if ipv6 == Ipv6Status::Unknown && snapshot.health.ipv6_checked_at.is_some() {
                self.t("未能确认", "Inconclusive")
            } else {
                ipv6_label(ipv6, english)
            };
        let ip_width =
            (size.width - 2.0 * d.card.pad - label_width - badge_width - 4.0 * d.space.sm)
                .clamp(60.0, 300.0);
        let ipv6_row = row![
            text("IPv6")
                .size(d.typography.meta)
                .style(design::text_style(design::muted))
                .width(Length::Fixed(label_width)),
            container(ip_chip(&ipv6_address, revealed))
                .width(Length::Shrink.max(ip_width))
                .clip(true),
            self.detail_badge(
                ipv6_caption,
                ipv6_role(ipv6),
                HealthDetail::Ipv6,
                badge_width
            ),
            Space::new().width(Length::Fill),
        ]
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
            && (snapshot.settings.paused || snapshot.verification_hold)
        {
            summary = summary.push(
                text(self.t(
                    "自动认证已暂停，可在「软件设置 → 常规」恢复。",
                    "Automatic authentication is paused. Resume it in Settings → General.",
                ))
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
            );
        } else if snapshot.settings.automatic
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
        card(
            summary
                .push(Self::rule(false))
                .push(ipv6_row)
                .push(Self::rule(false))
                .push(actions.wrap()),
        )
        .into()
    }

    fn detail_badge<'a>(
        &'a self,
        label: &'a str,
        role: fn(&Theme) -> Color,
        detail: HealthDetail,
        width: f32,
    ) -> Element<'a, Message> {
        button(badge_cell(label, role, width))
            .padding(0)
            .on_press(Message::HealthDetail(Some(detail)))
            .style(design::flat_button)
            .into()
    }

    fn health_badge<'a>(
        &'a self,
        health: &Health,
        detail: HealthDetail,
        width: f32,
    ) -> Element<'a, Message> {
        self.detail_badge(
            health_badge_label(health, self.english()),
            health_badge_role(health),
            detail,
            width,
        )
    }

    fn probe_outcome(&self, outcome: bitnet_core::ProbeOutcome) -> &'static str {
        use bitnet_core::ProbeOutcome::*;
        match outcome {
            Match => self.t("符合预期", "Expected response"),
            UnexpectedStatus => self.t("响应状态与预期不同", "Unexpected status"),
            UnexpectedBody => self.t("响应内容与预期不同", "Unexpected response body"),
            InvalidTarget => self.t("地址无效", "Invalid address"),
            Timeout => self.t("超时", "Timed out"),
            RequestFailed => self.t("连接失败", "Connection failed"),
            BodyLimit => self.t("响应内容过大", "Response too large"),
        }
    }

    fn health_detail_dialog(&self, detail: HealthDetail) -> Element<'_, Message> {
        use bitnet_core::{DnsMode, DnsProbeResult, Ipv6ProbeKind};
        let title = match detail {
            HealthDetail::Internet => self.t("互联网检测详情", "Internet check"),
            HealthDetail::Dns => self.t("DNS 检测详情", "DNS check"),
            HealthDetail::Ipv6 => self.t("IPv6 检测详情", "IPv6 check"),
        };
        let d = design::get();
        let mut evidence = column![].spacing(d.space.sm);
        if let Some(snapshot) = &self.snapshot {
            let health = &snapshot.health;
            let checked_at = match detail {
                HealthDetail::Ipv6 => health.ipv6_checked_at,
                _ => health.checked_at,
            };
            let checked_at = checked_at
                .and_then(|seconds| i64::try_from(seconds).ok())
                .and_then(|seconds| time::OffsetDateTime::from_unix_timestamp(seconds).ok())
                .map_or_else(
                    || self.t("尚未检测", "Not checked yet").into(),
                    |instant| {
                        format!(
                            "{:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                            instant.year(),
                            instant.month() as u8,
                            instant.day(),
                            instant.hour(),
                            instant.minute(),
                            instant.second()
                        )
                    },
                );
            evidence = evidence.push(pair(self.t("最近检测时间", "Last checked"), checked_at));
            match detail {
                HealthDetail::Internet => {
                    let targets = snapshot.settings.configured_health_targets();
                    if health.health_probes.is_empty() {
                        evidence = evidence.push(
                            text(
                                health
                                    .internet_detail
                                    .as_deref()
                                    .unwrap_or(self.t("暂无站点检测记录", "No site results yet")),
                            )
                            .size(d.typography.meta),
                        );
                    }
                    for probe in &health.health_probes {
                        let name = targets
                            .get(probe.target.saturating_sub(1))
                            .map(|t| t.url.as_str())
                            .unwrap_or("—");
                        let status = probe
                            .status
                            .map_or_else(|| "—".into(), |code| format!("HTTP {code}"));
                        evidence = evidence.push(card(column![
                            text(name.to_owned()).size(d.typography.body),
                            text(format!(
                                "{} · {} · {} ms",
                                self.probe_outcome(probe.outcome),
                                status,
                                probe.elapsed_ms
                            ))
                            .size(d.typography.meta)
                            .style(design::text_style(design::muted)),
                        ]));
                    }
                }
                HealthDetail::Dns => {
                    let adapter = ipv6_adapter(snapshot);
                    let config = adapter.and_then(|adapter| adapter.dns_config.as_ref());
                    let mode = |mode| match mode {
                        Some(DnsMode::Automatic) => self.t("自动", "Automatic"),
                        Some(DnsMode::Manual) => self.t("手动", "Manual"),
                        None => self.t("未能读取", "Unavailable"),
                    };
                    let servers = config
                        .map(|config| {
                            config
                                .servers
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_else(|| {
                            adapter
                                .map(|item| item.dns.iter().map(ToString::to_string).collect())
                                .unwrap_or_default()
                        });
                    evidence = evidence
                        .push(pair(
                            self.t("网卡", "Adapter"),
                            adapter.map_or("—", |item| item.name.as_str()),
                        ))
                        .push(pair(
                            self.t("IPv4 DNS 设置", "IPv4 DNS mode"),
                            mode(config.and_then(|c| c.ipv4_mode)),
                        ))
                        .push(pair(
                            self.t("IPv6 DNS 设置", "IPv6 DNS mode"),
                            mode(config.and_then(|c| c.ipv6_mode)),
                        ))
                        .push(pair(
                            self.t("当前 DNS 服务器", "Current DNS servers"),
                            if servers.is_empty() {
                                "—".into()
                            } else {
                                servers.join("\n")
                            },
                        ));
                    if let Some(probe) = &health.dns_probe {
                        let outcome = match probe.result {
                            DnsProbeResult::Ok => self.t("解析成功", "Resolved"),
                            DnsProbeResult::Timeout => self.t("查询超时", "Timed out"),
                            DnsProbeResult::Empty => self.t("未返回地址", "No addresses returned"),
                            DnsProbeResult::LookupFailed => self.t("查询失败", "Lookup failed"),
                        };
                        evidence = evidence.push(pair(
                            self.t("查询结果", "Lookup result"),
                            format!("{} · {outcome} · {} ms", probe.host, probe.elapsed_ms),
                        ));
                        if !probe.resolved_addresses.is_empty() {
                            let answers: Vec<_> = probe
                                .resolved_addresses
                                .iter()
                                .map(ToString::to_string)
                                .collect();
                            evidence = evidence.push(pair(
                                self.t("返回地址", "Returned addresses"),
                                answers.join("\n"),
                            ));
                        }
                    }
                }
                HealthDetail::Ipv6 => {
                    let show_ip = |ip: Option<std::net::Ipv6Addr>| match ip {
                        Some(ip) if self.ips_revealed => ip.to_string(),
                        Some(_) => self.t("已隐藏", "Hidden").into(),
                        None => "—".into(),
                    };
                    use bitnet_core::{education_network_ipv6, EducationNetworkV6};
                    let mut local = show_ip(health.local_ipv6);
                    if let Some(network) = health.local_ipv6.and_then(education_network_ipv6) {
                        local.push_str(match network {
                            EducationNetworkV6::Cernet => " · CERNET",
                            EducationNetworkV6::Cernet2 => " · CERNET2",
                            EducationNetworkV6::Fiti => " · FITI",
                        });
                    }
                    evidence = evidence
                        .push(pair(self.t("本机 IPv6", "Local IPv6"), local))
                        .push(pair(
                            self.t("站点所见出口 IPv6", "IPv6 seen by site"),
                            show_ip(health.confirmed_ipv6()),
                        ));
                    if let Some(reason) = health
                        .ipv6_detail
                        .as_ref()
                        .filter(|_| health.ipv6_status == health.observed_ipv6_status())
                    {
                        evidence = evidence.push(text(reason).size(d.typography.meta));
                    }
                    for probe in &health.ipv6_probes {
                        let check = match probe.kind {
                            Ipv6ProbeKind::Address => self.t("地址回显", "Address check"),
                            Ipv6ProbeKind::TestIpv6 => self.t("IPv6 连接", "IPv6 connectivity"),
                            Ipv6ProbeKind::DualStack => {
                                self.t("双栈连接", "Dual-stack connectivity")
                            }
                            Ipv6ProbeKind::LargePacket => self.t("大包测试", "Large packet check"),
                        };
                        let provider = snapshot
                            .settings
                            .ipv6_targets
                            .get(probe.target.saturating_sub(1))
                            .map_or("—", String::as_str);
                        let direct_https = provider == bitnet_core::DNS_FREE_IPV6_TARGET;
                        let target = match probe.kind {
                            Ipv6ProbeKind::DualStack => "ds.test-ipv6.com".to_owned(),
                            Ipv6ProbeKind::LargePacket => format!("{provider} · 1600 B"),
                            _ if direct_https => self
                                .t("Cloudflare IPv6 直连", "Cloudflare IPv6 direct")
                                .into(),
                            _ => provider.to_owned(),
                        };
                        let transport = match probe.local_is_v6 {
                            Some(true) => " · IPv6",
                            Some(false) => " · IPv4",
                            None => "",
                        };
                        let outcome = if probe.literal_https_reachable {
                            self.t("直连成功", "Direct connection succeeded")
                        } else if probe.non_public_target {
                            self.t(
                                "目标地址不在公网，未测到真实 IPv6",
                                "No public IPv6 target to test",
                            )
                        } else if probe.kind == Ipv6ProbeKind::DualStack
                            && probe.local_is_v6 == Some(false)
                            && probe.outcome == Some(bitnet_core::ProbeOutcome::Match)
                        {
                            self.t("站点通过 IPv4 响应", "Site reached over IPv4")
                        } else {
                            probe
                                .outcome
                                .map_or(self.t("无结果", "No result"), |result| {
                                    self.probe_outcome(result)
                                })
                        };
                        let status = probe
                            .status
                            .map_or_else(|| "—".into(), |code| format!("HTTP {code}"));
                        evidence = evidence.push(card(column![
                            text(if direct_https {
                                target
                            } else {
                                format!("{check} · {target}")
                            })
                            .size(d.typography.body),
                            text(format!(
                                "{outcome} · {status} · {} ms{transport}",
                                probe.elapsed_ms
                            ))
                            .size(d.typography.meta)
                            .style(design::text_style(design::muted)),
                        ]));
                    }
                }
            }
        }
        let mut content =
            column![iced::widget::scrollable(evidence).height(Length::Shrink.max(285.0))];
        if detail == HealthDetail::Ipv6 {
            if let Some(adapter) = self
                .snapshot
                .as_ref()
                .filter(|snapshot| ipv6_maintenance_available(snapshot))
                .and_then(ipv6_adapter)
                .filter(|adapter| ipv6_maintenance_target(adapter).is_some())
            {
                content = content
                    .push(Self::rule(false))
                    .push(self.ipv6_actions(adapter, false));
            }
        }
        self.dialog(
            title,
            content.spacing(d.space.sm).into(),
            Message::HealthDetail(None),
            730,
        )
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

    fn single_tab<'a>(
        &'a self,
        label: &'a str,
        body: Element<'a, Message>,
    ) -> Element<'a, Message> {
        let d = design::get();
        column![
            button(text(label).size(d.typography.body))
                .on_press(Message::Navigate(self.page))
                .padding([d.space.xs, d.space.sm])
                .style(design::nav_button(true)),
            body,
        ]
        .spacing(d.content.gap)
        .into()
    }

    fn service_view(&self) -> Element<'_, Message> {
        let d = design::get();
        let service = self.observed_service();
        let installed = service.is_some_and(|service| service.installed);
        let running = service.is_some_and(|service| service.running);
        let startup = service.is_some_and(|service| service.enabled);
        let idle = !self.busy && !self.connecting;
        let maintenance_ready = idle && !preview_only();
        let known = service.is_some_and(|service| service.detail.is_none());
        let connected = self.snapshot.as_ref().filter(|_| self.snapshot_current);
        let attached_system = connected.is_some_and(|snapshot| snapshot.scope == Scope::System);
        let width = if self.english() {
            144.0
        } else {
            d.actions.button_width
        };
        let control = |label: &'static str,
                       message: Option<Message>,
                       style: fn(&Theme, button::Status) -> button::Style| {
            action_button(label, width)
                .padding([d.space.xs + 2.0, d.space.sm])
                .style(if message.is_some() {
                    style
                } else {
                    design::disabled_button
                })
                .on_press_maybe(message)
        };
        let maintenance = |action, enabled: bool| {
            (maintenance_ready && enabled)
                .then_some(Message::Ask(Confirmation::Maintenance(action)))
        };
        let status = |label| {
            service_status(
                label,
                Length::Fixed(width),
                d.typography.body,
                [d.space.xs + 2.0, d.space.sm],
            )
        };

        let install = if installed {
            row![
                control(
                    self.t("修复安装", "Repair"),
                    maintenance(MaintenanceAction::Repair, true),
                    design::flat_button,
                ),
                control(
                    self.t("卸载后台", "Remove service"),
                    maintenance(MaintenanceAction::ServiceRemove { purge: false }, true),
                    design::danger_outline,
                ),
            ]
        } else {
            row![control(
                self.t("安装服务", "Install service"),
                maintenance(MaintenanceAction::ServiceInstall, known),
                design::solid,
            )]
        };
        let run = row![
            if running {
                status(self.t("正在运行", "Running"))
            } else {
                control(
                    self.t("启动后台", "Start service"),
                    maintenance(MaintenanceAction::ServiceStart, self.can_start_service()),
                    design::solid,
                )
                .into()
            },
            control(
                self.t("停止后台", "Stop service"),
                maintenance(MaintenanceAction::ServiceStop, installed && known),
                design::danger_outline,
            ),
        ];
        let set_startup = |enabled| {
            (maintenance_ready && known && installed)
                .then_some(Message::SetServiceAutostart(enabled))
        };
        let boot = row![
            if startup {
                status(self.t("已启用", "Enabled"))
            } else {
                control(
                    self.t("启用", "Enable"),
                    set_startup(true),
                    if running {
                        design::solid
                    } else {
                        design::flat_button
                    },
                )
                .into()
            },
            control(
                self.t("停用", "Disable"),
                if startup { set_startup(false) } else { None },
                design::danger_outline,
            ),
        ];
        let scope = match connected.map(|snapshot| snapshot.scope) {
            Some(Scope::System) => self.t(
                "系统后台（本机用户共享）",
                "System service (shared by this PC)",
            ),
            Some(Scope::User) if cfg!(windows) => self.t(
                "个人后台（仅当前 Windows 用户）",
                "Personal background (this Windows user only)",
            ),
            Some(Scope::User) => self.t(
                "个人后台（仅当前用户）",
                "Personal background (this user only)",
            ),
            None => self.t("未连接", "Disconnected"),
        };
        let mut scope_value = row![text(scope).size(d.typography.body)]
            .spacing(d.space.sm)
            .align_y(alignment::Vertical::Center);
        if attached_system {
            scope_value = scope_value.push(badge(
                self.t("推荐多数用户", "Recommended"),
                design::success,
            ));
        }

        let reconnect = if attached_system {
            Message::Refresh
        } else {
            Message::ModeChanged(Mode::System)
        };
        let scope_actions = row![control(
            if attached_system {
                self.t("重新连接后台", "Reconnect")
            } else {
                self.t("连接到系统后台", "Connect to service")
            },
            (maintenance_ready && running).then_some(reconnect),
            design::flat_button,
        )]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center);
        let scope_actions = if attached_system || self.mode == Mode::System {
            scope_actions.push(control(
                self.t("切回个人后台", "Switch to personal"),
                (maintenance_ready && !self.mode_switch_has_pending_changes())
                    .then_some(Message::SwitchPersonal),
                design::flat_button,
            ))
        } else if connected.is_some_and(|snapshot| snapshot.scope == Scope::User) {
            scope_actions.push(control(
                self.t("停止个人后台", "Stop personal"),
                maintenance_ready.then_some(Message::Ask(Confirmation::Shutdown)),
                design::danger_outline,
            ))
        } else {
            scope_actions
        };
        let label_width = 136.0;
        let mut rows = column![
            labelled_row(
                self.t("服务安装", "Installation"),
                install
                    .spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center)
                    .into(),
                label_width
            ),
            labelled_row(
                self.t("服务运行", "Service status"),
                run.spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center)
                    .into(),
                label_width
            ),
            labelled_row(
                self.t("开机启动", "Start at boot"),
                boot.spacing(d.space.sm)
                    .align_y(alignment::Vertical::Center)
                    .into(),
                label_width
            ),
            Self::rule(false),
            row![
                text(self.t("当前连接范围", "Connected scope"))
                    .size(d.typography.meta)
                    .width(Length::Fixed(label_width))
                    .style(design::text_style(design::muted)),
                column![scope_value.wrap(), scope_actions.wrap()].spacing(d.space.sm),
            ]
            .spacing(d.space.sm),
        ]
        .spacing(d.card.gap);
        if let Some(detail) = service.and_then(|service| service.detail.as_deref()) {
            rows = rows.push(text(detail).size(d.typography.meta));
        }
        if let Some(last_success) = connected.and_then(|snapshot| snapshot.last_success_at) {
            rows = rows.push(
                text(format!(
                    "{} {}",
                    self.t("上次认证", "Last sign-in:"),
                    self.relative_time(Some(last_success))
                ))
                .size(d.typography.meta)
                .style(design::text_style(design::muted)),
            );
        }
        self.single_tab(
            self.t("服务管理", "Service management"),
            column![
                card(rows),
                row![
                    Space::new().width(Length::Fill),
                    button(self.t("刷新", "Refresh"))
                        .padding([6, 12])
                        .style(design::flat_button)
                        .on_press_maybe(
                            (!self.busy && !self.connecting).then_some(Message::Refresh)
                        )
                ],
            ]
            .spacing(d.content.gap)
            .into(),
        )
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
            Space::new().width(Length::Fill),
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
        let ready = self.diagnostics.is_some();
        let actions = row![
            button(text(self.t("诊断网络", "Diagnose network")).size(d.typography.body))
                .on_press_maybe(
                    (!self.busy && self.snapshot_current && !self.diagnostics_running)
                        .then_some(Message::RunDiagnostics)
                )
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::solid),
            button(
                text(self.t("在浏览器打开报告", "Open report in browser")).size(d.typography.body)
            )
            .on_press_maybe(ready.then_some(Message::OpenDiagnostics))
            .padding([d.space.xs + 2.0, d.space.md])
            .style(design::flat_button),
            button(text(self.t("复制报告", "Copy report")).size(d.typography.body))
                .on_press_maybe(ready.then_some(Message::CopyDiagnostics))
                .padding([d.space.xs + 2.0, d.space.md])
                .style(design::flat_button),
        ]
        .spacing(d.space.sm)
        .align_y(alignment::Vertical::Center)
        .wrap();

        let report: Element<'_, Message> = match &self.diagnostics {
            None => text(if self.diagnostics_running {
                self.t("正在收集网络信息…", "Collecting network information…")
            } else {
                self.t(
                    "诊断当前网络，然后在浏览器查看检测过程和网络出口。",
                    "Run diagnostics to inspect network checks and egress in the browser.",
                )
            })
            .size(d.typography.meta)
            .style(design::text_style(design::muted))
            .into(),
            Some(report) => {
                let mut content = column![text(self.t("主要结果", "Key findings"))
                    .font(design::family_medium())
                    .size(d.typography.section_title)];
                for finding in report.findings.iter().take(4) {
                    content = content.push(text(finding).size(d.typography.body));
                }
                content.spacing(d.space.sm).into()
            }
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
            text(name.to_owned()).size(d.typography.body),
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
            Confirmation::Shutdown => (
                self.t("停止个人后台？", "Stop running BitNet in the background?"),
                self.t(
                    "当前用户的自动登录和连接检测将停止。已登录的校园网会话会保留。",
                    "Automatic sign-in and connection checks stop for this user. Your current campus session stays signed in.",
                ),
            ),
            Confirmation::ClearLogs { .. } => (self.t("清除诊断日志？", "Clear diagnostic logs?"), self.t("清除已写入本机的诊断日志。连接历史和运行统计仍可查看。", "Clear the diagnostic log files on this computer. Connection history and statistics stay available.")),
            Confirmation::PersonalMode => (
                self.t("切回个人后台？", "Switch to the personal background mode?"),
                self.t(
                    "系统后台会先停止，之后只有你当前的系统账户能用 BitNet 自动登录。切换时其他用户将暂时失去自动登录和连接检测。此操作需要管理员确认。",
                    "The system service stops first. Only your current user account will then have automatic sign-in and connection checks. This change requires administrator approval.",
                ),
            ),
            Confirmation::Maintenance(action) => match action {
                MaintenanceAction::ServiceInstall => (self.t("安装系统后台？", "Install the system service?"), self.t("需要管理员确认。安装后，BitNet 可在系统登录前自动连接校园网。", "Administrator approval is required. Once installed, BitNet can connect to the campus network before you sign in to this computer.")),
                MaintenanceAction::ServiceStart => (self.t("启动系统后台？", "Start the system service?"), self.t("此操作可能请求管理员确认，之后将重新连接后台。", "Administrator approval may be requested before the interface reconnects to the service.")),
                MaintenanceAction::ServiceStop => (
                    self.t("停止系统后台？", "Stop the system service?"),
                    self.t(
                        "暂停所有本机用户的自动登录和连接检测。已登录的校园网会话会保留；开机启动设置保持不变。",
                        "Automatic sign-in and connection checks stop for everyone on this computer. The current campus session and the start-at-boot setting are kept.",
                    ),
                ),
                MaintenanceAction::ServiceRemove { .. } => (
                    self.t("卸载系统后台？", "Remove the system service?"),
                    self.t(
                        "系统后台会停止并卸载。保留已有设置和已保存的账户。此操作需要管理员确认。",
                        "The system service will be stopped and removed. Your settings and saved accounts are kept. This change requires administrator approval.",
                    ),
                ),
                MaintenanceAction::Repair => (self.t("修复 BitNet 安装？", "Repair the BitNet installation?"), self.t("使用受信任的安装包修复文件和后台注册。系统级更改需要管理员确认。", "A trusted installer repairs files and service registration. System changes require administrator approval.")),
                MaintenanceAction::InstallUpdate | MaintenanceAction::InstallCheckedUpdate { .. } => (self.t("安装软件更新？", "Install the software update?"), self.t("将验证发布签名并运行更新安装。后台升级需要本次管理员确认，取消后保留当前版本。", "The release signature is verified before installation. Updating the service requires administrator approval for this update. Cancelling keeps the current version.")),
                MaintenanceAction::RollbackUpdate => (self.t("恢复上一已验证版本？", "Restore the previous verified version?"), self.t("只允许使用本机上次验签且 API/schema 兼容的缓存版本，执行前会再次验证。没有可用缓存时操作会明确失败，不从未知来源下载旧版。系统后台回滚需本次管理员确认。", "Only a previously verified local cache with a compatible API and schema can be restored, and it is verified again before use. If no usable cache exists, this operation reports unavailable. Service rollback requires administrator approval for this operation.")),
                MaintenanceAction::EnableIpv6 { .. }
                | MaintenanceAction::DisableIpv6 { .. }
                | MaintenanceAction::ResetIpv6 { .. }
                | MaintenanceAction::RenewIpv6 { .. } => {
                    let title = match action {
                        MaintenanceAction::EnableIpv6 { .. } => self.t("启用 IPv6？", "Enable IPv6?"),
                        MaintenanceAction::DisableIpv6 { .. } => self.t("禁用 IPv6？", "Disable IPv6?"),
                        MaintenanceAction::RenewIpv6 { .. } => self.t("重新获取 IPv6 地址？", "Renew the IPv6 address?"),
                        _ => self.t("重启 IPv6？", "Restart IPv6?"),
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
        if let Confirmation::Maintenance(action) = confirmation {
            if let Some(adapter) = action
                .ipv6_target()
                .and_then(|target| self.ipv6_maintenance_adapter(target))
            {
                contents = contents.push(pair(self.t("网卡", "Adapter"), &adapter.name));
            }
        }
        if let Confirmation::SwitchAccount(target) = confirmation {
            contents = contents.push(text(target.username.clone()).size(d.typography.body));
        }
        if let Confirmation::ClearLogs { scope, bytes } = confirmation {
            let source = match scope {
                Scope::System => self.t("系统后台", "System service"),
                Scope::User => self.t("个人后台", "Personal background"),
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
}
