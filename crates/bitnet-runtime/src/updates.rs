//! Project-authenticated updates. No configured feed/key means no update I/O.
use bitnet_core::{AppError, ErrorCode, Scope, APP_VERSION};
use futures_util::StreamExt;
use minisign_verify::{PublicKey, Signature};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};
use url::Url;

#[path = "update_cache.rs"]
pub(super) mod cache;

pub const MAX_PACKAGE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const MAX_SIGNATURE_BYTES: usize = 8192;
const BYTES_PER_SECOND: u64 = 6 * 1024 * 1024;

fn fail(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema_version: u32,
    pub api_version: u32,
    pub version: String,
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub platform: String,
    pub arch: String,
    pub kind: String,
    pub filename: String,
    pub bytes: u64,
    pub sha256: String,
    pub urls: Vec<String>,
}

pub struct UpdateConfig {
    feed: Url,
    key: PublicKey,
    allowed_origins: Vec<String>,
}

pub(super) struct VerifiedRelease {
    pub manifest: Manifest,
    pub raw_manifest: Vec<u8>,
    pub signature: String,
}
impl UpdateConfig {
    pub fn configured() -> bool {
        option_env!("BITNET_UPDATE_FEED").is_some_and(|v| !v.trim().is_empty())
            && option_env!("BITNET_RELEASE_PUBLIC_KEY").is_some_and(|v| !v.trim().is_empty())
    }

