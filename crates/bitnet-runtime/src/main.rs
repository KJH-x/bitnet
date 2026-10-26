#![cfg_attr(windows, windows_subsystem = "windows")]
use bitnet_core::{AppError, ErrorCode, Scope};
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--version"] {
        println!("BitNet {}", bitnet_core::APP_VERSION);
        return;
    }
    if args == ["--help"] {
        println!("bitengine [--user | --service] [--simulate --data-dir <absolute-directory>]");
        return;
    }
    let Ok(options) = parse(&args) else {
        std::process::exit(2)
    };
    let system = options.system;
    let simulation = options.simulation;
    if let Some(path) = options.data_dir {
        std::env::set_var("BITNET_DATA_DIR", path);
    }
    if std::env::var("BITNET_SIMULATE_ONLY").as_deref() == Ok("1") && !simulation {
        std::process::exit(2);
    }
    if simulation && (system || std::env::var_os("BITNET_DATA_DIR").is_none()) {
        std::process::exit(2);
    }
    let run = move || -> Result<(), AppError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| AppError::new(ErrorCode::Internal, "无法创建引擎运行时"))?;
        runtime.block_on(bitnet_runtime::engine::run(
            if system { Scope::System } else { Scope::User },
            simulation,
        ))
    };
    let result = if system {
        bitnet_platform::service::run(run)
    } else {
        run()
    };
    if let Err(error) = result {
        if let Ok(paths) = bitnet_platform::paths(if system { Scope::System } else { Scope::User })
        {
            let _ = std::fs::write(
                paths.log_dir.join("startup-error.log"),
                format!("{:?}: {}\n", error.code, error.message),
            );
        }
        std::process::exit(error.code.exit_code());
    }
}

#[derive(Default)]
struct Options {
    system: bool,
    simulation: bool,
    data_dir: Option<std::path::PathBuf>,
}
fn parse(args: &[String]) -> Result<Options, ()> {
    let mut result = Options::default();
    let mut mode_seen = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--user" | "--service" => {
                if mode_seen {
                    return Err(());
                }
                mode_seen = true;
                result.system = arg == "--service";
            }
            "--simulate" => {
                if result.simulation {
                    return Err(());
                }
                result.simulation = true;
            }
            "--data-dir" => {
                if result.data_dir.is_some() {
                    return Err(());
                }
                let path = std::path::PathBuf::from(args.next().ok_or(())?);
                if !path.is_absolute()
                    || path
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    return Err(());
                }
                result.data_dir = Some(path);
            }
            _ => return Err(()),
        }
    }
    if result.system && (result.simulation || result.data_dir.is_some()) {
        return Err(());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }
    #[test]
    fn ambiguous_or_unknown_engine_flags_fail_before_io() {
        for values in [
            &["--simlate"][..],
            &["--user", "--service"],
            &["--simulate", "--simulate"],
            &["--data-dir"],
            &["--data-dir", "relative"],
            &["--service", "--simulate"],
        ] {
            assert!(parse(&arguments(values)).is_err());
        }
        assert!(parse(&[]).is_ok());
        assert!(parse(&arguments(&["--service"])).unwrap().system);
        assert!(
            parse(&arguments(&["--user", "--simulate"]))
                .unwrap()
                .simulation
        );
    }
}
