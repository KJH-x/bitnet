use bitnet_core::AppError;
use std::path::PathBuf;

/// Machine reset status that the current Windows user has yet to acknowledge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ipv6StackResetStatus {
    pub id: String,
    pub backup_dir: PathBuf,
    pub reboot_required: bool,
    /// The reset started without a recorded successful exit.
    pub interrupted: bool,
}

#[cfg(windows)]
pub use windows::{acknowledge_ipv6_stack_reset, ipv6_stack_reset_status};
#[cfg(windows)]
pub(super) use windows::{elevated_result, execute};

#[cfg(not(windows))]
pub async fn ipv6_stack_reset_status() -> Result<Option<Ipv6StackResetStatus>, AppError> {
    Ok(None)
}

#[cfg(not(windows))]
pub fn acknowledge_ipv6_stack_reset(_id: &str) -> Result<(), AppError> {
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::{AppError, Ipv6StackResetStatus, PathBuf};
    use crate::maintenance::{
        action_name, err, program_data, reject_links, run_exit_code, run_os_tool,
        secure_windows_directory, system_plan, InterfaceTarget, MaintenanceAction,
        MaintenanceResult, Plan, Staging,
    };
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use bitnet_core::{ErrorCode, Scope};
    use serde::{Deserialize, Serialize};
    use serde_json::json;
    use std::{
        fs::{self, File, OpenOptions},
        io::{Read, Write},
        os::windows::{ffi::OsStrExt, fs::OpenOptionsExt},
        path::Path,
    };

    #[derive(Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ResetRecord {
        id: String,
        boot: u64,
        successful_exit: bool,
    }

    fn record_dir() -> Result<PathBuf, AppError> {
        Ok(program_data()?.join("BitNetIpv6Repair"))
    }

    fn backup_root() -> Result<PathBuf, AppError> {
        Ok(program_data()?
            .join("BitNetMaintenance")
            .join("ipv6-backups"))
    }

    fn acknowledgment_path() -> Result<PathBuf, AppError> {
        Ok(bitnet_platform::paths(Scope::User)?
            .data_dir
            .join("desktop")
            .join("ipv6-reset-ack"))
    }

    fn valid_id(id: &str) -> bool {
        id.strip_prefix("reset-").is_some_and(|suffix| {
            let mut fields = suffix.split('-');
            fields.next().is_some_and(|pid| pid.parse::<u32>().is_ok())
                && fields
                    .next()
                    .is_some_and(|time| time.parse::<u64>().is_ok())
                && fields
                    .next()
                    .is_some_and(|sequence| sequence.parse::<u8>().is_ok())
                && fields.next().is_none()
        }) && id.len() < 64
    }

    fn read_record() -> Result<Option<ResetRecord>, AppError> {
        let path = record_dir()?.join("status.json");
        reject_links(&path)?;
        let mut file = match File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => {
                return Err(err(
                    ErrorCode::Unavailable,
                    "Could not read IPv6 reset status",
                ))
            }
        };
        let mut bytes = Vec::new();
        Read::by_ref(&mut file)
            .take(513)
            .read_to_end(&mut bytes)
            .map_err(|_| err(ErrorCode::Unavailable, "Could not read IPv6 reset status"))?;
        let invalid = || err(ErrorCode::Unavailable, "IPv6 reset status is invalid");
        if bytes.len() > 512 {
            return Err(invalid());
        }
        let record: ResetRecord = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if !valid_id(&record.id) || record.boot == 0 {
            return Err(invalid());
        }
        Ok(Some(record))
    }

    fn publish_record(directory: &Path, record: &ResetRecord) -> Result<(), AppError> {
        let failure = || {
            err(
                ErrorCode::Unavailable,
                "Could not persist IPv6 reset status",
            )
        };
        let data = serde_json::to_vec(record).map_err(|_| failure())?;
        let temporary = directory.join(format!(
            ".{}-{}.tmp",
            record.id,
            if record.successful_exit {
                "complete"
            } else {
                "start"
            }
        ));
        let destination = directory.join("status.json");
        reject_links(&destination)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .custom_flags(0x0020_0000)
            .open(&temporary)
            .map_err(|_| failure())?;
        let written = file.write_all(&data).and_then(|_| file.sync_all());
        drop(file);
        let replaced = written.and_then(|_| fs::rename(&temporary, destination));
        if replaced.is_err() {
            let _ = fs::remove_file(temporary);
        }
        replaced.map_err(|_| failure())
    }

    pub async fn ipv6_stack_reset_status() -> Result<Option<Ipv6StackResetStatus>, AppError> {
        let Some(record) = read_record()? else {
            return Ok(None);
        };
        if acknowledgment_path()
            .ok()
            .and_then(|path| bitnet_platform::read_private_file(&path).ok().flatten())
            .is_some_and(|id| id == record.id.as_bytes())
        {
            return Ok(None);
        }
        Ok(Some(Ipv6StackResetStatus {
            backup_dir: backup_root()?.join(&record.id),
            id: record.id,
            reboot_required: record.boot == boot_id().await?,
            interrupted: !record.successful_exit,
        }))
    }

    pub fn acknowledge_ipv6_stack_reset(id: &str) -> Result<(), AppError> {
        if !valid_id(id) || read_record()?.is_none_or(|record| record.id != id) {
            return Err(err(ErrorCode::InvalidInput, "IPv6 reset status changed"));
        }
        let path = acknowledgment_path()?;
        let directory = path.parent().expect("desktop state has a parent directory");
        bitnet_platform::prepare_private_directory(directory)?;
        bitnet_platform::write_private_file(&path, id.as_bytes())
    }

    fn powershell(script: &str, seconds: u64) -> Result<Plan, AppError> {
        let encoded = STANDARD.encode(
            script
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        system_plan(
            r"WindowsPowerShell\v1.0\powershell.exe",
            &["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded],
            seconds,
        )
    }

    async fn boot_id() -> Result<u64, AppError> {
        let failure = || err(ErrorCode::Unavailable, "Windows boot time is unavailable");
        let plan = powershell(
            r#"
            $ErrorActionPreference = 'Stop'
            $env:PSModulePath = [IO.Path]::Combine($PSHOME, 'Modules')
            [Console]::Write((Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToUniversalTime().Ticks)
        "#,
            15,
        )?;
        let (code, output) = run_os_tool(plan, true).await?;
        if code != 0 {
            return Err(failure());
        }
        std::str::from_utf8(&output)
            .ok()
            .and_then(|value| value.trim().parse().ok())
            .filter(|boot| *boot > 0)
            .ok_or_else(failure)
    }

    fn reserve_backup(root: &Path) -> Result<(String, PathBuf), AppError> {
        secure_windows_directory(root, true, false)?;
        let now = bitnet_core::now_ms();
        for sequence in 0..128 {
            let id = format!("reset-{}-{now}-{sequence}", std::process::id());
            let directory = root.join(&id);
            match fs::create_dir(&directory) {
                Ok(()) => return Ok((id, directory)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                Err(_) => break,
            }
        }
        Err(err(
            ErrorCode::Unavailable,
            "Could not reserve a protected IPv6 backup directory",
        ))
    }

    async fn reset_stack() -> Result<MaintenanceResult, AppError> {
        let status_directory = record_dir()?;
        secure_windows_directory(&status_directory, true, true)?;
        let boot = boot_id().await?;
        if read_record()?.is_some_and(|record| record.boot == boot) {
            return Err(err(
                ErrorCode::Busy,
                "Restart Windows before another IPv6 stack reset",
            ));
        }
        let (id, backup_directory) = reserve_backup(&backup_root()?)?;
        let encoded = STANDARD.encode(
            backup_directory
                .as_os_str()
                .encode_wide()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>(),
        );
        let script = format!(
            "$backupDirectory = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String('{encoded}'))\n{}",
            include_str!("ipv6_backup.ps1"),
        );
        if run_exit_code(powershell(&script, 45)?).await? != 0 {
            return Err(err(
                ErrorCode::Unavailable,
                "The IPv6 settings backup could not be verified",
            ));
        }

        let mut record = ResetRecord {
            id,
            boot,
            successful_exit: false,
        };
        // Persist the attempt before reset. A timeout or interrupted helper must
        // leave the backup discoverable and prevent a second reset on this boot.
        publish_record(&status_directory, &record)?;
        if !matches!(
            run_exit_code(system_plan(
                "netsh.exe",
                &["interface", "ipv6", "reset"],
                40
            )?)
            .await?,
            0 | 3010
        ) {
            return Err(err(
                ErrorCode::Unavailable,
                "Windows did not report a completed IPv6 stack reset",
            ));
        }
        record.successful_exit = true;
        publish_record(&status_directory, &record)?;
        reset_result(backup_directory)
    }

    pub(in crate::maintenance) async fn execute(
        action: &MaintenanceAction,
    ) -> Result<MaintenanceResult, AppError> {
        let _operation = Staging::new(Scope::System)?;
        if let Some(interface) = action.ipv6_target() {
            return adapter_action(action, interface).await;
        }
        match action {
            MaintenanceAction::FlushDnsCache => {
                if run_exit_code(system_plan("ipconfig.exe", &["/flushdns"], 20)?).await? != 0 {
                    return Err(err(
                        ErrorCode::Unavailable,
                        "Windows could not clear the DNS cache",
                    ));
                }
                operation_result(action)
            }
            MaintenanceAction::ResetIpv6Stack => reset_stack().await,
            _ => Err(err(
                ErrorCode::InvalidInput,
                "Unknown IPv6 maintenance action",
            )),
        }
    }

    async fn adapter_action(
        action: &MaintenanceAction,
        interface: &InterfaceTarget,
    ) -> Result<MaintenanceResult, AppError> {
        let script = format!(
            "$interfaceGuid = '{}'\n$interfaceIndex = {}\n$action = '{}'\n{}",
            interface.guid,
            interface.index,
            action_name(action),
            include_str!("ipv6_binding.ps1"),
        );
        match run_exit_code(powershell(&script, 120)?).await? {
            0 => operation_result(action),
            2 => Err(err(
                ErrorCode::InvalidInput,
                "The selected Windows adapter changed or no longer meets the repair conditions",
            )),
            3 => Err(err(ErrorCode::Busy, "Another IPv6 operation is running")),
            _ => Err(err(
                ErrorCode::Unavailable,
                "Windows could not complete the selected IPv6 repair",
            )),
        }
    }

    fn reset_result(backup_directory: PathBuf) -> Result<MaintenanceResult, AppError> {
        Ok(MaintenanceResult {
            message: "Windows IPv6 stack reset completed; restart required".into(),
            details: json!({"reboot_required": true, "ipv6_backup_dir": backup_directory}),
        })
    }

    fn operation_result(action: &MaintenanceAction) -> Result<MaintenanceResult, AppError> {
        let (message, mut details) = match action {
            MaintenanceAction::RenewIpv6 { .. } | MaintenanceAction::RenewDhcpv6 { .. } => (
                "Windows completed IPv6 renewal on the selected adapter",
                json!({"ipv6_renewed": true}),
            ),
            MaintenanceAction::RestartIpv6Adapter { .. } => (
                "Windows restarted the selected adapter",
                json!({"adapter_restarted": true}),
            ),
            MaintenanceAction::FlushDnsCache => (
                "Windows cleared the DNS client cache",
                json!({"dns_cache_flushed": true}),
            ),
            MaintenanceAction::EnableIpv6 { .. } | MaintenanceAction::ResetIpv6 { .. } => (
                "Verified IPv6 enabled on the selected network adapter",
                json!({"ipv6_enabled": true}),
            ),
            MaintenanceAction::DisableIpv6 { .. } => (
                "Verified IPv6 disabled on the selected network adapter",
                json!({"ipv6_enabled": false}),
            ),
            _ => {
                return Err(err(
                    ErrorCode::InvalidInput,
                    "Unknown IPv6 maintenance result",
                ))
            }
        };
        details["reboot_required"] = json!(false);
        Ok(MaintenanceResult {
            message: message.into(),
            details,
        })
    }

    pub(in crate::maintenance) fn elevated_result(
        action: &MaintenanceAction,
        code: u32,
    ) -> Result<MaintenanceResult, AppError> {
        if matches!(action, MaintenanceAction::ResetIpv6Stack) && code == 3010 {
            let record = read_record()?
                .filter(|record| record.successful_exit)
                .ok_or_else(|| {
                    err(
                        ErrorCode::Unavailable,
                        "Could not verify IPv6 reset backup status",
                    )
                })?;
            return reset_result(backup_root()?.join(record.id));
        }
        if code == 0 {
            return operation_result(action);
        }
        let (error, message) = match code as i32 {
            2 => (
                ErrorCode::InvalidInput,
                "The selected Windows adapter changed or no longer meets the repair conditions",
            ),
            3 => (
                ErrorCode::PermissionDenied,
                "Windows refused administrator authorization",
            ),
            4 => (
                ErrorCode::Busy,
                "Another maintenance operation is running or Windows must restart",
            ),
            _ => (
                ErrorCode::Unavailable,
                "Windows could not complete the selected IPv6 repair",
            ),
        };
        Err(err(error, message))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn reset_ids_cannot_select_arbitrary_paths() {
            assert!(valid_id("reset-2468-1234567890-0"));
            for id in [
                "reset-2468-1234567890-0/..",
                "reset-1-2-3-4",
                "reset--1-0",
                "reset-100-9-256",
            ] {
                assert!(!valid_id(id));
            }
        }

        #[test]
        fn helper_results_describe_only_completed_operations() {
            let target =
                InterfaceTarget::parse("{E47D5E9C-1234-4321-ABCD-1234567890AB}@18").unwrap();
            let action = MaintenanceAction::RestartIpv6Adapter { interface: target };
            let result = elevated_result(&action, 0).unwrap();
            assert_eq!(result.details["adapter_restarted"], true);
            assert!(result.details.get("ipv6_enabled").is_none());
            assert_eq!(
                elevated_result(&action, 4).unwrap_err().code,
                ErrorCode::Busy
            );
            assert_eq!(
                elevated_result(&action, 2).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }
}
