//! Credential protection. An unavailable OS vault is an error, never a plaintext fallback.
use crate::{error, secure_fs};
use bitnet_core::{AccountSecret, AppError, ErrorCode, Scope};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

#[cfg(any(not(windows), test))]
const AES_HEADER: &[u8] = b"BITNET-AES-GCM-1\0";
#[cfg(windows)]
const DPAPI_HEADER: &[u8] = b"BITNET-DPAPI-1\0";

pub struct CredentialStore {
    scope: Scope,
    file: PathBuf,
    #[cfg(not(windows))]
    key: Zeroizing<[u8; 32]>,
}
impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("scope", &self.scope)
            .field("key", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl CredentialStore {
    pub fn open(scope: Scope, data_dir: &Path) -> Result<Self, AppError> {
        #[cfg(windows)]
        crate::windows::ensure_service_identity(scope)?;
        // The socket-activated root broker provisions /var/run/bitnet before touching Keychain.
        // Trigger it even when a damaged data-directory ACL would fail the checks below,
        // so the runtime can still offer repair IPC after catching this storage error.
        #[cfg(target_os = "macos")]
        let system_key = if scope == Scope::System {
            Some(Zeroizing::new(keychain_broker::request_key()?))
        } else {
            None
        };
        // systemd's DynamicUser StateDirectory deliberately contains a root-created symlink.
        #[cfg(target_os = "linux")]
        let resolved = if scope == Scope::System {
            std::fs::canonicalize(data_dir).map_err(crate::io_error)?
        } else {
            data_dir.to_owned()
        };
        #[cfg(not(target_os = "linux"))]
        let resolved = data_dir.to_owned();
        secure_fs::prepare_directory(&resolved, scope)?;
        #[cfg(all(not(windows), not(target_os = "macos")))]
        let key = Zeroizing::new(platform_key(scope, &resolved)?);
        #[cfg(target_os = "macos")]
        let key = match system_key {
            Some(key) => key,
            None => Zeroizing::new(platform_key(scope, &resolved)?),
        };
        Ok(Self {
            scope,
            file: resolved.join("accounts.enc"),
            #[cfg(not(windows))]
            key,
        })
    }
    pub fn load(&self) -> Result<Vec<AccountSecret>, AppError> {
        let Some(encoded) = secure_fs::read(&self.file)? else {
            return Ok(Vec::new());
        };
        #[cfg(windows)]
        let plaintext = {
            let data = encoded
                .strip_prefix(DPAPI_HEADER)
                .ok_or_else(invalid_store)?;
            Zeroizing::new(crate::windows::dpapi(data, false)?)
        };
        #[cfg(not(windows))]
        let plaintext = Zeroizing::new(decrypt(&self.key, &encoded)?);
        serde_json::from_slice(&plaintext).map_err(|_| invalid_store())
    }
    pub fn save(&self, accounts: &[AccountSecret]) -> Result<(), AppError> {
        if accounts.len() > 128 {
            return Err(error(ErrorCode::InvalidInput, "Too many accounts"));
        }
        let plaintext = Zeroizing::new(
            serde_json::to_vec(accounts)
                .map_err(|_| error(ErrorCode::Internal, "Cannot encode account data"))?,
        );
        #[cfg(windows)]
        let encoded = {
            let mut b = DPAPI_HEADER.to_vec();
            b.extend(crate::windows::dpapi(&plaintext, true)?);
            b
        };
        #[cfg(not(windows))]
        let encoded = encrypt(&self.key, &plaintext)?;
        secure_fs::atomic_write(&self.file, &encoded, self.scope)
    }
}
fn invalid_store() -> AppError {
    error(ErrorCode::CredentialsUnavailable,"Credential data is damaged, incompatible, or protected by a different key; enter credentials again")
}

#[cfg(any(not(windows), test))]
fn encrypt(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    use aes_gcm::{
        aead::{Aead, KeyInit, Payload},
        Aes256Gcm, Nonce,
    };
    let nonce = crate::random::<12>()?;
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|_| invalid_store())?;
    let data = cipher
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: AES_HEADER,
            },
        )
        .map_err(|_| invalid_store())?;
    let mut out = AES_HEADER.to_vec();
    out.extend(nonce);
    out.extend(data);
    Ok(out)
}
#[cfg(any(not(windows), test))]
fn decrypt(key: &[u8; 32], encoded: &[u8]) -> Result<Vec<u8>, AppError> {
    use aes_gcm::{
        aead::{Aead, KeyInit, Payload},
        Aes256Gcm, Nonce,
    };
    let data = encoded
        .strip_prefix(AES_HEADER)
        .filter(|b| b.len() >= 28)
        .ok_or_else(invalid_store)?;
    Aes256Gcm::new_from_slice(key)
        .map_err(|_| invalid_store())?
        .decrypt(
            Nonce::from_slice(&data[..12]),
            Payload {
                msg: &data[12..],
                aad: AES_HEADER,
            },
        )
        .map_err(|_| invalid_store())
}

