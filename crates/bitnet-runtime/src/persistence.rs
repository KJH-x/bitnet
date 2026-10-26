use bitnet_core::{AppError, ErrorCode, Settings};
use std::{
    fs,
    io::{Read, Write},
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

pub fn read_settings(path: &Path) -> Result<Settings, AppError> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(1024 * 1024 + 1).read_to_end(&mut bytes))
        .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法读取设置文件"))?;
    if bytes.len() > 1024 * 1024 {
        return Err(AppError::new(ErrorCode::InvalidInput, "设置文件过大"));
    }
    let value: Settings = serde_json::from_slice(&bytes).map_err(|_| {
        AppError::new(
            ErrorCode::InvalidInput,
            "设置文件损坏；原文件已保留，请修复或导入备份",
        )
    })?;
    value.validate()?;
    Ok(value)
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "无效的保存位置"))?;
    fs::create_dir_all(parent)
        .map_err(|_| AppError::new(ErrorCode::PermissionDenied, "无法创建数据目录"))?;
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
    let temporary = path.with_extension(format!(
        "{}-{}-{}.tmp",
        std::process::id(),
        bitnet_core::now_ms(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法创建原子保存文件"))?;
    let result = (|| {
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| AppError::new(ErrorCode::Unavailable, "保存数据失败"))?;
        drop(file);
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            let from = temporary
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let to = path
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            if unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    from.as_ptr(),
                    to.as_ptr(),
                    windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
                        | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
                )
            } == 0
            {
                return Err(AppError::new(ErrorCode::Unavailable, "原子替换数据失败"));
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temporary, path)
                .map_err(|_| AppError::new(ErrorCode::Unavailable, "原子替换数据失败"))?;
            fs::File::open(parent)
                .and_then(|d| d.sync_all())
                .map_err(|_| AppError::new(ErrorCode::Unavailable, "无法同步数据目录"))?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn write_settings(path: &Path, settings: &Settings) -> Result<(), AppError> {
    settings.validate()?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| AppError::new(ErrorCode::Internal, "设置编码失败"))?;
    atomic_write(path, &bytes)
}
