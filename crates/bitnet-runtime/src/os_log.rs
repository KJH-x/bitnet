//! Best-effort OS event sink. Input is already sanitized structured event text.
pub fn write(level: &str, message: &str) {
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::Foundation::HANDLE;
        #[link(name = "advapi32")]
        extern "system" {
            fn RegisterEventSourceW(server: *const u16, source: *const u16) -> HANDLE;
            fn ReportEventW(
                event: HANDLE,
                kind: u16,
                category: u16,
                id: u32,
                sid: *mut core::ffi::c_void,
                count: u16,
                size: u32,
                strings: *const *const u16,
                data: *mut core::ffi::c_void,
            ) -> i32;
            fn DeregisterEventSource(event: HANDLE) -> i32;
        }
        let source = "BitNet\0".encode_utf16().collect::<Vec<_>>();
        let handle = RegisterEventSourceW(std::ptr::null(), source.as_ptr());
        if !handle.is_null() {
            let text = message
                .replace('\0', "")
                .encode_utf16()
                .chain(Some(0))
                .collect::<Vec<_>>();
            let pointers = [text.as_ptr()];
            let kind = match level {
                "error" => 1,
                "warning" => 2,
                _ => 4,
            };
            let _ = ReportEventW(
                handle,
                kind,
                0,
                1000,
                std::ptr::null_mut(),
                1,
                0,
                pointers.as_ptr(),
                std::ptr::null_mut(),
            );
            DeregisterEventSource(handle);
        }
    }
    #[cfg(unix)]
    {
        // systemd and launchd capture this stream using their native log sinks.
        eprintln!(
            "{}",
            serde_json::json!({"source":"bitnet","level":level,"message":message})
        );
    }
}
