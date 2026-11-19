//! Restore a Win32 window using native coordinates for mixed-DPI monitors.
use std::{mem::size_of, ptr};

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows_sys::Win32::{
    Foundation::RECT,
    Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONULL,
        MONITOR_DEFAULTTOPRIMARY,
    },
    UI::WindowsAndMessaging::{
        FlashWindowEx, GetForegroundWindow, GetWindowRect, IsIconic, IsWindowVisible,
        SetForegroundWindow, SetWindowPos, ShowWindow, FLASHWINFO, FLASHW_TRAY, SWP_NOACTIVATE,
        SWP_NOSIZE, SWP_NOZORDER, SW_RESTORE, SW_SHOW,
    },
};

pub(crate) fn restore(window: &(impl HasWindowHandle + ?Sized)) {
    let Ok(handle) = window.window_handle() else {
        return;
    };
    let RawWindowHandle::Win32(handle) = handle.as_raw() else {
        return;
    };
    let hwnd = handle.hwnd.get() as windows_sys::Win32::Foundation::HWND;

    // This runs on the owning window thread via iced::window::run.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        } else if IsWindowVisible(hwnd) == 0 {
            ShowWindow(hwnd, SW_SHOW);
        }
        let mut bounds = RECT::default();
        if GetWindowRect(hwnd, &mut bounds) != 0
            && MonitorFromRect(&bounds, MONITOR_DEFAULTTONULL).is_null()
        {
            let monitor = MonitorFromRect(&bounds, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..MONITORINFO::default()
            };
            if !monitor.is_null() && GetMonitorInfoW(monitor, &mut info) != 0 {
                SetWindowPos(
                    hwnd,
                    ptr::null_mut(),
                    info.rcWork.left + 40,
                    info.rcWork.top + 40,
                    0,
                    0,
                    SWP_NOZORDER | SWP_NOSIZE | SWP_NOACTIVATE,
                );
            }
        }
        if GetForegroundWindow() != hwnd && SetForegroundWindow(hwnd) == 0 {
            FlashWindowEx(&FLASHWINFO {
                cbSize: size_of::<FLASHWINFO>() as u32,
                hwnd,
                dwFlags: FLASHW_TRAY,
                uCount: 2,
                dwTimeout: 0,
            });
        }
    }
}
