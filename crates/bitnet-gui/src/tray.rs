#![cfg_attr(
    not(any(target_os = "windows", target_os = "macos", target_os = "linux")),
    allow(dead_code)
)]

#[derive(Debug, Clone, Copy)]
pub enum Action {
    Open,
    Check,
    Pause,
    Resume,
    Exit,
}

const MENU: [(Action, &str, &str); 5] = [
    (Action::Open, "open", "打开 BitNet / Open"),
    (Action::Check, "check", "立即检查 / Check"),
    (Action::Pause, "pause", "暂停自动认证 / Pause"),
    (Action::Resume, "resume", "恢复自动认证 / Resume"),
    (Action::Exit, "exit", "退出界面 / Exit UI"),
];

const ICON_SIZE: i32 = 32;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Status {
    Authenticated,
    Paused,
    Attention,
    Disconnected,
    Checking,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Appearance {
    status: Status,
    paused: bool,
    english: bool,
}

impl Appearance {
    const CHECKING: Self = Self {
        status: Status::Checking,
        paused: false,
        english: false,
    };

    fn from_snapshot(snapshot: &bitnet_core::Snapshot, english: bool) -> Self {
        use bitnet_core::{AuthState, EngineState, Health};

        let paused = snapshot.settings.paused || snapshot.verification_hold;
        let status = if snapshot.session.auth == AuthState::Authenticated {
            if snapshot.health.internet == Health::Degraded {
                Status::Attention
            } else {
                Status::Authenticated
            }
        } else if matches!(
            snapshot.state,
            EngineState::AuthError | EngineState::CredentialsUnavailable
        ) {
            Status::Attention
        } else if paused {
            Status::Paused
        } else if matches!(
            snapshot.state,
            EngineState::Initializing | EngineState::Checking | EngineState::LoggingIn
        ) {
            Status::Checking
        } else {
            Status::Disconnected
        };
        Self {
            status,
            paused,
            english,
        }
    }

    fn pixels(self) -> Vec<u8> {
        let color = match self.status {
            Status::Authenticated => [31, 145, 93, 255],
            Status::Paused => [207, 151, 28, 255],
            Status::Attention => [199, 70, 70, 255],
            Status::Disconnected => [110, 120, 132, 255],
            Status::Checking => [39, 106, 219, 255],
        };
        let mut pixels = vec![0_u8; (ICON_SIZE * ICON_SIZE * 4) as usize];
        for y in 0..ICON_SIZE {
            for x in 0..ICON_SIZE {
                if (x - 16).pow(2) + (y - 16).pow(2) >= 225 {
                    continue;
                }
                let i = ((y * ICON_SIZE + x) * 4) as usize;
                let letter = (8..=12).contains(&x) && (8..=24).contains(&y)
                    || (12..=22).contains(&x)
                        && ((8..=11).contains(&y)
                            || (14..=17).contains(&y)
                            || (21..=24).contains(&y))
                    || (20..=23).contains(&x) && ((10..=15).contains(&y) || (17..=21).contains(&y));
                pixels[i..i + 4].copy_from_slice(if letter { &[255; 4] } else { &color });
            }
        }
        pixels
    }

    fn tooltip(self) -> String {
        let label = match (self.status, self.english) {
            (Status::Authenticated, false) => "已认证",
            (Status::Authenticated, true) => "Authenticated",
            (Status::Paused, false) => "自动认证已暂停",
            (Status::Paused, true) => "Automatic authentication paused",
            (Status::Attention, false) => "连接需要关注",
            (Status::Attention, true) => "Connection needs attention",
            (Status::Disconnected, false) => "尚未认证",
            (Status::Disconnected, true) => "Not authenticated",
            (Status::Checking, false) => "正在检查",
            (Status::Checking, true) => "Checking",
        };
        let suffix = if self.status == Status::Authenticated && self.paused {
            if self.english {
                " · automatic authentication paused"
            } else {
                " · 自动认证已暂停"
            }
        } else {
            ""
        };
        format!("BitNet · {label}{suffix}")
    }
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub struct Tray {
    icon: tray_icon::TrayIcon,
    appearance: Appearance,
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
impl Tray {
    pub fn new() -> Result<Self, String> {
        use tray_icon::{
            menu::{Menu, MenuItem},
            Icon, TrayIconBuilder,
        };

        let menu = Menu::new();
        for (_, id, title) in MENU {
            menu.append(&MenuItem::with_id(id, title, true, None))
                .map_err(|e| e.to_string())?;
        }
        let appearance = Appearance::CHECKING;
        let icon = Icon::from_rgba(appearance.pixels(), ICON_SIZE as u32, ICON_SIZE as u32)
            .map_err(|e| e.to_string())?;
        let icon = TrayIconBuilder::new()
            .with_icon(icon)
            .with_tooltip(appearance.tooltip())
            .with_menu(Box::new(menu))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { icon, appearance })
    }

    pub fn is_starting(&self) -> bool {
        false
    }

    pub fn is_available(&self) -> bool {
        true
    }

    pub fn update(&mut self, snapshot: &bitnet_core::Snapshot, english: bool) {
        let appearance = Appearance::from_snapshot(snapshot, english);
        if appearance == self.appearance {
            return;
        }
        self.appearance = appearance;
        if let Ok(icon) =
            tray_icon::Icon::from_rgba(appearance.pixels(), ICON_SIZE as u32, ICON_SIZE as u32)
        {
            let _ = self.icon.set_icon(Some(icon));
        }
        let _ = self.icon.set_tooltip(Some(appearance.tooltip()));
    }

    pub fn poll(&self) -> Option<Action> {
        use tray_icon::{MouseButton, MouseButtonState, TrayIconEvent};

        if let Ok(event) = tray_icon::menu::MenuEvent::receiver().try_recv() {
            return MENU
                .iter()
                .find(|(_, id, _)| *id == event.id.as_ref())
                .map(|(action, _, _)| *action);
        }
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

#[cfg(target_os = "linux")]
mod linux {
    use super::{Action, Appearance, ICON_SIZE, MENU};
    use ksni::TrayMethods;
    use std::{
        sync::mpsc::{self, Receiver, SyncSender},
        time::Duration,
    };
    use tokio::sync::watch;

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Availability {
        Starting,
        Available,
        Unavailable,
    }

    pub struct Tray {
        actions: Receiver<Action>,
        appearance: watch::Sender<Appearance>,
        availability: watch::Receiver<Availability>,
    }

    struct StatusItem {
        actions: SyncSender<Action>,
        appearance: Appearance,
        availability: watch::Sender<Availability>,
    }

    impl ksni::Tray for StatusItem {
        fn id(&self) -> String {
            "bitnet".into()
        }

        fn title(&self) -> String {
            "BitNet".into()
        }

        fn activate(&mut self, _: i32, _: i32) {
            let _ = self.actions.try_send(Action::Open);
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            let mut bytes = self.appearance.pixels();
            // StatusNotifierItem expects ARGB32 in network byte order.
            for pixel in bytes.as_chunks_mut::<4>().0 {
                pixel.rotate_right(1);
            }
            vec![ksni::Icon {
                width: ICON_SIZE,
                height: ICON_SIZE,
                data: bytes,
            }]
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: "BitNet".into(),
                description: self.appearance.tooltip(),
                ..Default::default()
            }
        }

        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            MENU.iter()
                .map(|&(action, _, title)| {
                    ksni::menu::StandardItem {
                        label: title.into(),
                        activate: Box::new(move |item: &mut Self| {
                            let _ = item.actions.try_send(action);
                        }),
                        ..Default::default()
                    }
                    .into()
                })
                .collect()
        }

        fn watcher_offline(&self, _: ksni::OfflineReason) -> bool {
            self.availability.send_replace(Availability::Unavailable);
            false
        }
    }

    impl Tray {
        pub fn new() -> Result<Self, String> {
            let (action_tx, actions) = mpsc::sync_channel(16);
            let (appearance, mut changes) = watch::channel(Appearance::CHECKING);
            let (availability_tx, availability) = watch::channel(Availability::Starting);
            std::thread::Builder::new()
                .name("bitnet-desktop-tray".into())
                .spawn(move || {
                    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                    {
                        runtime.block_on(run(action_tx, &mut changes, &availability_tx));
                    }
                    availability_tx.send_replace(Availability::Unavailable);
                })
                .map_err(|error| error.to_string())?;
            Ok(Self {
                actions,
                appearance,
                availability,
            })
        }

        pub fn is_starting(&self) -> bool {
            self.availability.has_changed().is_ok()
                && *self.availability.borrow() == Availability::Starting
        }

        pub fn is_available(&self) -> bool {
            self.availability.has_changed().is_ok()
                && *self.availability.borrow() == Availability::Available
        }

        pub fn update(&mut self, snapshot: &bitnet_core::Snapshot, english: bool) {
            let appearance = Appearance::from_snapshot(snapshot, english);
            self.appearance.send_if_modified(|current| {
                if *current == appearance {
                    return false;
                }
                *current = appearance;
                true
            });
        }

        pub fn poll(&self) -> Option<Action> {
            self.actions.try_recv().ok()
        }
    }

    async fn run(
        actions: SyncSender<Action>,
        changes: &mut watch::Receiver<Appearance>,
        availability: &watch::Sender<Availability>,
    ) {
        let item = StatusItem {
            actions,
            appearance: *changes.borrow_and_update(),
            availability: availability.clone(),
        };
        let Ok(Ok(handle)) = tokio::time::timeout(Duration::from_secs(3), item.spawn()).await
        else {
            return;
        };
        availability.send_if_modified(|state| {
            if *state == Availability::Starting {
                *state = Availability::Available;
                true
            } else {
                false
            }
        });

        while changes.changed().await.is_ok() {
            let appearance = *changes.borrow_and_update();
            let update = handle.update(|item| item.appearance = appearance);
            if !matches!(
                tokio::time::timeout(Duration::from_secs(2), update).await,
                Ok(Some(()))
            ) {
                availability.send_replace(Availability::Unavailable);
                break;
            }
        }

        let _ = tokio::time::timeout(Duration::from_secs(1), handle.shutdown()).await;
    }
}

#[cfg(target_os = "linux")]
pub use linux::Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
pub struct Tray;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
impl Tray {
    pub fn new() -> Result<Self, String> {
        Err("System tray integration is unavailable".into())
    }

    pub fn is_starting(&self) -> bool {
        false
    }

    pub fn is_available(&self) -> bool {
        false
    }

    pub fn update(&mut self, _: &bitnet_core::Snapshot, _: bool) {}

    pub fn poll(&self) -> Option<Action> {
        None
    }
}
