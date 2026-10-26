#[cfg(target_os = "macos")]
fn main() {
    if let Err(error) = bitnet_platform::credentials::run_system_keychain_broker() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("The Keychain broker is supported only on macOS");
    std::process::exit(1);
}
