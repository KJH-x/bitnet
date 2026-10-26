//! Local, signature-authenticated previous-release cache. Never downloads and
//! never runs an installer. The maintenance entry point owns authorization.
use super::{read_small_file, verify_asset, UpdateConfig, VerifiedRelease};
use bitnet_core::{AppError, ErrorCode, Scope};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub(crate) const MAX_CACHE_BYTES: u64 = 1024 * 1024 * 1024;
const INDEX_ALLOWANCE: u64 = 4096;
static NEXT: AtomicU64 = AtomicU64::new(0);
fn error(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Index {
    schema_version: u32,
    scope: Scope,
    installed_version: String,
    active: Option<String>,
    previous: Option<String>,
}

pub(crate) struct Cache {
    root: PathBuf,
    target: PathBuf,
    scope: Scope,
}
pub(crate) struct StoredRelease {
    pub release: VerifiedRelease,
    pub packages: Vec<PathBuf>,
    generation: String,
}
#[derive(Debug, Serialize)]
pub(crate) struct CacheStatus {
    pub active_cached: bool,
    pub previous_available: bool,
    pub skipped_limit: bool,
}

impl Cache {
    pub(crate) fn open(staging_root: &Path, scope: Scope, system: bool) -> Result<Self, AppError> {
        let root = staging_root.join("verified-release-cache");
        super::super::secure_directory(&root, system)?;
        let target = root.join(if scope == Scope::System {
            "system"
        } else {
            "user"
        });
        super::super::secure_directory(&target, system)?;
        Ok(Self {
            root,
            target,
            scope,
        })
    }

    fn empty_index(&self, current: &str) -> Index {
        Index {
            schema_version: 1,
            scope: self.scope,
            installed_version: current.into(),
            active: None,
            previous: None,
        }
    }
    fn read_index(&self, current: &str) -> Result<Index, AppError> {
        let path = self.target.join("index.json");
        if !path.try_exists().map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Could not inspect the release cache",
            )
        })? {
            return Ok(self.empty_index(current));
        }
        let bytes = read_small_file(&path, INDEX_ALLOWANCE as usize)?;
        let index: Index = serde_json::from_slice(&bytes).map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "The local release-cache index is invalid",
            )
        })?;
        if index.schema_version != 1
            || index.scope != self.scope
            || index
                .active
                .iter()
                .chain(index.previous.iter())
                .any(|id| !safe_generation(id))
            || (index.active.is_some() && index.active == index.previous)
        {
            return Err(error(
                ErrorCode::Incompatible,
                "The local release-cache index is incompatible",
            ));
        }
        Ok(index)
    }
    fn write_index(&self, index: &Index) -> Result<(), AppError> {
        let path = self.target.join("index.json");
        super::super::reject_links(&path)?;
        let bytes = serde_json::to_vec(index).map_err(|_| {
            error(
                ErrorCode::Internal,
                "Could not encode the release-cache index",
            )
        })?;
        crate::persistence::atomic_write(&path, &bytes)
    }
    fn generation(&self, id: &str) -> Result<PathBuf, AppError> {
        if !safe_generation(id) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Invalid release-cache generation",
            ));
        }
        let path = self.target.join(id);
        super::super::reject_links(&path)?;
        Ok(path)
    }
    fn load_generation(&self, config: &UpdateConfig, id: &str) -> Result<StoredRelease, AppError> {
        let directory = self.generation(id)?;
        let raw = read_small_file(&directory.join("manifest.json"), super::MAX_MANIFEST_BYTES)?;
        let signature = read_small_file(
            &directory.join("manifest.json.minisig"),
            super::MAX_SIGNATURE_BYTES,
        )?;
        let signature = std::str::from_utf8(&signature).map_err(|_| {
            error(
                ErrorCode::PermissionDenied,
                "Cached release signature is invalid",
            )
        })?;
        let release = config.verify_cached_release(&raw, signature)?;
        let packages = release
            .manifest
            .assets_for_install(self.scope)?
            .into_iter()
            .map(|asset| {
                let package = directory.join(&asset.filename);
                verify_asset(&package, asset)?;
                Ok(package)
            })
            .collect::<Result<Vec<_>, AppError>>()?;
        Ok(StoredRelease {
            release,
            packages,
            generation: id.into(),
        })
    }

    pub(crate) fn previous(
        &self,
        config: &UpdateConfig,
        current: &str,
    ) -> Result<StoredRelease, AppError> {
        let index = self.read_index(current)?;
        if index.installed_version != current {
            return Err(error(
                ErrorCode::Unavailable,
                "No previous verified cache matches this installed version",
            ));
        }
        let id = index.previous.ok_or_else(|| {
            error(
                ErrorCode::Unavailable,
                "No previous verified version is cached on this computer",
            )
        })?;
        let previous = self.load_generation(config, &id)?;
        let target = semver::Version::parse(&previous.release.manifest.version)
            .map_err(|_| error(ErrorCode::Incompatible, "Cached version is invalid"))?;
        let current = semver::Version::parse(current)
            .map_err(|_| error(ErrorCode::Internal, "Installed version is invalid"))?;
        if target >= current {
            return Err(error(
                ErrorCode::Unavailable,
                "Rollback requires an older compatible locally cached version",
            ));
        }
        Ok(previous)
    }

    pub(crate) fn record_update(
        &self,
        config: &UpdateConfig,
        release: &VerifiedRelease,
        packages: &[PathBuf],
        current: &str,
    ) -> Result<CacheStatus, AppError> {
        self.record_with_limit(config, release, packages, current, MAX_CACHE_BYTES)
    }
    fn record_with_limit(
        &self,
        config: &UpdateConfig,
        release: &VerifiedRelease,
        packages: &[PathBuf],
        current: &str,
        limit: u64,
    ) -> Result<CacheStatus, AppError> {
        // Recheck all metadata, not just the in-memory parsed model.
        let verified = config.verify_cached_release(&release.raw_manifest, &release.signature)?;
        let target_version = semver::Version::parse(&verified.manifest.version)
            .map_err(|_| error(ErrorCode::Incompatible, "Cached release version is invalid"))?;
        let current_version = semver::Version::parse(current)
            .map_err(|_| error(ErrorCode::Internal, "Installed release version is invalid"))?;
        if target_version <= current_version {
            return Err(error(
                ErrorCode::Unavailable,
                "Normal update cache requires a newer release",
            ));
        }
        let assets = verified.manifest.assets_for_install(self.scope)?;
        if assets.len() != packages.len() {
            return Err(error(
                ErrorCode::InvalidInput,
                "Incomplete cache installer set",
            ));
        }
        for (asset, package) in assets.iter().zip(packages) {
            verify_asset(package, asset)?;
        }
        let index = self.read_index(current)?;
        let previous = if index.installed_version == current {
            index
                .active
                .as_ref()
                .map(|id| self.load_generation(config, id))
                .transpose()?
                .filter(|stored| stored.release.manifest.version == current)
        } else {
            None
        };
        let previous_id = previous.as_ref().map(|stored| stored.generation.clone());
        let previous_bytes = previous_id
            .as_ref()
            .map(|id| tree_size(&self.generation(id)?, 0))
            .transpose()?
            .unwrap_or(0);
        let root_bytes = tree_size(&self.root, 0)?;
        let target_bytes = tree_size(&self.target, 0)?;
        let incoming_bytes = assets.iter().try_fold(
            (release.raw_manifest.len() + release.signature.len()) as u64,
            |sum, asset| {
                sum.checked_add(asset.bytes)
                    .ok_or_else(|| error(ErrorCode::InvalidInput, "Cache size overflow"))
            },
        )?;
        let projected = root_bytes
            .saturating_sub(target_bytes)
            .saturating_add(previous_bytes)
            .saturating_add(incoming_bytes)
            .saturating_add(INDEX_ALLOWANCE);
        // Delete only old, non-retained generations in this scope. The other
        // scope participates in the same root budget but is never evicted here.
        self.prune(previous_id.as_deref(), None)?;
        if projected > limit {
            self.write_index(&Index {
                installed_version: verified.manifest.version,
                previous: previous_id,
                ..self.empty_index(current)
            })?;
            return Ok(CacheStatus {
                active_cached: false,
                previous_available: previous.is_some(),
                skipped_limit: true,
            });
        }
        let id = format!(
            "release-{}-{}-{}",
            bitnet_core::now_ms(),
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let directory = self.generation(&id)?;
        fs::create_dir(&directory).map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Could not create the verified release cache",
            )
        })?;
        let staged = (|| {
            write_new(&directory.join("manifest.json"), &release.raw_manifest)?;
            write_new(
                &directory.join("manifest.json.minisig"),
                release.signature.as_bytes(),
            )?;
            for (asset, source) in assets.iter().zip(packages) {
                let destination = directory.join(&asset.filename);
                copy_package(source, &destination)?;
                verify_asset(&destination, asset)?;
            }
            self.write_index(&Index {
                schema_version: 1,
                scope: self.scope,
                installed_version: verified.manifest.version.clone(),
                active: Some(id.clone()),
                previous: previous_id,
            })
        })();
        if let Err(failure) = staged {
            let _ = self.remove_generation(&id);
            return Err(failure);
        }
        Ok(CacheStatus {
            active_cached: true,
            previous_available: previous.is_some(),
            skipped_limit: false,
        })
    }

    pub(crate) fn record_rollback(
        &self,
        stored: &StoredRelease,
        current: &str,
    ) -> Result<(), AppError> {
        let index = self.read_index(current)?;
        if index.installed_version != current
            || index.previous.as_deref() != Some(&stored.generation)
        {
            return Err(error(
                ErrorCode::Busy,
                "The previous-version cache changed during rollback",
            ));
        }
        self.write_index(&Index {
            schema_version: 1,
            scope: self.scope,
            installed_version: stored.release.manifest.version.clone(),
            active: Some(stored.generation.clone()),
            previous: index.active,
        })
    }

    fn prune(&self, keep_a: Option<&str>, keep_b: Option<&str>) -> Result<(), AppError> {
        for entry in fs::read_dir(&self.target).map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Could not inspect cache generations",
            )
        })? {
            let entry = entry.map_err(|_| {
                error(
                    ErrorCode::Unavailable,
                    "Could not inspect a cache generation",
                )
            })?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return Err(error(ErrorCode::PermissionDenied, "Invalid cache entry"));
            };
            if name == "index.json" {
                continue;
            }
            if name
                .strip_prefix("index.")
                .and_then(|s| s.strip_suffix(".tmp"))
                .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|c| c.is_ascii_digit()))
            {
                let temporary = entry.path();
                super::super::reject_links(&temporary)?;
                let metadata = fs::symlink_metadata(&temporary).map_err(|_| {
                    error(
                        ErrorCode::Unavailable,
                        "Could not inspect a stale cache index",
                    )
                })?;
                if !metadata.is_file() || metadata.len() > INDEX_ALLOWANCE {
                    return Err(error(
                        ErrorCode::PermissionDenied,
                        "Unexpected cache index temporary file",
                    ));
                }
                fs::remove_file(&temporary).map_err(|_| {
                    error(
                        ErrorCode::Unavailable,
                        "Could not remove a stale cache index",
                    )
                })?;
                continue;
            }
            if !safe_generation(name) {
                return Err(error(
                    ErrorCode::PermissionDenied,
                    "Unexpected entry in the release cache",
                ));
            }
            if Some(name) != keep_a && Some(name) != keep_b {
                self.remove_generation(name)?;
            }
        }
        Ok(())
    }
    fn remove_generation(&self, id: &str) -> Result<(), AppError> {
        let path = self.generation(id)?;
        let target = self.target.canonicalize().map_err(|_| {
            error(
                ErrorCode::PermissionDenied,
                "Could not resolve the cache root",
            )
        })?;
        let resolved = path.canonicalize().map_err(|_| {
            error(
                ErrorCode::PermissionDenied,
                "Could not resolve the cache generation",
            )
        })?;
        if resolved.parent() != Some(target.as_path()) || !resolved.starts_with(&target) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Refusing cache cleanup outside its fixed root",
            ));
        }
        // The depth-limited walk rejects links before any recursive removal.
        tree_size(&resolved, 0)?;
        fs::remove_dir_all(resolved).map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Could not remove an obsolete cache generation",
            )
        })
    }
}