#[cfg(target_os = "linux")]
fn platform_key(scope: Scope, data_dir: &Path) -> Result<[u8; 32], AppError> {
    use std::os::unix::fs::MetadataExt;
    if scope == Scope::System {
        let directory = std::env::var_os("CREDENTIALS_DIRECTORY")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .ok_or_else(|| {
                error(
                    ErrorCode::CredentialsUnavailable,
                    "systemd LoadCredential masterkey is required",
                )
            })?;
        let path = directory.join("masterkey");
        crate::reject_link(&path)?;
        let m = std::fs::metadata(&path).map_err(|_| {
            error(
                ErrorCode::CredentialsUnavailable,
                "Cannot read systemd credential masterkey",
            )
        })?;
        if !m.is_file()
            || m.mode() & 0o077 != 0
            || ![0, unsafe { libc::geteuid() }].contains(&m.uid())
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "systemd masterkey permissions are unsafe",
            ));
        }
        let bytes = Zeroizing::new(std::fs::read(path).map_err(|_| invalid_store())?);
        return bytes.as_slice().try_into().map_err(|_| {
            error(
                ErrorCode::CredentialsUnavailable,
                "systemd masterkey must contain exactly 32 bytes",
            )
        });
    }
    desktop_secret_service_key(data_dir)
}

#[cfg(target_os = "linux")]
fn desktop_secret_service_key(data_dir: &Path) -> Result<[u8; 32], AppError> {
    use base64::Engine;
    let identifier = data_dir
        .to_str()
        .ok_or_else(|| error(ErrorCode::InvalidInput, "Credential path is not UTF-8"))?;
    let lookup = run_secret_tool(
        &["lookup", "application", "bitnet", "storage", identifier],
        None,
    )?;
    if lookup.0 {
        let value = Zeroizing::new(
            base64::engine::general_purpose::STANDARD
                .decode(lookup.1.trim_ascii())
                .map_err(|_| invalid_store())?,
        );
        return value.as_slice().try_into().map_err(|_| invalid_store());
    }
    // A missing item has exit status 1 and no stderr. Provider/session failures are errors.
    if !lookup.2.is_empty() {
        return Err(error(
            ErrorCode::CredentialsUnavailable,
            "Desktop Secret Service is unavailable or locked",
        ));
    }
    // Never generate a replacement key over an existing encrypted database.
    if data_dir.join("accounts.enc").exists() {
        return Err(error(
            ErrorCode::CredentialsUnavailable,
            "Desktop keyring item is missing; existing credentials have been preserved",
        ));
    }
    let key = crate::random::<32>()?;
    let encoded = Zeroizing::new(base64::engine::general_purpose::STANDARD.encode(key));
    let result = run_secret_tool(
        &[
            "store",
            "--label=BitNet credential key",
            "application",
            "bitnet",
            "storage",
            identifier,
        ],
        Some(encoded.as_bytes()),
    )?;
    if !result.0 {
        return Err(error(
            ErrorCode::CredentialsUnavailable,
            "Desktop keyring refused to save the credential key",
        ));
    }
    Ok(key)
}