    pub fn embedded() -> Result<Self, AppError> {
        let feed = option_env!("BITNET_UPDATE_FEED")
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                fail(
                    ErrorCode::Unavailable,
                    "Updates are not configured in this build: BITNET_UPDATE_FEED is missing",
                )
            })?;
        let public_key = option_env!("BITNET_RELEASE_PUBLIC_KEY").filter(|v| !v.trim().is_empty())
            .ok_or_else(|| fail(ErrorCode::Unavailable, "Updates are not configured in this build: BITNET_RELEASE_PUBLIC_KEY is missing"))?;
        Self::new(
            feed,
            public_key,
            option_env!("BITNET_UPDATE_ALLOWED_ORIGINS").unwrap_or(""),
        )
    }

    fn new(feed: &str, key: &str, additional_origins: &str) -> Result<Self, AppError> {
        let feed = https_url(feed)?;
        let key = decode_key(key)?;
        let mut allowed_origins = vec![feed.origin().ascii_serialization()];
        for origin in additional_origins
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            let url = https_url(origin)?;
            if url.path() != "/" || url.query().is_some() {
                return Err(fail(
                    ErrorCode::InvalidInput,
                    "Update origins must be HTTPS origins",
                ));
            }
            allowed_origins.push(url.origin().ascii_serialization());
        }
        Ok(Self {
            feed,
            key,
            allowed_origins,
        })
    }

    fn validate_url(&self, value: &str) -> Result<Url, AppError> {
        let url = https_url(value)?;
        if !self
            .allowed_origins
            .contains(&url.origin().ascii_serialization())
        {
            return Err(fail(
                ErrorCode::PermissionDenied,
                "Update download origin is not in the compiled allowlist",
            ));
        }
        Ok(url)
    }

    pub fn verify_manifest(
        &self,
        bytes: &[u8],
        signature: &str,
        current_version: &str,
    ) -> Result<Manifest, AppError> {
        self.verify_manifest_policy(bytes, signature, current_version, true)
    }

    fn verify_manifest_policy(
        &self,
        bytes: &[u8],
        signature: &str,
        current_version: &str,
        require_newer: bool,
    ) -> Result<Manifest, AppError> {
        if bytes.len() > MAX_MANIFEST_BYTES || signature.len() > MAX_SIGNATURE_BYTES {
            return Err(fail(
                ErrorCode::InvalidInput,
                "Update metadata exceeds its size limit",
            ));
        }
        let signature = Signature::decode(signature).map_err(|_| {
            fail(
                ErrorCode::PermissionDenied,
                "Update manifest signature is invalid",
            )
        })?;
        self.key.verify(bytes, &signature, false).map_err(|_| {
            fail(
                ErrorCode::PermissionDenied,
                "Update manifest signature verification failed",
            )
        })?;
        let manifest: Manifest = serde_json::from_slice(bytes).map_err(|_| {
            fail(
                ErrorCode::Incompatible,
                "Signed update manifest has an invalid schema",
            )
        })?;
        if manifest.schema_version != 1 || manifest.api_version != bitnet_core::API_VERSION {
            return Err(fail(
                ErrorCode::Incompatible,
                "Signed update manifest is incompatible with this client",
            ));
        }
        let target = Version::parse(&manifest.version)
            .map_err(|_| fail(ErrorCode::Incompatible, "Signed update version is invalid"))?;
        let current = Version::parse(current_version).map_err(|_| {
            fail(
                ErrorCode::Internal,
                "Installed application version is invalid",
            )
        })?;
        if !target.pre.is_empty() || !target.build.is_empty() {
            return Err(fail(
                ErrorCode::Incompatible,
                "Automatic updates require a stable release version without build metadata",
            ));
        }
        if require_newer && target <= current {
            return Err(fail(ErrorCode::Unavailable, "No newer stable update is available; downgrade and same-version replacement are disabled"));
        }
        if manifest.assets.is_empty() || manifest.assets.len() > 32 {
            return Err(fail(
                ErrorCode::Incompatible,
                "Signed update manifest has an invalid asset count",
            ));
        }
        let mut identities = std::collections::HashSet::new();
        for asset in &manifest.assets {
            if !valid_asset(asset)
                || !identities.insert((&asset.platform, &asset.arch, &asset.kind))
            {
                return Err(fail(
                    ErrorCode::Incompatible,
                    "Signed update asset is invalid or ambiguous",
                ));
            }
            for url in &asset.urls {
                self.validate_url(url)?;
            }
        }
        Ok(manifest)
    }

    pub async fn check(&self) -> Result<Manifest, AppError> {
        self.fetch_verified(true).await
    }

    pub(super) async fn check_release(&self) -> Result<VerifiedRelease, AppError> {
        self.fetch_verified_release(true).await
    }

    pub(super) fn verify_cached_release(
        &self,
        bytes: &[u8],
        signature: &str,
    ) -> Result<VerifiedRelease, AppError> {
        let manifest = self.verify_manifest_policy(bytes, signature, APP_VERSION, false)?;
        Ok(VerifiedRelease {
            manifest,
            raw_manifest: bytes.to_vec(),
            signature: signature.to_owned(),
        })
    }

    pub async fn check_available(&self) -> Result<Option<Manifest>, AppError> {
        let manifest = self.fetch_verified(false).await?;
        let target = Version::parse(&manifest.version)
            .map_err(|_| fail(ErrorCode::Incompatible, "Signed version is invalid"))?;
        let current = Version::parse(APP_VERSION)
            .map_err(|_| fail(ErrorCode::Internal, "Installed version is invalid"))?;
        Ok((target > current).then_some(manifest))
    }

    async fn fetch_verified(&self, require_newer: bool) -> Result<Manifest, AppError> {
        Ok(self.fetch_verified_release(require_newer).await?.manifest)
    }

    async fn fetch_verified_release(
        &self,
        require_newer: bool,
    ) -> Result<VerifiedRelease, AppError> {
        let client = http_client(Duration::from_secs(20))?;
        let manifest = fetch_small(&client, self.feed.clone(), MAX_MANIFEST_BYTES).await?;
        let mut signature_url = self.feed.clone();
        signature_url.set_path(&format!("{}.minisig", self.feed.path()));
        let signature = fetch_small(&client, signature_url, MAX_SIGNATURE_BYTES).await?;
        let signature = std::str::from_utf8(&signature).map_err(|_| {
            fail(
                ErrorCode::PermissionDenied,
                "Update signature encoding is invalid",
            )
        })?;
        let model =
            self.verify_manifest_policy(&manifest, signature, APP_VERSION, require_newer)?;
        Ok(VerifiedRelease {
            manifest: model,
            raw_manifest: manifest,
            signature: signature.to_owned(),
        })
    }

    pub async fn download(&self, asset: &Asset, directory: &Path) -> Result<PathBuf, AppError> {
        if !valid_asset(asset) {
            return Err(fail(
                ErrorCode::InvalidInput,
                "Invalid signed installer asset",
            ));
        }
        super::reject_links(directory)?;
        let destination = directory.join(&asset.filename);
        if destination.exists() {
            return Err(fail(
                ErrorCode::Busy,
                "An update staging file already exists",
            ));
        }
        let client = http_client(Duration::from_secs(300))?;
        // Mirrors are tried only from the signed manifest and compiled origins.
        for (index, value) in asset.urls.iter().enumerate() {
            let url = self.validate_url(value)?;
            let result = download_one(&client, url, asset, &destination).await;
            match result {
                Ok(()) => return Ok(destination),
                Err(error) => {
                    let _ = std::fs::remove_file(&destination);
                    if index + 1 == asset.urls.len() {
                        return Err(error);
                    }
                }
            }
        }
        Err(fail(
            ErrorCode::Unavailable,
            "No update mirror is available",
        ))
    }
}

