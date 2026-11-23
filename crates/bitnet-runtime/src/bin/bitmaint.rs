//! A closed maintenance action CLI; never accepts a command, path, URL or secret.
use bitnet_core::{AppError, ErrorCode, Scope};
use bitnet_runtime::maintenance::{self, InterfaceTarget, MaintenanceAction, UpdateManifestId};
fn arguments() -> Result<(Scope, MaintenanceAction, bool, bool), AppError> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    parse_arguments(&args)
}
fn parse_arguments(args: &[String]) -> Result<(Scope, MaintenanceAction, bool, bool), AppError> {
    let mut scope = Scope::User;
    let mut action = None;
    let mut interactive = false;
    let mut elevated = false;
    let mut purge = false;
    let mut interface = None;
    let mut manifest_sha256 = None;
    let mut seen = std::collections::HashSet::new();
    let mut index = 0;
    while index < args.len() {
        if !seen.insert(args[index].as_str()) {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Duplicate maintenance argument",
            ));
        }
        match args[index].as_str() {
            "--scope" => {
                index += 1;
                scope = match args.get(index).map(String::as_str) {
                    Some("user") => Scope::User,
                    Some("system") => Scope::System,
                    _ => {
                        return Err(AppError::new(
                            ErrorCode::InvalidInput,
                            "Expected --scope user|system",
                        ))
                    }
                };
            }
            "--action" => {
                index += 1;
                action = args.get(index).cloned();
            }
            "--interface" => {
                index += 1;
                interface = Some(InterfaceTarget::parse(args.get(index).ok_or_else(
                    || AppError::new(ErrorCode::InvalidInput, "Expected --interface GUID@index"),
                )?)?);
            }
            "--manifest-sha256" => {
                index += 1;
                let value = args.get(index).ok_or_else(|| {
                    AppError::new(ErrorCode::InvalidInput, "Expected --manifest-sha256 SHA256")
                })?;
                manifest_sha256 = Some(UpdateManifestId::parse(value)?);
            }
            "--interactive" => interactive = true,
            "--elevated" => elevated = true,
            "--purge" => purge = true,
            _ => {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "Unknown maintenance argument",
                ))
            }
        }
        index += 1;
    }
    let action = maintenance::parse_action_with_interface(
        &action.ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "Maintenance --action is required")
        })?,
        purge,
        interface,
    )?;
    let action = match (action, manifest_sha256) {
        (MaintenanceAction::InstallUpdate, Some(manifest_sha256)) => {
            MaintenanceAction::InstallCheckedUpdate { manifest_sha256 }
        }
        (_, Some(_)) => {
            return Err(AppError::new(
                ErrorCode::InvalidInput,
                "Manifest identifier is valid only for installing an update",
            ))
        }
        (action, None) => action,
    };
    if elevated && interactive {
        return Err(AppError::new(
            ErrorCode::InvalidInput,
            "Authorized helpers cannot request another interactive authorization",
        ));
    }
    if elevated && !bitnet_platform::service::is_admin() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Elevated maintenance requires a real administrator token",
        ));
    }
    Ok((scope, action, interactive, elevated))
}
fn main() {
    if std::env::args().len() == 2
        && std::env::args()
            .nth(1)
            .is_some_and(|a| a == "--help" || a == "-h")
    {
        println!("BitNet maintenance\nUsage: bitmaint --scope user|system --action ACTION [--interactive] [--purge] [--interface GUID@index] [--manifest-sha256 SHA256]\nActions: service-install, service-start, service-stop, service-remove, repair,\n         check-update, install-update, rollback-update, autostart-on, autostart-off,\n         gui-autostart-on, gui-autostart-off, ipv6-enable, ipv6-disable, ipv6-reset, ipv6-renew\nIPv6 actions require Windows, system scope and --interface GUID@index.\n--purge is valid only with service-remove. --manifest-sha256 is valid only with install-update.\nNoninteractive operations never elevate.");
        return;
    }
    let outcome = (|| {
        let (scope, action, interactive, elevated) = arguments()?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| {
                AppError::new(
                    ErrorCode::Internal,
                    "Could not initialize maintenance runtime",
                )
            })?;
        if elevated {
            runtime.block_on(maintenance::execute_elevated(scope, action))
        } else {
            runtime.block_on(maintenance::execute(scope, action, interactive))
        }
    })();
    match outcome {
        Ok(result) => {
            println!(
                "{}",
                serde_json::json!({"ok":true,"message":result.message,"details":result.details})
            );
            if result
                .details
                .get("reboot_required")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                std::process::exit(3010);
            }
        }
        Err(error) => {
            eprintln!("{}", serde_json::json!({"ok":false,"error":error}));
            std::process::exit(error.code.exit_code());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<(Scope, MaintenanceAction, bool, bool), AppError> {
        parse_arguments(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }
    #[test]
    fn malformed_and_ambiguous_flags_are_rejected_without_os_actions() {
        assert!(parse(&["--action", "check-update", "--scope", "user"]).is_ok());
        for args in [
            vec!["--action", "service-start", "--purge"],
            vec!["--action", "check-update", "--action", "service-start"],
            vec![
                "--scope",
                "system",
                "--scope",
                "user",
                "--action",
                "check-update",
            ],
            vec!["--action", "service-start;whoami"],
            vec!["--action", "check-update", "--url", "https://example.test"],
            vec!["--action", "check-update", "--interactive", "--elevated"],
            vec!["--action"],
        ] {
            assert!(parse(&args).is_err());
        }
    }

    #[test]
    fn manifest_identifier_is_allowed_only_for_install_update() {
        let hash = "aB".repeat(32);
        assert!(matches!(
            parse(&["--action", "install-update", "--manifest-sha256", &hash])
                .unwrap()
                .1,
            MaintenanceAction::InstallCheckedUpdate { .. },
        ));
        assert!(parse(&["--action", "check-update", "--manifest-sha256", &hash]).is_err());
        assert!(parse(&[
            "--action",
            "install-update",
            "--manifest-sha256",
            "not-a-hash"
        ])
        .is_err());
    }

    #[test]
    fn ipv6_actions_require_one_exact_adapter_identity() {
        let selector = "{E47D5E9C-1234-4321-ABCD-1234567890AB}@18";
        let (_, action, _, _) = parse(&[
            "--scope",
            "system",
            "--action",
            "ipv6-disable",
            "--interface",
            selector,
        ])
        .unwrap();
        assert!(matches!(action, MaintenanceAction::DisableIpv6 { .. }));
        let (_, action, _, _) = parse(&[
            "--scope",
            "system",
            "--action",
            "ipv6-renew",
            "--interface",
            selector,
        ])
        .unwrap();
        assert!(matches!(action, MaintenanceAction::RenewIpv6 { .. }));
        for args in [
            vec!["--scope", "system", "--action", "ipv6-renew"],
            vec!["--action", "ipv6-enable", "--interface", "Ethernet@18"],
            vec![
                "--action",
                "ipv6-enable",
                "--interface",
                "E47D5E9C-1234-4321-ABCD-1234567890AB@0",
            ],
            vec![
                "--action",
                "ipv6-enable",
                "--interface",
                "E47D5E9C-1234-4321-ABCD-1234567890AB@18;exit",
            ],
            vec!["--action", "service-start", "--interface", selector],
            vec![
                "--action",
                "ipv6-enable",
                "--interface",
                selector,
                "--interface",
                selector,
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }
}