#[cfg(target_os = "linux")]
type SecretToolOutput = (bool, Zeroizing<Vec<u8>>, Vec<u8>);
#[cfg(target_os = "linux")]
fn run_secret_tool(args: &[&str], input: Option<&[u8]>) -> Result<SecretToolOutput, AppError> {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child=Command::new("/usr/bin/secret-tool").args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().map_err(|_|error(ErrorCode::CredentialsUnavailable,"Install libsecret-tools and a working Secret Service provider for desktop credential storage"))?;
    if let Some(mut pipe) = child.stdin.take() {
        if let Some(data) = input {
            if pipe.write_all(data).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(invalid_store());
            }
        }
    }
    let start = Instant::now();
    loop {
        if child.try_wait().map_err(crate::io_error)?.is_some() {
            let output = child.wait_with_output().map_err(crate::io_error)?;
            return Ok((
                output.status.success(),
                Zeroizing::new(output.stdout),
                output.stderr,
            ));
        }
        if start.elapsed() > Duration::from_secs(15) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error(
                ErrorCode::CredentialsUnavailable,
                "Desktop keyring operation timed out; no credentials were written in plaintext",
            ));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

#[cfg(target_os = "macos")]
#[path = "keychain.rs"]
mod keychain;
#[cfg(target_os = "macos")]
fn platform_key(scope: Scope, data_dir: &Path) -> Result<[u8; 32], AppError> {
    if scope == Scope::System {
        keychain_broker::request_key()
    } else {
        keychain::key(scope, data_dir)
    }
}

#[cfg(target_os = "macos")]
#[path = "keychain_broker.rs"]
mod keychain_broker;
/// Only the root-owned, socket-activated macOS helper calls this entry point.
#[cfg(target_os = "macos")]
pub fn run_system_keychain_broker() -> Result<(), AppError> {
    keychain_broker::run()
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
fn platform_key(_: Scope, _: &Path) -> Result<[u8; 32], AppError> {
    Err(error(
        ErrorCode::Unavailable,
        "No secure credential backend exists for this platform",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitnet_core::Secret;
    fn account() -> AccountSecret {
        AccountSecret {
            id: "test".into(),
            username: "fixture-user".into(),
            password: Secret::new("fixture-password-never-on-disk"),
        }
    }
    #[test]
    fn authenticated_encryption_rejects_damage_and_wrong_key() {
        let key = [7; 32];
        let a = encrypt(&key, b"secret").unwrap();
        let b = encrypt(&key, b"secret").unwrap();
        assert_ne!(a, b);
        assert_eq!(decrypt(&key, &a).unwrap(), b"secret");
        assert!(decrypt(&[8; 32], &a).is_err());
        let mut bad = a;
        *bad.last_mut().unwrap() ^= 1;
        assert!(decrypt(&key, &bad).is_err());
    }
    #[test]
    fn no_plaintext_fallback() {
        assert!(decrypt(&[1; 32], br#"[{"password":"fixture"}]"#).is_err());
    }
    #[test]
    fn debug_redacts_key() {
        let _ = account();
        assert!(!format!("{:?}", account()).contains("fixture-password"));
    }
    #[cfg(windows)]
    #[test]
    fn dpapi_roundtrip_in_isolated_directory() {
        let temp = tempfile::tempdir_in(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        )
        .unwrap();
        let store = CredentialStore::open(Scope::User, temp.path()).unwrap();
        store.save(&[account()]).unwrap();
        let saved = std::fs::read(temp.path().join("accounts.enc")).unwrap();
        assert!(!String::from_utf8_lossy(&saved).contains("fixture-password"));
        assert_eq!(
            store.load().unwrap()[0].password.expose(),
            account().password.expose()
        );
        let mut bad = saved;
        *bad.last_mut().unwrap() ^= 1;
        std::fs::write(temp.path().join("accounts.enc"), bad).unwrap();
        assert!(store.load().is_err());
    }
    #[cfg(not(windows))]
    #[test]
    fn aes_store_roundtrip_in_isolated_directory() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        std::fs::create_dir_all(&root).unwrap();
        let temp = tempfile::tempdir_in(root).unwrap();
        secure_fs::prepare_directory(temp.path(), Scope::User).unwrap();
        let store = CredentialStore {
            scope: Scope::User,
            file: temp.path().join("accounts.enc"),
            key: Zeroizing::new([19; 32]),
        };
        store.save(&[account()]).unwrap();
        assert_eq!(
            store.load().unwrap()[0].password.expose(),
            account().password.expose()
        );
        assert!(
            !String::from_utf8_lossy(&std::fs::read(&store.file).unwrap())
                .contains("fixture-password")
        );
    }
}