impl Manifest {
    pub fn asset(&self, scope: Scope) -> Result<&Asset, AppError> {
        self.assets_for_install(scope)?
            .into_iter()
            .last()
            .ok_or_else(|| fail(ErrorCode::Unavailable, "No installer asset"))
    }

    pub fn assets_for_install(&self, scope: Scope) -> Result<Vec<&Asset>, AppError> {
        self.assets_for_target(scope, std::env::consts::OS, std::env::consts::ARCH)
    }

    fn assets_for_target(
        &self,
        scope: Scope,
        platform: &str,
        arch: &str,
    ) -> Result<Vec<&Asset>, AppError> {
        let kinds: &[&str] = match platform {
            "windows" => {
                if scope == Scope::System {
                    &["service-msi"]
                } else {
                    &["client-msi"]
                }
            }
            "linux" => {
                if scope == Scope::System {
                    &["core-deb"]
                } else {
                    &["core-deb", "gui-deb"]
                }
            }
            "macos" => {
                if scope == Scope::System {
                    &["service-pkg"]
                } else {
                    &["client-pkg"]
                }
            }
            _ => {
                return Err(fail(
                    ErrorCode::Unavailable,
                    "This update platform is unsupported",
                ))
            }
        };
        let mut selected = Vec::new();
        let mut filenames = std::collections::HashSet::new();
        for kind in kinds {
            let asset = self
                .assets
                .iter()
                .find(|asset| {
                    asset.platform == platform && asset.arch == arch && asset.kind == *kind
                })
                .ok_or_else(|| {
                    fail(
                    ErrorCode::Unavailable,
                    "The signed release has no installer for this platform, architecture and scope",
                )
                })?;
            if !filenames.insert(&asset.filename) {
                return Err(fail(
                    ErrorCode::Incompatible,
                    "Selected installer filenames must be unique",
                ));
            }
            selected.push(asset);
        }
        Ok(selected)
    }
}

