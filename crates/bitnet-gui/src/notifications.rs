//! User-session notifications. Payloads are static; never copy gateway errors,
//! account names, IP addresses, passwords, URLs or diagnostics into a toast.
use bitnet_core::{AuthState, EngineState, ErrorCode, Snapshot};

const PERSISTENCE_SECONDS: u64 = 10;
const COOLDOWN_SECONDS: u64 = 15 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Credentials,
    Vault,
    Update,
}

impl Kind {
    pub fn body(self, english: bool) -> &'static str {
        match (self, english) {
            (Self::Credentials, false) => "校园网账户需要处理，自动重试已暂停。请打开 BitNet 检查账户并替换密码。",
            (Self::Credentials, true) => "Your campus account needs attention. Automatic retries are paused. Open BitNet to check the account and replace its password.",
            (Self::Vault, false) => "安全凭据存储暂不可用。请打开 BitNet 查看提示并修复或重新输入账户。",
            (Self::Vault, true) => "Protected credentials are unavailable. Open BitNet to review the error and repair storage or enter the account again.",
            (Self::Update, false) => "发现可用更新。请打开 BitNet 的软件更新页面查看并确认安装。",
            (Self::Update, true) => "An update is available. Open the Updates page in BitNet to review it and confirm installation.",
        }
    }
}

#[derive(Default)]
pub struct Gate {
    candidate: Option<(Kind, u64)>,
    sent: Option<Kind>,
    last_sent_at: Option<u64>,
}

impl Gate {
    pub fn observe(&mut self, snapshot: &Snapshot, now: u64) -> Option<Kind> {
        let blocked = snapshot.session.auth != AuthState::Authenticated
            && snapshot
                .settings
                .selected_account
                .as_ref()
                .is_some_and(|id| snapshot.settings.blocked_accounts.contains(id));
        let vault_unavailable = snapshot.session.auth != AuthState::Authenticated
            && snapshot
                .last_error
                .as_ref()
                .is_some_and(|error| error.code == ErrorCode::CredentialsUnavailable);
        let kind = if snapshot.settings.notify_errors {
            match snapshot.state {
                EngineState::AuthError => Some(Kind::Credentials),
                EngineState::CredentialsUnavailable => Some(Kind::Vault),
                _ if blocked => Some(Kind::Credentials),
                _ if vault_unavailable => Some(Kind::Vault),
                _ => None,
            }
        } else {
            None
        };
        let Some(kind) = kind else {
            self.candidate = None;
            self.sent = None;
            return None;
        };
        let since = match self.candidate {
            Some((previous, since)) if previous == kind => since,
            _ => {
                self.candidate = Some((kind, now));
                return None;
            }
        };
        if self.sent == Some(kind)
            || now.saturating_sub(since) < PERSISTENCE_SECONDS
            || self
                .last_sent_at
                .is_some_and(|last| now.saturating_sub(last) < COOLDOWN_SECONDS)
        {
            return None;
        }
        self.sent = Some(kind);
        self.last_sent_at = Some(now);
        Some(kind)
    }
}

fn notification(kind: Kind, english: bool) -> notify_rust::Notification {
    let mut value = notify_rust::Notification::new();
    value
        .appname("BitNet")
        .summary("BitNet")
        .body(kind.body(english));
    #[cfg(target_os = "windows")]
    value.app_id("top.nsapi.bitnet.gui");
    value
}

#[cfg(target_os = "linux")]
pub async fn show(kind: Kind, english: bool) -> Result<(), String> {
    notification(kind, english)
        .show_async()
        .await
        .map(|_| ())
        .map_err(|_| "Desktop notification service is unavailable".into())
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
pub async fn show(kind: Kind, english: bool) -> Result<(), String> {
    // The OS bridge is synchronous. Keep it off the GUI executor; create this
    // short-lived worker only when a deduplicated notification is due.
    let (sender, receiver) = iced::futures::channel::oneshot::channel();
    std::thread::Builder::new()
        .name("bitnet-notify".into())
        .spawn(move || {
            #[cfg(target_os = "macos")]
            if notify_rust::set_application("top.nsapi.bitnet.gui").is_err() {
                let _ = sender.send(Err(
                    "The BitNet app bundle is not available for notifications".into(),
                ));
                return;
            }
            let result = notification(kind, english).show().map(|_| ()).map_err(|_| {
                "Desktop notifications are unavailable for this BitNet installation".into()
            });
            let _ = sender.send(result);
        })
        .map_err(|_| "Could not start the notification worker".to_string())?;
    receiver
        .await
        .map_err(|_| "The notification worker stopped".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitnet_core::Scope;

    #[test]
    fn persistent_credentials_are_notified_once_and_recovery_resets_the_episode() {
        let mut gate = Gate::default();
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.state = EngineState::AuthError;
        assert_eq!(gate.observe(&snapshot, 100), None);
        assert_eq!(gate.observe(&snapshot, 109), None);
        assert_eq!(gate.observe(&snapshot, 110), Some(Kind::Credentials));
        assert_eq!(gate.observe(&snapshot, 5000), None);
        snapshot.state = EngineState::Online;
        assert_eq!(gate.observe(&snapshot, 5001), None);
        snapshot.state = EngineState::AuthError;
        assert_eq!(gate.observe(&snapshot, 5002), None);
        assert_eq!(gate.observe(&snapshot, 5012), Some(Kind::Credentials));
    }

    #[test]
    fn preference_and_global_cooldown_prevent_notification_storms() {
        let mut gate = Gate::default();
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.state = EngineState::AuthError;
        snapshot.settings.notify_errors = false;
        assert_eq!(gate.observe(&snapshot, 100), None);
        assert_eq!(gate.observe(&snapshot, 1000), None);
        snapshot.settings.notify_errors = true;
        assert_eq!(gate.observe(&snapshot, 1001), None);
        assert_eq!(gate.observe(&snapshot, 1011), Some(Kind::Credentials));
        snapshot.state = EngineState::CredentialsUnavailable;
        assert_eq!(gate.observe(&snapshot, 1012), None);
        assert_eq!(gate.observe(&snapshot, 1022), None);
        assert_eq!(gate.observe(&snapshot, 1911), Some(Kind::Vault));
    }

    #[test]
    fn periodic_checking_does_not_reset_a_persisted_credential_error_episode() {
        let mut gate = Gate::default();
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.settings.selected_account = Some("campus".into());
        snapshot.settings.blocked_accounts.push("campus".into());
        snapshot.state = EngineState::AuthError;
        assert_eq!(gate.observe(&snapshot, 0), None);
        assert_eq!(gate.observe(&snapshot, 10), Some(Kind::Credentials));
        snapshot.state = EngineState::Checking;
        assert_eq!(gate.observe(&snapshot, 2000), None);
        snapshot.state = EngineState::AuthError;
        assert_eq!(gate.observe(&snapshot, 2010), None);
    }

    #[test]
    fn arbitrary_error_messages_never_enter_notification_text() {
        let mut snapshot = Snapshot::new(Scope::User);
        snapshot.state = EngineState::AuthError;
        snapshot.last_error = Some(bitnet_core::AppError::new(
            bitnet_core::ErrorCode::Credentials,
            "synthetic-secret-from-error",
        ));
        let mut gate = Gate::default();
        assert_eq!(gate.observe(&snapshot, 0), None);
        let kind = gate.observe(&snapshot, 10).unwrap();
        assert!(!kind.body(false).contains("synthetic-secret-from-error"));
        assert!(!kind.body(true).contains("synthetic-secret-from-error"));
    }
}
