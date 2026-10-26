#[derive(Debug, Clone, Copy)]
#[cfg_attr(not(any(target_os = "windows", target_os = "macos")), allow(dead_code))]
pub enum Action {
    Open,
    Check,
    Pause,
    Resume,
    Exit,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub struct Tray {
    _icon: tray_icon::TrayIcon,
    pixels: Vec<u8>,
    status: Option<(u8, bool, bool)>,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Tray {
    pub fn new() -> Result<Self, String> {
        use tray_icon::{
            menu::{Menu, MenuItem},
            Icon, TrayIconBuilder,
        };
        let menu = Menu::new();
        for (id, title) in [
            ("open", "打开 BitNet / Open"),
            ("check", "立即检查 / Check"),
            ("pause", "暂停自动认证 / Pause"),
            ("resume", "恢复自动认证 / Resume"),
            ("exit", "退出界面 / Exit UI"),
        ] {
            menu.append(&MenuItem::with_id(id, title, true, None))
                .map_err(|e| e.to_string())?;
        }
        let mut pixels = vec![0_u8; 32 * 32 * 4];
        for y in 0..32_i32 {
            for x in 0..32_i32 {
                let i = ((y * 32 + x) * 4) as usize;
                if (x - 16).pow(2) + (y - 16).pow(2) < 225 {
                    pixels[i..i + 4].copy_from_slice(&[39, 106, 219, 255]);
                    if (8..=12).contains(&x) && (8..=24).contains(&y)
                        || (12..=22).contains(&x)
                            && ((8..=11).contains(&y)
                                || (14..=17).contains(&y)
                                || (21..=24).contains(&y))
                        || (20..=23).contains(&x)
                            && ((10..=15).contains(&y) || (17..=22).contains(&y))
                    {
                        pixels[i..i + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
        let icon = Icon::from_rgba(pixels.clone(), 32, 32).map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip("BitNet 校园网管理器")
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            _icon: icon,
            pixels,
            status: None,
        })
    }

    pub fn update(&mut self, snapshot: &bitnet_core::Snapshot, english: bool) {
        use bitnet_core::{AuthState, EngineState, Health};
        let state = if snapshot.session.auth == AuthState::Authenticated {
            if snapshot.health.internet == Health::Degraded {
                2
            } else {
                0
            }
        } else if matches!(
            snapshot.state,
            EngineState::AuthError | EngineState::CredentialsUnavailable
        ) {
            2
        } else if snapshot.settings.paused {
            1
        } else if matches!(
            snapshot.state,
            EngineState::Initializing | EngineState::Checking | EngineState::LoggingIn
        ) {
            4
        } else {
            3
        };
        let key = (state, snapshot.settings.paused, english);
        if self.status == Some(key) {
            return;
        }
        self.status = Some(key);
        let color = match state {
            0 => [31, 145, 93, 255],
            1 => [207, 151, 28, 255],
            2 => [199, 70, 70, 255],
            3 => [110, 120, 132, 255],
            _ => [39, 106, 219, 255],
        };
        let mut pixels = self.pixels.clone();
        for pixel in pixels.chunks_exact_mut(4) {
            if pixel[3] != 0 && pixel != [255, 255, 255, 255] {
                pixel.copy_from_slice(&color);
            }
        }
        if let Ok(icon) = tray_icon::Icon::from_rgba(pixels, 32, 32) {
            let _ = self._icon.set_icon(Some(icon));
        }
        let label = match (state, english) {
            (0, false) => "已认证",
            (0, true) => "Authenticated",
            (1, false) => "自动认证已暂停",
            (1, true) => "Automatic authentication paused",
            (2, false) => "连接需要关注",
            (2, true) => "Connection needs attention",
            (3, false) => "尚未认证",
            (3, true) => "Not authenticated",
            (_, false) => "正在检查",
            (_, true) => "Checking",
        };
        let suffix = if state == 0 && snapshot.settings.paused {
            if english {
                " · automatic authentication paused"
            } else {
                " · 自动认证已暂停"
            }
        } else {
            ""
        };
        let _ = self
            ._icon
            .set_tooltip(Some(format!("BitNet · {label}{suffix}")));
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn poll() -> Option<Action> {
    if let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
        match event.id.as_ref() {
            "open" => Some(Action::Open),
            "check" => Some(Action::Check),
            "pause" => Some(Action::Pause),
            "resume" => Some(Action::Resume),
            "exit" => Some(Action::Exit),
            _ => None,
        }
    } else {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                return Some(Action::Open);
            }
        }
        None
    }
}

// No GTK/AppIndicator build dependency is pulled into the Linux GUI. A system
// without a tray keeps a normal closeable window; the independent engine remains.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub struct Tray;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl Tray {
    pub fn new() -> Result<Self, String> {
        Err("System tray integration is unavailable".into())
    }
    pub fn update(&mut self, _: &bitnet_core::Snapshot, _: bool) {}
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn poll() -> Option<Action> {
    None
}