fn https_url(value: &str) -> Result<Url, AppError> {
    let url =
        Url::parse(value).map_err(|_| fail(ErrorCode::InvalidInput, "Update URL is invalid"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(fail(
            ErrorCode::PermissionDenied,
            "Update URLs require HTTPS with no credentials or fragment",
        ));
    }
    Ok(url)
}
fn safe_filename(value: &str) -> bool {
    let stem = value.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && matches!(stem.as_bytes()[3], b'1'..=b'9'));
    value
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphanumeric)
        && value.len() <= 128
        && !value.contains("..")
        && !device
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}
fn valid_asset(asset: &Asset) -> bool {
    let format_matches = match (asset.platform.as_str(), asset.kind.as_str()) {
        ("windows", "client-msi" | "service-msi") => asset.filename.ends_with(".msi"),
        ("linux", "core-deb" | "gui-deb") => asset.filename.ends_with(".deb"),
        ("macos", "client-pkg" | "service-pkg") => asset.filename.ends_with(".pkg"),
        _ => false,
    };
    format_matches
        && ["x86", "x86_64", "aarch64"].contains(&asset.arch.as_str())
        && !(asset.platform == "macos" && asset.arch == "x86")
        && safe_filename(&asset.filename)
        && asset.bytes > 0
        && asset.bytes <= MAX_PACKAGE_BYTES
        && asset.sha256.len() == 64
        && asset.sha256.bytes().all(|v| v.is_ascii_hexdigit())
        && !asset.urls.is_empty()
        && asset.urls.len() <= 3
}
fn decode_key(value: &str) -> Result<PublicKey, AppError> {
    let result = if value.trim().contains('\n') {
        PublicKey::decode(value.trim())
    } else {
        PublicKey::from_base64(value.trim())
    };
    result.map_err(|_| {
        fail(
            ErrorCode::Unavailable,
            "The compiled release verification key is invalid",
        )
    })
}
fn http_client(timeout: Duration) -> Result<reqwest::Client, AppError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .https_only(true)
        .user_agent(concat!("BitNet/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| {
            fail(
                ErrorCode::Unavailable,
                "Could not initialize the HTTPS update client",
            )
        })
}
async fn fetch_small(client: &reqwest::Client, url: Url, max: usize) -> Result<Vec<u8>, AppError> {
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| fail(ErrorCode::Network, "Could not fetch update metadata"))?;
    if !response.status().is_success() {
        return Err(fail(
            ErrorCode::Network,
            "Update metadata request failed or redirected",
        ));
    }
    if response.content_length().is_some_and(|n| n > max as u64) {
        return Err(fail(
            ErrorCode::InvalidInput,
            "Update metadata is too large",
        ));
    }
    let mut stream = response.bytes_stream();
    let mut result = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|_| fail(ErrorCode::Network, "Update metadata transfer failed"))?;
        if chunk.len() > max - result.len() {
            return Err(fail(
                ErrorCode::InvalidInput,
                "Update metadata is too large",
            ));
        }
        result.extend_from_slice(&chunk);
    }
    Ok(result)
}
async fn download_one(
    client: &reqwest::Client,
    url: Url,
    asset: &Asset,
    destination: &Path,
) -> Result<(), AppError> {
    let response = client.get(url).send().await.map_err(|_| {
        fail(
            ErrorCode::Network,
            "Could not download the signed update asset",
        )
    })?;
    if !response.status().is_success() {
        return Err(fail(
            ErrorCode::Network,
            "Update asset request failed or redirected",
        ));
    }
    if response.content_length().is_some_and(|n| n != asset.bytes) {
        return Err(fail(
            ErrorCode::PermissionDenied,
            "Update asset length differs from its signed manifest",
        ));
    }
    let mut file = create_file(destination)?;
    let mut total = 0_u64;
    let mut hash = Sha256::new();
    let started = tokio::time::Instant::now();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| fail(ErrorCode::Network, "Update asset transfer failed"))?;
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| fail(ErrorCode::PermissionDenied, "Update asset length overflow"))?;
        if total > asset.bytes || total > MAX_PACKAGE_BYTES {
            return Err(fail(
                ErrorCode::PermissionDenied,
                "Update asset exceeds its signed size",
            ));
        }
        file.write_all(&chunk).map_err(|_| {
            fail(
                ErrorCode::Unavailable,
                "Could not write the update staging file",
            )
        })?;
        hash.update(&chunk);
        let budget = Duration::from_secs_f64(total as f64 / BYTES_PER_SECOND as f64);
        let pause = budget.saturating_sub(started.elapsed());
        if !pause.is_zero() {
            tokio::time::sleep(pause).await;
        }
    }
    file.sync_all().map_err(|_| {
        fail(
            ErrorCode::Unavailable,
            "Could not synchronize the update staging file",
        )
    })?;
    if total != asset.bytes || format!("{:x}", hash.finalize()) != asset.sha256.to_ascii_lowercase()
    {
        return Err(fail(
            ErrorCode::PermissionDenied,
            "Update asset integrity verification failed",
        ));
    }
    drop(file);
    verify_asset(destination, asset)
}