fn safe_generation(value: &str) -> bool {
    value.starts_with("release-")
        && value.len() <= 96
        && value.len() > 8
        && value[8..].bytes().all(|b| b.is_ascii_digit() || b == b'-')
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let mut file = super::create_file(path)?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| {
            error(
                ErrorCode::Unavailable,
                "Could not write the verified release cache",
            )
        })
}
fn copy_package(source: &Path, destination: &Path) -> Result<(), AppError> {
    let mut input = super::open_read(source)?;
    let mut output = super::create_file(destination)?;
    let copied = std::io::copy(
        &mut Read::by_ref(&mut input).take(super::MAX_PACKAGE_BYTES + 1),
        &mut output,
    )
    .map_err(|_| {
        error(
            ErrorCode::Unavailable,
            "Could not copy a verified installer",
        )
    })?;
    if copied == 0 || copied > super::MAX_PACKAGE_BYTES {
        return Err(error(
            ErrorCode::InvalidInput,
            "Cached installer exceeds its size limit",
        ));
    }
    output.sync_all().map_err(|_| {
        error(
            ErrorCode::Unavailable,
            "Could not synchronize a cached installer",
        )
    })
}
fn tree_size(path: &Path, depth: usize) -> Result<u64, AppError> {
    if depth > 5 {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Cache directory nesting is invalid",
        ));
    }
    super::super::reject_links(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| error(ErrorCode::Unavailable, "Could not inspect cache storage"))?;
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Cache entry is not a regular file or directory",
        ));
    }
    let mut sum = 0_u64;
    for (count, entry) in fs::read_dir(path)
        .map_err(|_| error(ErrorCode::Unavailable, "Could not enumerate cache storage"))?
        .enumerate()
    {
        if count >= 256 {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Too many entries in release cache",
            ));
        }
        let entry =
            entry.map_err(|_| error(ErrorCode::Unavailable, "Could not inspect a cache entry"))?;
        sum = sum
            .checked_add(tree_size(&entry.path(), depth + 1)?)
            .ok_or_else(|| error(ErrorCode::InvalidInput, "Cache size overflow"))?;
    }
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn fixture(directory: &Path, version: &str) -> (UpdateConfig, VerifiedRelease, Vec<PathBuf>) {
        fixture_for(directory, version, Scope::User)
    }
    fn fixture_for(
        directory: &Path,
        version: &str,
        scope: Scope,
    ) -> (UpdateConfig, VerifiedRelease, Vec<PathBuf>) {
        let platform = std::env::consts::OS;
        let (kinds, extension) = match (platform, scope) {
            ("windows", Scope::User) => (vec!["client-msi"], "msi"),
            ("windows", Scope::System) => (vec!["service-msi"], "msi"),
            ("macos", Scope::User) => (vec!["client-pkg"], "pkg"),
            ("macos", Scope::System) => (vec!["service-pkg"], "pkg"),
            (_, Scope::User) => (vec!["core-deb", "gui-deb"], "deb"),
            (_, Scope::System) => (vec!["core-deb"], "deb"),
        };
        let mut assets = Vec::new();
        let mut packages = Vec::new();
        for kind in kinds {
            let filename = format!("{kind}-{version}.{extension}");
            let payload = format!("synthetic signed package {kind} {version}, never executed");
            let path = directory.join(&filename);
            fs::write(&path, &payload).unwrap();
            packages.push(path);
            assets.push(super::super::Asset {
                platform: platform.into(),
                arch: std::env::consts::ARCH.into(),
                kind: kind.into(),
                filename: filename.clone(),
                bytes: payload.len() as u64,
                sha256: format!("{:x}", Sha256::digest(payload.as_bytes())),
                urls: vec![format!("https://updates.example.test/{filename}")],
            });
        }
        let model = super::super::Manifest {
            schema_version: 1,
            api_version: 1,
            version: version.into(),
            assets,
        };
        let bytes = serde_json::to_vec(&model).unwrap();
        let (key, signature) = super::super::tests::sign(&bytes);
        let config =
            UpdateConfig::new("https://updates.example.test/latest.json", &key, "").unwrap();
        let release = config.verify_cached_release(&bytes, &signature).unwrap();
        (config, release, packages)
    }
    fn cache(directory: &Path) -> Cache {
        cache_for(directory, Scope::User)
    }
    fn cache_for(directory: &Path, scope: Scope) -> Cache {
        let root = directory.join("cache");
        let target = root.join(if scope == Scope::User {
            "user"
        } else {
            "system"
        });
        fs::create_dir_all(&target).unwrap();
        Cache {
            root,
            target,
            scope,
        }
    }

    #[test]
    fn only_an_exact_previous_signed_install_can_be_restored() {
        let directory = super::super::super::tests::directory();
        let cache = cache(&directory.0);
        let (config, first, packages) = fixture(&directory.0, "0.2.0");
        assert!(cache.previous(&config, "0.1.0").is_err());
        assert!(
            !cache
                .record_update(&config, &first, &packages, "0.1.0")
                .unwrap()
                .previous_available
        );
        let (config, second, packages) = fixture(&directory.0, "0.3.0");
        cache
            .record_update(&config, &second, &packages, "0.2.0")
            .unwrap();
        let stored = cache.previous(&config, "0.3.0").unwrap();
        assert_eq!(stored.release.manifest.version, "0.2.0");
        assert!(cache.previous(&config, "0.4.0").is_err());
        assert!(cache
            .record_update(&config, &second, &packages, "0.3.0")
            .is_err());
        cache.record_rollback(&stored, "0.3.0").unwrap();
        assert!(cache.previous(&config, "0.2.0").is_err());
    }

    #[test]
    fn tampered_manifest_packages_and_cross_scope_index_are_rejected() {
        let directory = super::super::super::tests::directory();
        let cache = cache(&directory.0);
        let (config, first, packages) = fixture(&directory.0, "0.2.0");
        cache
            .record_update(&config, &first, &packages, "0.1.0")
            .unwrap();
        let (config, mut second, packages) = fixture(&directory.0, "0.3.0");
        second.raw_manifest[0] = b'!';
        assert!(cache
            .record_update(&config, &second, &packages, "0.2.0")
            .is_err());
        let (config, second, packages) = fixture(&directory.0, "0.3.0");
        cache
            .record_update(&config, &second, &packages, "0.2.0")
            .unwrap();
        let stored = cache.previous(&config, "0.3.0").unwrap();
        fs::write(&stored.packages[0], b"tampered").unwrap();
        assert!(cache.previous(&config, "0.3.0").is_err());
        let other = Cache {
            root: cache.root.clone(),
            target: cache.target.clone(),
            scope: Scope::System,
        };
        assert!(other.previous(&config, "0.3.0").is_err());
    }

    #[test]
    fn signed_incompatible_schema_or_api_is_rejected_before_any_installer() {
        let directory = super::super::super::tests::directory();
        let (config, release, _) = fixture(&directory.0, "0.2.0");
        for schema in [true, false] {
            let mut model: super::super::Manifest =
                serde_json::from_slice(&release.raw_manifest).unwrap();
            if schema {
                model.schema_version += 1;
            } else {
                model.api_version += 1;
            }
            let bytes = serde_json::to_vec(&model).unwrap();
            let (_, signature) = super::super::tests::sign(&bytes);
            assert_eq!(
                config
                    .verify_cached_release(&bytes, &signature)
                    .err()
                    .unwrap()
                    .code,
                ErrorCode::Incompatible
            );
        }
    }

    #[test]
    fn cache_budget_keeps_two_generations_and_reports_skipped_retention() {
        let directory = super::super::super::tests::directory();
        let cache = cache(&directory.0);
        let stale_index = cache.target.join("index.999999.tmp");
        fs::write(
            &stale_index,
            b"incomplete index from an interrupted cache write",
        )
        .unwrap();
        let orphan = cache.target.join("release-0-0");
        fs::create_dir(&orphan).unwrap();
        fs::write(
            orphan.join("unused"),
            b"incomplete generation, never executed",
        )
        .unwrap();
        for (current, next) in [("0.1.0", "0.2.0"), ("0.2.0", "0.3.0"), ("0.3.0", "0.4.0")] {
            let (config, release, packages) = fixture(&directory.0, next);
            cache
                .record_update(&config, &release, &packages, current)
                .unwrap();
        }
        assert_eq!(
            fs::read_dir(&cache.target)
                .unwrap()
                .filter(|e| e.as_ref().unwrap().file_type().unwrap().is_dir())
                .count(),
            2
        );
        assert!(!stale_index.exists());
        assert!(!orphan.exists());
        let index = cache.read_index("0.4.0").unwrap();
        let retained_bytes = tree_size(
            &cache.generation(index.active.as_deref().unwrap()).unwrap(),
            0,
        )
        .unwrap();
        let limit = retained_bytes + INDEX_ALLOWANCE;
        let (config, release, packages) = fixture(&directory.0, "0.5.0");
        let status = cache
            .record_with_limit(&config, &release, &packages, "0.4.0", limit)
            .unwrap();
        assert!(status.skipped_limit);
        assert!(status.previous_available);
        assert!(!status.active_cached);
        assert!(tree_size(&cache.root, 0).unwrap() <= limit);
        assert_eq!(
            cache
                .previous(&config, "0.5.0")
                .unwrap()
                .release
                .manifest
                .version,
            "0.4.0"
        );
    }

    #[test]
    fn scopes_share_the_root_budget_but_cannot_evict_each_others_cache() {
        let directory = super::super::super::tests::directory();
        let user = cache_for(&directory.0, Scope::User);
        let system = cache_for(&directory.0, Scope::System);
        let (config, release, packages) = fixture(&directory.0, "0.2.0");
        user.record_update(&config, &release, &packages, "0.1.0")
            .unwrap();
        let user_before = tree_size(&user.target, 0).unwrap();
        let (config, release, packages) = fixture_for(&directory.0, "0.2.0", Scope::System);
        let limit = user_before + INDEX_ALLOWANCE;
        assert!(
            system
                .record_with_limit(&config, &release, &packages, "0.1.0", limit)
                .unwrap()
                .skipped_limit
        );
        assert_eq!(tree_size(&user.target, 0).unwrap(), user_before);
        assert!(tree_size(&user.root, 0).unwrap() <= limit);
    }

    #[test]
    fn invalid_cache_index_never_resolves_a_path_outside_the_fixed_root() {
        let directory = super::super::super::tests::directory();
        let cache = cache(&directory.0);
        let (config, _, _) = fixture(&directory.0, "0.2.0");
        assert!(cache.generation("../outside").is_err());
        assert!(cache.generation("release-../../outside").is_err());
        let index = Index {
            schema_version: 1,
            scope: Scope::User,
            installed_version: "0.3.0".into(),
            active: None,
            previous: Some("../outside".into()),
        };
        fs::write(
            cache.target.join("index.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        assert!(cache.previous(&config, "0.3.0").is_err());
    }
}
