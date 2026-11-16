//! Personal GUI update scheduling. Authentication and Status never await it.
use crate::maintenance::updates::UpdateConfig;
use bitnet_core::{AppError, ErrorCode, Scope, Settings};
use std::time::Duration;

pub const STARTUP_DELAY_SECS: u64 = 30;
pub const CHECK_INTERVAL_SECS: u64 = 24 * 60 * 60;
pub const FAILURE_BACKOFF_SECS: u64 = 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    Available {
        version: String,
    },
    PublicRelease {
        version: String,
    },
    NoUpdate,
    Unconfigured,
    /// A separate updater was started; this does not claim installation success.
    InstallerStarted {
        message: String,
    },
}

#[derive(Debug)]
pub struct UpdatePolicy {
    next_check: u64,
    last_clock: u64,
    in_flight: bool,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self::new()
    }
}
impl UpdatePolicy {
    pub fn new() -> Self {
        Self::new_at(bitnet_core::now_secs())
    }
    pub fn new_at(now_secs: u64) -> Self {
        Self {
            next_check: now_secs.saturating_add(STARTUP_DELAY_SECS),
            last_clock: now_secs,
            in_flight: false,
        }
    }

    /// Called by the GUI's existing timer. Marks a check in flight before
    /// returning true, so repeated timer events cannot launch duplicate work.
    pub fn due(&mut self, settings: &Settings, now_secs: u64) -> bool {
        if now_secs < self.last_clock {
            self.next_check = now_secs.saturating_add(STARTUP_DELAY_SECS);
        }
        self.last_clock = now_secs;
        if !settings.check_updates || self.in_flight || now_secs < self.next_check {
            return false;
        }
        self.in_flight = true;
        true
    }

    pub fn complete(&mut self, success: bool, now_secs: u64) {
        self.in_flight = false;
        self.last_clock = now_secs;
        self.next_check = now_secs.saturating_add(if success {
            CHECK_INTERVAL_SECS
        } else {
            FAILURE_BACKOFF_SECS
        });
    }

    pub fn begin_manual(&mut self) -> bool {
        if self.in_flight {
            return false;
        }
        self.in_flight = true;
        true
    }
}

/// Run as an independent GUI task, never on the authentication state machine.
/// No configured release trust means no network I/O or installer process.
pub async fn check(scope: Scope, auto_update: bool) -> Result<UpdateOutcome, AppError> {
    if !UpdateConfig::configured() {
        return Ok(UpdateOutcome::Unconfigured);
    }
    let config = UpdateConfig::embedded()?;
    let Some(manifest) = config.check_available().await? else {
        return Ok(UpdateOutcome::NoUpdate);
    };
    // Make sure a complete set of installers exists before notifying or acting.
    let _ = manifest.assets_for_install(scope)?;
    #[cfg(windows)]
    let service_registered = bitnet_platform::service::status().installed;
    #[cfg(not(windows))]
    let service_registered = false;
    if auto_install_allowed(scope, auto_update, service_registered, std::env::consts::OS) {
        start_personal_updater()?;
        return Ok(UpdateOutcome::InstallerStarted { message: format!("Verified update {}: the personal updater was started; installation is not yet confirmed", manifest.version) });
    }
    Ok(UpdateOutcome::Available {
        version: manifest.version,
    })
}

/// Public metadata can prompt a manual download. Signed manifests are the
/// only inputs accepted by the installer.
pub async fn check_desktop(scope: Scope, auto_update: bool) -> Result<UpdateOutcome, AppError> {
    const LATEST: &str = "https://api.github.com/repos/KJH-x/bitnet/releases/latest";
    const MAX_BYTES: usize = 256 * 1024;
    if UpdateConfig::configured() {
        return check(scope, auto_update).await;
    }
    let fail = || AppError::new(ErrorCode::Unavailable, "无法查询 BitNet 发布版本");
    let mut response = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|_| fail())?
        .get(LATEST)
        .header(reqwest::header::USER_AGENT, "BitNet")
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| fail())?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(UpdateOutcome::NoUpdate);
    }
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_BYTES as u64)
    {
        return Err(fail());
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| fail())? {
        if chunk.len() > MAX_BYTES - data.len() {
            return Err(fail());
        }
        data.extend_from_slice(&chunk);
    }
    let release: PublicReleaseInfo = serde_json::from_slice(&data).map_err(|_| fail())?;
    Ok(public_release_outcome(release, bitnet_core::APP_VERSION))
}