pub fn verify_asset(path: &Path, asset: &Asset) -> Result<(), AppError> {
    let (bytes, hash) = hash_file(path)?;
    if bytes != asset.bytes || hash != asset.sha256.to_ascii_lowercase() {
        return Err(fail(
            ErrorCode::PermissionDenied,
            "Staged installer differs from the signed manifest",
        ));
    }
    Ok(())
}

#[cfg(any(windows, target_os = "macos", test))]
pub(super) fn verify_offline_package(path: &Path) -> Result<(), AppError> {
    let key = option_env!("BITNET_RELEASE_PUBLIC_KEY")
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            fail(
                ErrorCode::Unavailable,
                "Offline installation is not provisioned: BITNET_RELEASE_PUBLIC_KEY is missing",
            )
        })?;
    verify_package_signature(
        path,
        &path.with_extension(format!(
            "{}.minisig",
            path.extension().and_then(|s| s.to_str()).unwrap_or("")
        )),
        key,
    )
}
#[cfg(any(windows, target_os = "macos", test))]
pub(super) fn verify_package_signature(
    path: &Path,
    signature_path: &Path,
    key: &str,
) -> Result<(), AppError> {
    super::reject_links(path)?;
    super::reject_links(signature_path)?;
    let signature_text = read_small_file(signature_path, MAX_SIGNATURE_BYTES)?;
    let signature_text = std::str::from_utf8(&signature_text).map_err(|_| {
        fail(
            ErrorCode::PermissionDenied,
            "Offline package signature encoding is invalid",
        )
    })?;
    let key = decode_key(key)?;
    let signature = Signature::decode(signature_text).map_err(|_| {
        fail(
            ErrorCode::PermissionDenied,
            "Offline package signature is invalid",
        )
    })?;
    let mut verifier = key.verify_stream(&signature).map_err(|_| {
        fail(
            ErrorCode::PermissionDenied,
            "Offline package must use a valid prehashed Minisign signature",
        )
    })?;
    let mut file = open_read(path)?;
    let mut buffer = [0; 64 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer).map_err(|_| {
            fail(
                ErrorCode::Unavailable,
                "Could not read the offline installer",
            )
        })?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_PACKAGE_BYTES {
            return Err(fail(
                ErrorCode::InvalidInput,
                "Offline installer is too large",
            ));
        }
        verifier.update(&buffer[..count]);
    }
    if total == 0 {
        return Err(fail(ErrorCode::InvalidInput, "Offline installer is empty"));
    }
    verifier.finalize().map_err(|_| {
        fail(
            ErrorCode::PermissionDenied,
            "Offline installer signature verification failed",
        )
    })
}

