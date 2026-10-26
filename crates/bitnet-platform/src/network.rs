//! Native interface snapshots and conservative route selection.
use bitnet_core::{AppError, NetworkBinding, NetworkInterface};
use std::net::Ipv4Addr;
use tokio::sync::watch;

#[cfg(windows)]
#[path = "network_windows.rs"]
mod native;
#[cfg(unix)]
#[path = "network_unix.rs"]
mod native;

pub fn interfaces() -> Result<Vec<NetworkInterface>, AppError> {
    native::interfaces()
}
pub fn preferred_binding(target: Ipv4Addr) -> Result<Option<NetworkBinding>, AppError> {
    native::preferred_binding(target)
}

/// Keep this guard alive for native notifications. Snapshot polling remains necessary as a fallback.
pub struct NetworkChanges {
    pub receiver: watch::Receiver<u64>,
    pub polling_only: bool,
    _native: Option<native::ChangeGuard>,
    _keepalive: watch::Sender<u64>,
}
pub fn subscribe_changes() -> Result<NetworkChanges, AppError> {
    let (sender, receiver) = watch::channel(0);
    let keepalive = sender.clone();
    Ok(complete_subscription(
        receiver,
        keepalive,
        native::subscribe(sender),
    ))
}
fn complete_subscription(
    receiver: watch::Receiver<u64>,
    keepalive: watch::Sender<u64>,
    result: Result<native::ChangeGuard, AppError>,
) -> NetworkChanges {
    let guard = result.ok();
    let polling_only = guard.as_ref().is_none_or(native::polling_only);
    NetworkChanges {
        receiver,
        polling_only,
        _native: guard,
        _keepalive: keepalive,
    }
}
pub(crate) fn is_tunnel(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "wireguard",
        "wintun",
        "tailscale",
        "zerotier",
        "mihomo",
        "clash",
        "openvpn",
        "tunnel",
        "loopback",
    ]
    .iter()
    .any(|s| n.contains(s))
        || n.starts_with("tun")
        || n.starts_with("tap")
        || n.starts_with("utun")
        || n.starts_with("wg")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn loopback_is_not_an_authentication_binding() {
        assert!(preferred_binding(Ipv4Addr::LOCALHOST).unwrap().is_none());
    }
    #[test]
    fn native_notification_registration_can_be_released() {
        #[cfg(any(windows, target_os = "linux"))]
        {
            let guard = subscribe_changes().unwrap();
            let _ = guard.receiver.borrow().to_owned();
            drop(guard);
        }
    }
    #[test]
    fn denied_native_notifications_keep_polling_channel_open() {
        let (sender, receiver) = watch::channel(0);
        let guard = complete_subscription(
            receiver,
            sender,
            Err(AppError::new(
                bitnet_core::ErrorCode::PermissionDenied,
                "fixture",
            )),
        );
        assert!(guard.polling_only);
        assert!(matches!(guard.receiver.has_changed(), Ok(false)));
    }
}
