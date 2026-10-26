use crate::{error, io_error, random, reject_link};
use bitnet_core::{AppError, ErrorCode, Scope};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

pub(crate) const MAX_STORE_BYTES: u64 = 1024 * 1024;

pub(crate) fn prepare_directory(path: &Path, scope: Scope) -> Result<(), AppError> {
    if !path.is_absolute() {
        return Err(error(
            ErrorCode::InvalidInput,
            "Credential directory must be absolute",
        ));
    }
    // Inspect existing components before creating a missing child. Never secure a linked target.
    for p in path.ancestors() {
        reject_link(p)?;
    }
    fs::create_dir_all(path).map_err(io_error)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if fs::metadata(path).map_err(io_error)?.uid() != unsafe { libc::geteuid() } {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Credential directory belongs to another identity",
            ));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    }
    #[cfg(windows)]
    crate::windows::restrict_acl(path, true, scope)?;
    let _ = scope;
    Ok(())
}

pub(crate) fn read(path: &Path) -> Result<Option<Vec<u8>>, AppError> {
    reject_link(path)?;
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io_error(e)),
    };
    let m = f.metadata().map_err(io_error)?;
    if !m.is_file() || m.len() > MAX_STORE_BYTES {
        return Err(error(
            ErrorCode::CredentialsUnavailable,
            "Credential data is invalid or too large",
        ));
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut f)
        .take(MAX_STORE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(error(
            ErrorCode::CredentialsUnavailable,
            "Credential data is too large",
        ));
    }
    Ok(Some(bytes))
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8], scope: Scope) -> Result<(), AppError> {
    if bytes.len() as u64 > MAX_STORE_BYTES {
        return Err(error(
            ErrorCode::InvalidInput,
            "Credential store is too large",
        ));
    }
    reject_link(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| error(ErrorCode::InvalidInput, "Invalid credential path"))?;
    let suffix = random::<16>()?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let tmp = parent.join(format!(".accounts-{suffix}.tmp"));
    let mut opt = OpenOptions::new();
    opt.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opt.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let result = (|| {
        let mut f = opt.open(&tmp).map_err(io_error)?;
        #[cfg(windows)]
        crate::windows::restrict_acl(&tmp, false, scope)?;
        f.write_all(bytes).map_err(io_error)?;
        f.sync_all().map_err(io_error)?;
        drop(f);
        #[cfg(windows)]
        crate::windows::replace_file(&tmp, path)?;
        #[cfg(not(windows))]
        fs::rename(&tmp, path).map_err(io_error)?;
        #[cfg(unix)]
        fs::File::open(parent)
            .and_then(|d| d.sync_all())
            .map_err(io_error)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    let _ = scope;
    result
}