#[derive(serde::Deserialize)]
struct PublicReleaseInfo {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn public_release_outcome(release: PublicReleaseInfo, current: &str) -> UpdateOutcome {
    use semver::Version;
    if !release.draft && !release.prerelease {
        if let (Some(candidate), Ok(current)) = (
            release
                .tag_name
                .strip_prefix('v')
                .and_then(|tag| Version::parse(tag).ok()),
            Version::parse(current),
        ) {
            if candidate.pre.is_empty() && candidate > current {
                return UpdateOutcome::PublicRelease {
                    version: candidate.to_string(),
                };
            }
        }
    }
    UpdateOutcome::NoUpdate
}

fn auto_install_allowed(
    scope: Scope,
    enabled: bool,
    service_registered: bool,
    platform: &str,
) -> bool {
    enabled && scope == Scope::User && platform == "windows" && !service_registered
}

fn start_personal_updater() -> Result<(), AppError> {
    #[cfg(windows)]
    {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let helper = std::env::current_exe()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Unavailable,
                    "Could not locate the personal updater",
                )
            })?
            .with_file_name("bitmaint.exe");
        crate::maintenance::reject_links(&helper)?;
        if !helper.is_file() {
            return Err(AppError::new(
                ErrorCode::Unavailable,
                "The installed personal updater is missing",
            ));
        }
        Command::new(helper)
            .args(["--scope", "user", "--action", "install-update"])
            .creation_flags(0x0800_0000 | 0x0000_0008)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Unavailable,
                    "Could not start the personal updater",
                )
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        Err(AppError::new(
            ErrorCode::PermissionDenied,
            "This platform requires explicit authorization for system package updates",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_delays_start_blocks_duplicates_and_uses_success_or_failure_intervals() {
        let settings = Settings::default();
        let mut policy = UpdatePolicy::new_at(100);
        assert!(!policy.due(&settings, 129));
        assert!(policy.due(&settings, 130));
        assert!(!policy.due(&settings, 131));
        policy.complete(true, 140);
        assert!(!policy.due(&settings, 140 + CHECK_INTERVAL_SECS - 1));
        assert!(policy.due(&settings, 140 + CHECK_INTERVAL_SECS));
        policy.complete(false, 100000);
        assert!(!policy.due(&settings, 100000 + FAILURE_BACKOFF_SECS - 1));
        assert!(policy.due(&settings, 100000 + FAILURE_BACKOFF_SECS));
    }
    #[test]
    fn disabled_checks_and_backwards_clock_cannot_create_a_busy_loop() {
        let mut settings = Settings {
            check_updates: false,
            ..Settings::default()
        };
        let mut policy = UpdatePolicy::new_at(100);
        assert!(!policy.due(&settings, 500));
        settings.check_updates = true;
        assert!(policy.due(&settings, 501));
        policy.complete(true, 502);
        assert!(!policy.due(&settings, 100));
        assert!(!policy.due(&settings, 129));
        assert!(policy.due(&settings, 130));
    }
    #[test]
    fn automatic_installation_cannot_elevate_or_replace_a_system_service() {
        assert!(auto_install_allowed(Scope::User, true, false, "windows"));
        for (scope, enabled, registered, platform) in [
            (Scope::System, true, false, "windows"),
            (Scope::User, false, false, "windows"),
            (Scope::User, true, true, "windows"),
            (Scope::User, true, false, "linux"),
            (Scope::User, true, false, "macos"),
        ] {
            assert!(!auto_install_allowed(scope, enabled, registered, platform));
        }
    }
    #[tokio::test]
    async fn unconfigured_build_skips_network_and_installer_even_with_auto_enabled() {
        if !UpdateConfig::configured() {
            assert_eq!(
                check(Scope::User, true).await.unwrap(),
                UpdateOutcome::Unconfigured
            );
            assert_eq!(
                check(Scope::System, true).await.unwrap(),
                UpdateOutcome::Unconfigured
            );
        }
    }

    #[test]
    fn public_release_metadata_can_only_offer_manual_download() {
        for (tag, draft, prerelease) in [
            ("v0.1.2", false, false),
            ("v0.1.3", true, false),
            ("v0.1.3", false, true),
            ("v0.1.3-rc.1", false, false),
            ("https://example.test/download", false, false),
        ] {
            let info = PublicReleaseInfo {
                tag_name: tag.into(),
                draft,
                prerelease,
            };
            assert_eq!(
                public_release_outcome(info, "0.1.2"),
                UpdateOutcome::NoUpdate
            );
        }
        let info = PublicReleaseInfo {
            tag_name: "v0.1.3".into(),
            draft: false,
            prerelease: false,
        };
        assert_eq!(
            public_release_outcome(info, "0.1.2"),
            UpdateOutcome::PublicRelease {
                version: "0.1.3".into()
            }
        );
    }
}