pub(super) fn hash_file(path: &Path) -> Result<(u64, String), AppError> {
    super::reject_links(path)?;
    let mut file = open_read(path)?;
    let mut hash = Sha256::new();
    let mut total = 0_u64;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|_| fail(ErrorCode::Unavailable, "Could not read a staged installer"))?;
        if count == 0 {
            break;
        }
        total += count as u64;
        if total > MAX_PACKAGE_BYTES {
            return Err(fail(
                ErrorCode::InvalidInput,
                "Installer exceeds its size limit",
            ));
        }
        hash.update(&buffer[..count]);
    }
    Ok((total, format!("{:x}", hash.finalize())))
}
pub(super) fn read_small_file(path: &Path, limit: usize) -> Result<Vec<u8>, AppError> {
    let mut file = open_read(path)?;
    let mut result = Vec::new();
    Read::by_ref(&mut file)
        .take(limit as u64 + 1)
        .read_to_end(&mut result)
        .map_err(|_| fail(ErrorCode::Unavailable, "Could not read installer metadata"))?;
    if result.len() > limit {
        return Err(fail(
            ErrorCode::InvalidInput,
            "Installer metadata exceeds its size limit",
        ));
    }
    Ok(result)
}
pub(super) fn open_read(path: &Path) -> Result<File, AppError> {
    super::reject_links(path)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    let file = options.open(path).map_err(|_| {
        fail(
            ErrorCode::Unavailable,
            "Could not open the installer resource",
        )
    })?;
    if !file
        .metadata()
        .map_err(|_| {
            fail(
                ErrorCode::Unavailable,
                "Could not inspect the installer resource",
            )
        })?
        .is_file()
    {
        return Err(fail(
            ErrorCode::PermissionDenied,
            "Installer resource is not a regular file",
        ));
    }
    Ok(file)
}
pub(super) fn create_file(path: &Path) -> Result<File, AppError> {
    super::reject_links(path)?;
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(0).custom_flags(0x8020_0000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    options.open(path).map_err(|_| {
        fail(
            ErrorCode::Unavailable,
            "Could not create a new installer staging file",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use blake2::Blake2b512;
    use ed25519_dalek::{Signer, SigningKey};

    pub(super) fn sign(data: &[u8]) -> (String, String) {
        // Test fixture key only. Never used as a production release default.
        let key = SigningKey::from_bytes(&[71; 32]);
        let mut public = b"Edtest-key".to_vec();
        public.extend_from_slice(key.verifying_key().as_bytes());
        let signature = key.sign(&Blake2b512::digest(data));
        let mut packet = b"EDtest-key".to_vec();
        packet.extend_from_slice(&signature.to_bytes());
        let comment = "timestamp:1700000000\tfile:synthetic-fixture\tprehashed";
        let mut global = signature.to_bytes().to_vec();
        global.extend_from_slice(comment.as_bytes());
        let global = key.sign(&global);
        (
            STANDARD.encode(public),
            format!(
                "untrusted comment: synthetic fixture\n{}\ntrusted comment: {comment}\n{}\n",
                STANDARD.encode(packet),
                STANDARD.encode(global.to_bytes())
            ),
        )
    }
    fn manifest(version: &str) -> Vec<u8> {
        serde_json::to_vec(&Manifest {
            schema_version: 1,
            api_version: 1,
            version: version.into(),
            assets: vec![Asset {
                platform: "windows".into(),
                arch: "x86_64".into(),
                kind: "client-msi".into(),
                filename: "Client.msi".into(),
                bytes: 4,
                sha256: format!("{:x}", Sha256::digest(b"test")),
                urls: vec!["https://updates.example.test/Client.msi".into()],
            }],
        })
        .unwrap()
    }

    #[test]
    fn signed_manifest_verifies_and_tampering_is_rejected() {
        let bytes = manifest("0.2.0");
        let (key, signature) = sign(&bytes);
        let config =
            UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
        assert_eq!(
            config
                .verify_manifest(&bytes, &signature, "0.1.0")
                .unwrap()
                .version,
            "0.2.0"
        );
        assert!(config
            .verify_manifest(&manifest("0.3.0"), &signature, "0.1.0")
            .is_err());
        assert!(config
            .verify_manifest(
                &bytes,
                &signature.replace("timestamp:1700000000", "timestamp:1700000001"),
                "0.1.0"
            )
            .is_err());
    }
    #[test]
    fn signed_downgrades_prereleases_and_foreign_urls_fail_closed() {
        for version in ["0.0.9", "0.1.0", "0.2.0-rc.1", "0.1.0+build.2"] {
            let bytes = manifest(version);
            let (key, signature) = sign(&bytes);
            let config =
                UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
            assert!(config.verify_manifest(&bytes, &signature, "0.1.0").is_err());
        }
        let mut model: Manifest = serde_json::from_slice(&manifest("0.2.0")).unwrap();
        model.assets[0].urls = vec!["https://foreign.example.test/Client.msi".into()];
        let bytes = serde_json::to_vec(&model).unwrap();
        let (key, signature) = sign(&bytes);
        let config =
            UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
        assert_eq!(
            config
                .verify_manifest(&bytes, &signature, "0.1.0")
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert!(config
            .validate_url("http://updates.example.test/file")
            .is_err());
        assert!(config
            .validate_url("https://user:pass@updates.example.test/file")
            .is_err());
    }

    #[test]
    fn passive_checks_allow_verified_current_metadata_but_never_relax_installation_policy() {
        let bytes = manifest("0.1.0");
        let (key, signature) = sign(&bytes);
        let config =
            UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
        assert!(config
            .verify_manifest_policy(&bytes, &signature, "0.1.0", false)
            .is_ok());
        assert!(config.verify_manifest(&bytes, &signature, "0.1.0").is_err());
        assert!(config
            .verify_manifest_policy(&manifest("0.2.0"), &signature, "0.1.0", false)
            .is_err());
    }
    #[test]
    fn incompatible_or_ambiguous_signed_manifests_are_rejected() {
        for change in 0..8 {
            let mut model: Manifest = serde_json::from_slice(&manifest("0.2.0")).unwrap();
            match change {
                0 => model.schema_version = 2,
                1 => model.assets.push(model.assets[0].clone()),
                2 => model.assets[0].filename = "../Client.msi".into(),
                3 => model.assets[0].bytes = MAX_PACKAGE_BYTES + 1,
                4 => model.assets[0].filename = "NUL.msi".into(),
                5 => model.assets[0].kind = "pkg".into(),
                6 => model.assets[0].filename = "Client.exe".into(),
                _ => model.assets[0].platform = "linux".into(),
            }
            let bytes = serde_json::to_vec(&model).unwrap();
            let (key, signature) = sign(&bytes);
            let config =
                UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
            assert!(config.verify_manifest(&bytes, &signature, "0.1.0").is_err());
        }
    }

    #[test]
    fn x86_and_linux_gui_dependencies_have_unambiguous_asset_selection() {
        let mut release: Manifest = serde_json::from_slice(&manifest("0.2.0")).unwrap();
        release.assets[0].arch = "x86".into();
        assert!(valid_asset(&release.assets[0]));
        assert_eq!(
            release
                .assets_for_target(Scope::User, "windows", "x86")
                .unwrap()[0]
                .kind,
            "client-msi"
        );
        assert!(release
            .assets_for_target(Scope::User, "windows", "x86_64")
            .is_err());
        release.assets[0].platform = "linux".into();
        release.assets[0].kind = "core-deb".into();
        release.assets[0].filename = "bitnet-core.deb".into();
        assert!(valid_asset(&release.assets[0]));
        assert_eq!(
            release
                .assets_for_target(Scope::System, "linux", "x86")
                .unwrap()
                .len(),
            1
        );
        assert!(release
            .assets_for_target(Scope::User, "linux", "x86")
            .is_err());
        let mut gui = release.assets[0].clone();
        gui.kind = "gui-deb".into();
        gui.filename = "bitnet-gui.deb".into();
        release.assets.push(gui);
        let selected = release
            .assets_for_target(Scope::User, "linux", "x86")
            .unwrap();
        assert_eq!(
            selected.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(),
            ["core-deb", "gui-deb"]
        );
        release.assets[1].filename = release.assets[0].filename.clone();
        assert!(release
            .assets_for_target(Scope::User, "linux", "x86")
            .is_err());
        release.assets[0].platform = "macos".into();
        release.assets[0].kind = "client-pkg".into();
        release.assets[0].filename = "BitNet.pkg".into();
        assert!(!valid_asset(&release.assets[0]));
        release.assets[0].arch = "aarch64".into();
        assert!(valid_asset(&release.assets[0]));
        assert_eq!(
            release
                .assets_for_target(Scope::User, "macos", "aarch64")
                .unwrap()[0]
                .kind,
            "client-pkg"
        );
        assert!(release
            .assets_for_target(Scope::System, "macos", "aarch64")
            .is_err());
    }

    #[test]
    fn staged_payload_and_offline_signature_are_independently_reverified() {
        let directory = super::super::tests::directory();
        let package = directory.0.join("Service.msi");
        let signature_path = directory.0.join("Service.msi.minisig");
        let payload = b"synthetic installer fixture, not an executable";
        let (key, signature) = sign(payload);
        std::fs::write(&package, payload).unwrap();
        std::fs::write(&signature_path, signature).unwrap();
        verify_package_signature(&package, &signature_path, &key).unwrap();
        let asset = Asset {
            platform: "windows".into(),
            arch: "x86_64".into(),
            kind: "service-msi".into(),
            filename: "Service.msi".into(),
            bytes: payload.len() as u64,
            sha256: format!("{:x}", Sha256::digest(payload)),
            urls: vec![],
        };
        verify_asset(&package, &asset).unwrap();
        std::fs::write(&package, b"changed fixture").unwrap();
        assert!(verify_asset(&package, &asset).is_err());
        assert!(verify_package_signature(&package, &signature_path, &key).is_err());
    }

    #[test]
    fn signature_identity_and_metadata_size_are_not_optional() {
        let bytes = manifest("0.2.0");
        let (key, signature) = sign(&bytes);
        let config =
            UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
        assert!(config
            .verify_manifest(&bytes, "not a signature", "0.1.0")
            .is_err());
        assert!(config
            .verify_manifest(&vec![b'x'; MAX_MANIFEST_BYTES + 1], &signature, "0.1.0")
            .is_err());
        assert!(UpdateConfig::new("http://updates.example.test/latest.json", &key, "").is_err());
        assert!(
            UpdateConfig::new("https://updates.example.test/latest.json", "not a key", "").is_err()
        );
    }

    async fn fixture_server(response: &'static [u8]) -> (Url, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut connection, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = tokio::io::AsyncReadExt::read(&mut connection, &mut request)
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::write_all(&mut connection, response)
                .await
                .unwrap();
            tokio::io::AsyncWriteExt::shutdown(&mut connection)
                .await
                .unwrap();
        });
        (
            Url::parse(&format!("http://{address}/fixture")).unwrap(),
            server,
        )
    }

    fn fixture_client() -> reqwest::Client {
        // HTTP exists only inside this private loopback fixture. Production
        // config and production clients unconditionally require HTTPS.
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn metadata_transport_rejects_redirects_and_declared_or_streamed_overflow() {
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n5\r\n12345\r\n0\r\n\r\n".as_slice(),
        ] {
            let (url, server) = fixture_server(response).await;
            assert!(fetch_small(&fixture_client(), url, 4).await.is_err());
            server.await.unwrap();
        }
        let (url, server) = fixture_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest",
        )
        .await;
        assert_eq!(
            fetch_small(&fixture_client(), url, 4).await.unwrap(),
            b"test"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn asset_transport_checks_size_hash_truncation_and_redirect_without_installing() {
        let directory = super::super::tests::directory();
        let asset = Asset {
            platform: "windows".into(),
            arch: "x86_64".into(),
            kind: "client-msi".into(),
            filename: "Client.msi".into(),
            bytes: 4,
            sha256: format!("{:x}", Sha256::digest(b"test")),
            urls: vec!["https://updates.example.test/Client.msi".into()],
        };
        for (index, response) in [
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 8\r\nConnection: close\r\n\r\ntoolarge".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nbad!".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ncut".as_slice(),
        ].into_iter().enumerate() {
            let (url, server) = fixture_server(response).await;
            assert!(download_one(&fixture_client(), url, &asset, &directory.0.join(format!("rejected-{index}.msi"))).await.is_err());
            server.await.unwrap();
        }
        let (url, server) = fixture_server(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ntest",
        )
        .await;
        let target = directory.0.join("Client.msi");
        download_one(&fixture_client(), url, &asset, &target)
            .await
            .unwrap();
        server.await.unwrap();
        verify_asset(&target, &asset).unwrap();
    }
}
