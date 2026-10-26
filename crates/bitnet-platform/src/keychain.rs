//! Legacy macOS Keychain integration. System-keychain ACL and signed launchd behavior require
//! real macOS acceptance testing; never substitute a plaintext file if the OS denies access.
use crate::{error, random};
use bitnet_core::{AppError, ErrorCode, Scope};
use std::{ffi::c_void, path::Path, ptr};
use zeroize::Zeroizing;
type Ref = *mut c_void;
#[link(name = "Security", kind = "framework")]
extern "C" {
    fn SecKeychainOpen(path: *const i8, keychain: *mut Ref) -> i32;
    fn SecKeychainFindGenericPassword(
        keychain: Ref,
        service_len: u32,
        service: *const u8,
        account_len: u32,
        account: *const u8,
        password_len: *mut u32,
        password: *mut *mut c_void,
        item: *mut Ref,
    ) -> i32;
    fn SecKeychainAddGenericPassword(
        keychain: Ref,
        service_len: u32,
        service: *const u8,
        account_len: u32,
        account: *const u8,
        password_len: u32,
        password: *const c_void,
        item: *mut Ref,
    ) -> i32;
    fn SecKeychainItemFreeContent(attributes: *mut c_void, data: *mut c_void) -> i32;
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
}
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(value: Ref);
}
pub(super) fn key(scope: Scope, data_dir: &Path) -> Result<[u8; 32], AppError> {
    let service = b"top.nsapi.bitnet.masterkey.v1";
    let account = data_dir.to_string_lossy();
    unsafe {
        let mut keychain = ptr::null_mut();
        if scope == Scope::System {
            SecKeychainSetUserInteractionAllowed(0);
            if SecKeychainOpen(
                c"/Library/Keychains/System.keychain".as_ptr(),
                &mut keychain,
            ) != 0
            {
                return Err(unavailable());
            }
        }
        struct Handle(Ref);
        impl Drop for Handle {
            fn drop(&mut self) {
                if !self.0.is_null() {
                    unsafe { CFRelease(self.0) }
                }
            }
        }
        let _handle = Handle(keychain);
        let (mut len, mut data) = (0, ptr::null_mut());
        let code = SecKeychainFindGenericPassword(
            keychain,
            service.len() as u32,
            service.as_ptr(),
            account.len() as u32,
            account.as_ptr(),
            &mut len,
            &mut data,
            ptr::null_mut(),
        );
        if code == 0 {
            let value = Zeroizing::new(
                std::slice::from_raw_parts(data.cast::<u8>(), len as usize).to_vec(),
            );
            // Erase the framework-owned plaintext buffer before releasing it.
            zeroize::Zeroize::zeroize(std::slice::from_raw_parts_mut(
                data.cast::<u8>(),
                len as usize,
            ));
            SecKeychainItemFreeContent(ptr::null_mut(), data);
            return value.as_slice().try_into().map_err(|_| unavailable());
        }
        if code != -25300 || data_dir.join("accounts.enc").exists() {
            return Err(unavailable());
        }
        let key = random::<32>()?;
        if SecKeychainAddGenericPassword(
            keychain,
            service.len() as u32,
            service.as_ptr(),
            account.len() as u32,
            account.as_ptr(),
            32,
            key.as_ptr().cast(),
            ptr::null_mut(),
        ) != 0
        {
            return Err(unavailable());
        }
        Ok(key)
    }
}
fn unavailable() -> AppError {
    error(
        ErrorCode::CredentialsUnavailable,
        "macOS Keychain is unavailable, locked, or denied access; no plaintext fallback is allowed",
    )
}
