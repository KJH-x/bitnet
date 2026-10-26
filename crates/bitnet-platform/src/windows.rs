use crate::{error, io_error, WINDOWS_SERVICE_NAME};
use bitnet_core::{AppError, ErrorCode, Scope};
use std::{ffi::c_void, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::*,
    Security::{Authorization::*, *},
    System::Threading::*,
};

pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub(crate) unsafe fn wide_string(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0;
    while *p.add(len) != 0 && len < 32768 {
        len += 1;
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

pub(crate) fn current_sid() -> Result<String, AppError> {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let mut needed = 0;
        GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut needed);
        let mut storage = vec![0u64; (needed as usize).div_ceil(8)];
        let ok = GetTokenInformation(
            token,
            TokenUser,
            storage.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        CloseHandle(token);
        if ok == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let user = &*storage.as_ptr().cast::<TOKEN_USER>();
        let mut text = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
            return Err(io_error(std::io::Error::last_os_error()));
        }
        let s = wide_string(text);
        LocalFree(text.cast());
        Ok(s)
    }
}

pub(crate) fn ensure_service_identity(scope: Scope) -> Result<(), AppError> {
    if scope == Scope::User {
        return Ok(());
    }
    unsafe {
        let account = wide(&format!("NT SERVICE\\{WINDOWS_SERVICE_NAME}"));
        let (mut sidlen, mut domlen, mut use_type) = (0, 0, 0);
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut sidlen,
            ptr::null_mut(),
            &mut domlen,
            &mut use_type,
        );
        if sidlen == 0 {
            return Err(error(
                ErrorCode::CredentialsUnavailable,
                "Service identity is not provisioned",
            ));
        }
        let mut sid = vec![0u64; (sidlen as usize).div_ceil(8)];
        let mut domain = vec![0u16; domlen as usize];
        if LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sidlen,
            domain.as_mut_ptr(),
            &mut domlen,
            &mut use_type,
        ) == 0
        {
            return Err(error(
                ErrorCode::CredentialsUnavailable,
                "Cannot resolve the service identity",
            ));
        }
        let mut service_sid = ptr::null_mut();
        if ConvertSidToStringSidW(sid.as_ptr() as *mut c_void, &mut service_sid) == 0 {
            return Err(error(
                ErrorCode::CredentialsUnavailable,
                "Cannot resolve service identity",
            ));
        }
        let expected = wide_string(service_sid);
        LocalFree(service_sid.cast());
        if current_sid()? != expected {
            return Err(error(
                ErrorCode::PermissionDenied,
                "System credentials may only be opened by the BitNet virtual service identity",
            ));
        }
    }
    Ok(())
}

pub(crate) fn restrict_acl(path: &Path, directory: bool, scope: Scope) -> Result<(), AppError> {
    ensure_service_identity(scope)?;
    let sid = current_sid()?;
    let flags = if directory { "OICI" } else { "" };
    let sddl = wide(&format!(
        "D:P(A;{flags};FA;;;SY)(A;{flags};FA;;;BA)(A;{flags};FA;;;{sid})"
    ));
    unsafe {
        let mut sd = ptr::null_mut();
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut sd,
            ptr::null_mut(),
        ) == 0
        {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Cannot build a private credential ACL",
            ));
        }
        let (mut present, mut defaulted, mut dacl) = (0, 0, ptr::null_mut());
        let ok = GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted);
        let path = wide(&path.to_string_lossy());
        let result = if ok == 0 || present == 0 {
            1
        } else {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                dacl,
                ptr::null(),
            )
        };
        LocalFree(sd);
        if result != 0 {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Cannot restrict credential access; refusing to save",
            ));
        }
    }
    Ok(())
}

pub(crate) fn replace_file(from: &Path, to: &Path) -> Result<(), AppError> {
    use windows_sys::Win32::Storage::FileSystem::*;
    let from = wide(&from.to_string_lossy());
    let to = wide(&to.to_string_lossy());
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io_error(std::io::Error::last_os_error()));
    }
    Ok(())
}

pub(crate) fn dpapi(input: &[u8], protect: bool) -> Result<Vec<u8>, AppError> {
    use windows_sys::Win32::Security::Cryptography::*;
    if input.len() > u32::MAX as usize {
        return Err(error(
            ErrorCode::InvalidInput,
            "Credential data exceeds DPAPI limit",
        ));
    }
    let data = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let entropy_bytes = b"BitNet/accounts/v1/current-identity";
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: entropy_bytes.len() as u32,
        pbData: entropy_bytes.as_ptr() as *mut u8,
    };
    unsafe {
        let mut output: CRYPT_INTEGER_BLOB = std::mem::zeroed();
        let ok = if protect {
            CryptProtectData(
                &data,
                ptr::null(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &data,
                ptr::null_mut(),
                &entropy,
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(error(
                ErrorCode::CredentialsUnavailable,
                "DPAPI could not protect or unlock credentials for this identity",
            ));
        }
        let result = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        // DPAPI owns this buffer; erase plaintext before returning it to the local allocator.
        if !protect {
            zeroize::Zeroize::zeroize(std::slice::from_raw_parts_mut(
                output.pbData,
                output.cbData as usize,
            ));
        }
        LocalFree(output.pbData.cast());
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_acl_is_protected_and_has_no_general_users() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target");
        let temp = tempfile::tempdir_in(root).unwrap();
        restrict_acl(temp.path(), true, Scope::User).unwrap();
        unsafe {
            let path = wide(&temp.path().to_string_lossy());
            let mut sd = ptr::null_mut();
            assert_eq!(
                GetNamedSecurityInfoW(
                    path.as_ptr(),
                    SE_FILE_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut sd
                ),
                0
            );
            let mut text = ptr::null_mut();
            assert_ne!(
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    sd,
                    1,
                    DACL_SECURITY_INFORMATION,
                    &mut text,
                    ptr::null_mut()
                ),
                0
            );
            let descriptor = wide_string(text);
            LocalFree(text.cast());
            LocalFree(sd);
            assert!(descriptor.starts_with("D:P"));
            assert!(!descriptor.contains(";;;WD)"));
            assert!(!descriptor.contains(";;;AU)"));
            assert!(!descriptor.contains(";;;BU)"));
            assert!(descriptor.contains(&current_sid().unwrap()));
        }
    }
}
