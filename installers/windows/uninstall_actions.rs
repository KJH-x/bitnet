#![no_std]
//! Deferred and impersonated MSI removal action. No commands or paths are accepted.
use core::ffi::{c_char, c_void};
type Key = *mut c_void;
#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExA(root: Key, name: *const c_char, options: u32, access: u32, key: *mut Key) -> i32;
    fn RegDeleteValueA(key: Key, name: *const c_char) -> i32;
    fn RegCloseKey(key: Key) -> i32;
}
#[no_mangle]
pub extern "system" fn RemoveStartup(_: u32) -> u32 {
    const SUCCESS: i32 = 0;
    const NOT_FOUND: i32 = 2;
    const INSTALL_FAILURE: u32 = 1603;
    let current_user = 0x80000001u32 as i32 as isize as Key;
    let mut key = core::ptr::null_mut();
    let opened = unsafe { RegOpenKeyExA(current_user, c"Software\\Microsoft\\Windows\\CurrentVersion\\Run".as_ptr(), 0, 0x0002, &mut key) };
    if opened == NOT_FOUND { return 0; }
    if opened != SUCCESS { return INSTALL_FAILURE; }
    let engine = unsafe { RegDeleteValueA(key, c"BitNetEngine".as_ptr()) };
    let gui = unsafe { RegDeleteValueA(key, c"BitNetGUI".as_ptr()) };
    unsafe { RegCloseKey(key) };
    if matches!(engine, SUCCESS | NOT_FOUND) && matches!(gui, SUCCESS | NOT_FOUND) { 0 } else { INSTALL_FAILURE }
}
#[panic_handler]
fn panic(_: &core::panic::PanicInfo<'_>) -> ! { loop { core::hint::spin_loop(); } }
