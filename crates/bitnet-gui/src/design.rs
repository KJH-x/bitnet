// Design tokens for the BitNet desktop shell. Loaded from JSON so layout
// directions can be compared without recompiling.

#![allow(dead_code)]

use iced::theme::palette;
use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Design {
    pub name: String,
    pub palette: Palettes,
    pub space: Space,
    pub typography: Typography,
    pub nav: Nav,
    pub header: Header,
    pub content: Content,
    pub card: Card,
    pub networks: Networks,
    pub actions: Actions,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Palettes {
    pub light: Palette,
    pub dark: Palette,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub background: String,
    pub surface: String,
    pub chrome: String,
    pub text: String,
    pub muted: String,
    pub line: String,
    pub accent: String,
    pub on_accent: String,
    pub danger: String,
    pub success: String,
    pub physical: String,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Space {
    pub xs: f32,
    pub sm: f32,
    pub md: f32,
    pub lg: f32,
    pub xl: f32,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Typography {
    pub family: String,
    pub family_medium: String,
    pub brand: f32,
    pub page_title: f32,
    pub section_title: f32,
    pub body: f32,
    pub meta: f32,
    pub nav: f32,
    pub line_height: f32,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Nav {
    /// sidebar | rail | topbar
    pub mode: String,
    pub width: f32,
    pub item_height: f32,
    pub item_pad_x: f32,
    pub item_pad_y: f32,
    pub brand_size: f32,
    pub label_size: f32,
    /// fill | bar | underline
    pub selected: String,
    pub brand_gap: f32,
    pub section_gap: f32,
    pub item_gap: f32,
    pub bar_width: f32,
    /// top | center — vertical placement of the navigation block
    pub valign: String,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Header {
    pub title_size: f32,
    pub height: f32,
    pub border: bool,
    pub mode_picker: bool,
    pub refresh: bool,
    pub picker_width: f32,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Content {
    pub pad: f32,
    pub gap: f32,
    pub max_width: f32,
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Actions {
    /// Uniform width for action-row buttons.
    pub button_width: f32,
}

impl Default for Actions {
    fn default() -> Self {
        Self {
            button_width: 150.0,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Networks {
    pub group_by_kind: bool,
    pub row_pad_y: f32,
    pub dns_width: f32,
}

impl Default for Networks {
    fn default() -> Self {
        Self {
            group_by_kind: false,
            row_pad_y: 7.0,
            dns_width: 132.0,
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct Card {
    pub pad: f32,
    pub border: f32,
    pub radius: f32,
    /// surface | none
    pub tone: String,
    pub gap: f32,
}

impl Default for Design {
    fn default() -> Self {
        Self {
            name: "current".into(),
            palette: Palettes::default(),
            space: Space::default(),
            typography: Typography::default(),
            nav: Nav::default(),
            header: Header::default(),
            content: Content::default(),
            card: Card::default(),
            networks: Networks::default(),
            actions: Actions::default(),
        }
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self::light()
    }
}

impl Default for Palettes {
    fn default() -> Self {
        Self {
            light: Palette::light(),
            dark: Palette::dark(),
        }
    }
}

impl Palette {
    fn light() -> Self {
        Self {
            background: "#FFFFFF".into(),
            surface: "#F7F8FA".into(),
            chrome: "#FFFFFF".into(),
            text: "#14171A".into(),
            muted: "#667085".into(),
            line: "#E4E7EC".into(),
            accent: "#3E63DD".into(),
            on_accent: "#FFFFFF".into(),
            danger: "#D92D20".into(),
            success: "#12805C".into(),
            physical: "#C2410C".into(),
        }
    }
    fn dark() -> Self {
        Self {
            background: "#14161A".into(),
            surface: "#1B1E23".into(),
            chrome: "#171A1E".into(),
            text: "#E7E9EC".into(),
            muted: "#98A2B3".into(),
            line: "#2A2F37".into(),
            accent: "#6E8BFF".into(),
            on_accent: "#0B0D10".into(),
            danger: "#F97066".into(),
            success: "#3CCB92".into(),
            physical: "#F59E5B".into(),
        }
    }
}

impl Default for Space {
    fn default() -> Self {
        Self {
            xs: 4.0,
            sm: 8.0,
            md: 12.0,
            lg: 16.0,
            xl: 24.0,
        }
    }
}

impl Default for Typography {
    fn default() -> Self {
        Self {
            family: "BitNet Han Sans SC".into(),
            family_medium: "BitNet Han Sans SC Medium".into(),
            brand: 19.0,
            page_title: 21.0,
            section_title: 14.0,
            body: 14.0,
            meta: 12.0,
            nav: 14.0,
            line_height: 1.45,
        }
    }
}

impl Default for Nav {
    fn default() -> Self {
        Self {
            mode: "sidebar".into(),
            width: 168.0,
            item_height: 34.0,
            item_pad_x: 4.0,
            item_pad_y: 7.0,
            brand_size: 19.0,
            label_size: 14.0,
            selected: "bar".into(),
            brand_gap: 18.0,
            section_gap: 10.0,
            item_gap: 4.0,
            bar_width: 3.0,
            valign: "top".into(),
        }
    }
}

impl Default for Header {
    fn default() -> Self {
        Self {
            title_size: 21.0,
            height: 48.0,
            border: true,
            mode_picker: true,
            refresh: true,
            picker_width: 180.0,
        }
    }
}

impl Default for Content {
    fn default() -> Self {
        Self {
            pad: 20.0,
            gap: 12.0,
            max_width: 0.0,
        }
    }
}

impl Default for Card {
    fn default() -> Self {
        Self {
            pad: 14.0,
            border: 1.0,
            radius: 0.0,
            tone: "surface".into(),
            gap: 10.0,
        }
    }
}

static DESIGN: OnceLock<Design> = OnceLock::new();

pub fn get() -> &'static Design {
    DESIGN.get_or_init(|| {
        let Ok(path) = std::env::var("BITNET_DESIGN") else {
            return Design::default();
        };
        let Ok(source) = std::fs::read_to_string(&path) else {
            eprintln!("design: cannot read {path}");
            return Design::default();
        };
        match serde_json::from_str::<Design>(&source) {
            Ok(design) => design,
            Err(error) => {
                eprintln!("design: {path}: {error}");
                Design::default()
            }
        }
    })
}

pub fn color(value: &str) -> Color {
    let hex = value.trim().trim_start_matches('#');
    let parsed = u32::from_str_radix(hex, 16).unwrap_or(0);
    match hex.len() {
        8 => Color::from_rgba8(
            (parsed >> 24) as u8,
            (parsed >> 16) as u8,
            (parsed >> 8) as u8,
            (parsed & 0xff) as u8 as f32 / 255.0,
        ),
        _ => Color::from_rgb8((parsed >> 16) as u8, (parsed >> 8) as u8, parsed as u8),
    }
}

fn set(theme: &Theme) -> &'static Palette {
    let palettes = &get().palette;
    if theme.palette().is_dark {
        &palettes.dark
    } else {
        &palettes.light
    }
}

pub fn line(theme: &Theme) -> Color {
    color(&set(theme).line)
}

pub fn muted(theme: &Theme) -> Color {
    color(&set(theme).muted)
}

pub fn accent(theme: &Theme) -> Color {
    color(&set(theme).accent)
}

pub fn text(theme: &Theme) -> Color {
    color(&set(theme).text)
}

pub fn app_style(theme: &Theme) -> iced::theme::Style {
    let palette = set(theme);
    iced::theme::Style {
        background_color: color(&palette.background),
        text_color: color(&palette.text),
    }
}

/// Builds the palette used for the whole application.
pub fn theme(name: &str) -> Theme {
    let palettes = &get().palette;
    let palette = if name == "dark" {
        &palettes.dark
    } else {
        &palettes.light
    };
    let seed = palette::Seed {
        background: color(&palette.background),
        text: color(&palette.text),
        primary: color(&palette.accent),
        success: color(&palette.success),
        warning: color(&palette.accent),
        danger: color(&palette.danger),
    };
    Theme::custom(get().name.clone(), seed)
}

pub fn panel(theme: &Theme) -> container::Style {
    let design = get();
    let surface = if design.card.tone == "none" {
        Color::TRANSPARENT
    } else {
        color(&set(theme).surface)
    };
    container::Style {
        background: (surface.a > 0.0).then_some(Background::Color(surface)),
        border: Border {
            color: line(theme),
            width: design.card.border,
            radius: design.card.radius.into(),
        },
        ..container::Style::default()
    }
}

/// Always-solid surface used by modal dialogs, independent of the page
/// card tone.
pub fn elevated(theme: &Theme) -> container::Style {
    let design = get();
    container::Style {
        background: Some(Background::Color(color(&set(theme).surface))),
        border: Border {
            color: line(theme),
            width: 1.0,
            radius: design.card.radius.into(),
        },
        shadow: iced::Shadow {
            color: iced::Color::from_rgba(0.0, 0.0, 0.0, 0.35),
            offset: iced::Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        ..container::Style::default()
    }
}

pub fn chrome(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(&set(theme).chrome))),
        ..container::Style::default()
    }
}

pub fn bare(_theme: &Theme) -> container::Style {
    container::Style::default()
}

pub fn hairline(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(line(theme))),
        ..container::Style::default()
    }
}

pub fn flat_button(theme: &Theme, status: button::Status) -> button::Style {
    let design = get();
    let mut style = button::Style {
        text_color: text(theme),
        background: None,
        border: Border {
            color: line(theme),
            width: 1.0,
            radius: design.card.radius.into(),
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            style.background = Some(Background::Color(color(&set(theme).surface)));
        }
        button::Status::Disabled => {
            style.text_color = muted(theme);
            style.border.color = line(theme);
        }
        button::Status::Active => {}
    }
    style
}

pub fn disabled_button(theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(muted(theme).scale_alpha(0.12))),
        ..flat_button(theme, button::Status::Disabled)
    }
}

pub fn solid(theme: &Theme, status: button::Status) -> button::Style {
    let design = get();
    let accent = color(&set(theme).accent);
    let mut style = button::Style {
        text_color: color(&set(theme).on_accent),
        background: Some(Background::Color(accent)),
        border: Border {
            radius: design.card.radius.into(),
            ..Border::default()
        },
        ..button::Style::default()
    };
    match status {
        button::Status::Hovered | button::Status::Pressed => {
            style.background = Some(Background::Color(accent.scale_alpha(0.85)));
        }
        button::Status::Disabled => {
            style.background = Some(Background::Color(line(theme)));
            style.text_color = muted(theme);
        }
        button::Status::Active => {}
    }
    style
}

pub fn danger_solid(theme: &Theme, status: button::Status) -> button::Style {
    let design = get();
    let danger = color(&set(theme).danger);
    let mut style = button::Style {
        text_color: Color::WHITE,
        background: Some(Background::Color(danger)),
        border: Border {
            radius: design.card.radius.into(),
            ..Border::default()
        },
        ..button::Style::default()
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(danger.scale_alpha(0.85)));
    }
    style
}

/// Navigation item: flat text, no radius. Selected state is driven by the
/// `nav.selected` token (`fill`, `bar` or `underline`).
pub fn nav_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style + 'static {
    move |theme, status| {
        let design = get();
        let mut style = button::Style {
            text_color: if selected {
                accent(theme)
            } else {
                muted(theme)
            },
            background: None,
            border: Border::default(),
            ..button::Style::default()
        };
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        match design.nav.selected.as_str() {
            "fill" if selected => {
                style.background = Some(Background::Color(color(&set(theme).surface)));
                style.text_color = text(theme);
            }
            "bar" if selected => {
                style.text_color = accent(theme);
            }
            "underline" if selected => {
                style.border = Border {
                    color: accent(theme),
                    width: 2.0,
                    radius: 0.0.into(),
                };
            }
            _ => {}
        }
        if hovered && !selected {
            style.text_color = text(theme);
            style.background = Some(Background::Color(color(&set(theme).surface)));
        }
        style
    }
}

/// Left accent bar used by the `bar` selected style.
pub fn accent_bar(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(accent(theme))),
        border: Border {
            color: accent(theme),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

pub fn success(theme: &Theme) -> Color {
    color(&set(theme).success)
}

pub fn text_style(f: fn(&Theme) -> Color) -> impl Fn(&Theme) -> iced::widget::text::Style {
    move |theme| iced::widget::text::Style {
        color: Some(f(theme)),
    }
}

pub fn tint(f: fn(&Theme) -> Color, alpha: f32) -> impl Fn(&Theme) -> container::Style {
    move |theme| container::Style {
        background: Some(Background::Color(f(theme).scale_alpha(alpha))),
        border: Border {
            color: f(theme).scale_alpha(alpha),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn physical(theme: &Theme) -> Color {
    color(&set(theme).physical)
}

/// Badge with a tinted background and the regular theme foreground text.
pub fn badge(f: fn(&Theme) -> Color) -> impl Fn(&Theme) -> container::Style {
    move |theme| container::Style {
        background: Some(Background::Color(f(theme).scale_alpha(0.20))),
        border: Border {
            radius: 0.0.into(),
            ..Border::default()
        },
        ..container::Style::default()
    }
}

/// Full-width selected band behind a navigation item (no border, no radius).
pub fn band(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(&set(theme).surface))),
        ..container::Style::default()
    }
}

pub fn danger(theme: &Theme) -> Color {
    color(&set(theme).danger)
}

pub fn danger_outline(theme: &Theme, status: button::Status) -> button::Style {
    let design = get();
    let tone = color(&set(theme).danger);
    let mut style = button::Style {
        text_color: tone,
        background: None,
        border: Border {
            color: tone,
            width: 1.0,
            radius: design.card.radius.into(),
        },
        ..button::Style::default()
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(tone.scale_alpha(0.12)));
    }
    if matches!(status, button::Status::Disabled) {
        style.text_color = muted(theme);
        style.border.color = line(theme);
    }
    style
}

pub fn danger_text(theme: &Theme, status: button::Status) -> button::Style {
    let tone = color(&set(theme).danger);
    let mut style = button::Style {
        text_color: tone,
        background: None,
        border: Border {
            color: line(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..button::Style::default()
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(tone.scale_alpha(0.12)));
    }
    if matches!(status, button::Status::Disabled) {
        style.text_color = muted(theme);
    }
    style
}

pub fn bare_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::Style {
        text_color: text(theme),
        background: None,
        border: Border::default(),
        ..button::Style::default()
    };
    if matches!(status, button::Status::Hovered | button::Status::Pressed) {
        style.background = Some(Background::Color(
            color(&set(theme).danger).scale_alpha(0.14),
        ));
    }
    style
}

/// Recessed background for code-like values (IPs, IDs).
pub fn code(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(&set(theme).background))),
        border: Border {
            color: line(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn code_text(theme: &Theme) -> Color {
    color(&set(theme).text)
}

/// Look of a text field, used for read-only values.
pub fn field(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(color(&set(theme).surface))),
        border: Border {
            color: line(theme),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..container::Style::default()
    }
}

pub fn family() -> iced::Font {
    match get().typography.family.as_str() {
        "DengXian" => iced::Font::new("DengXian"),
        "Microsoft YaHei UI" => iced::Font::new("Microsoft YaHei UI"),
        "PingFang SC" => iced::Font::new("PingFang SC"),
        "Noto Sans CJK SC" => iced::Font::new("Noto Sans CJK SC"),
        _ => iced::Font::new("BitNet Han Sans SC"),
    }
}

pub fn family_medium() -> iced::Font {
    let name = get().typography.family_medium.as_str();
    match name {
        "" => family(),
        "BitNet Han Sans SC Medium" => iced::Font::new("BitNet Han Sans SC Medium"),
        "DengXian" => iced::Font::new("DengXian"),
        "Microsoft YaHei UI" => iced::Font::new("Microsoft YaHei UI"),
        _ => family(),
    }
}
