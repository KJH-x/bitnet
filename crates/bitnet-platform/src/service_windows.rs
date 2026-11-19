use super::*;
use crate::{error, windows::wide, WINDOWS_SERVICE_NAME};
use bitnet_core::ErrorCode;
use std::{
    ptr,
    sync::{atomic::AtomicUsize, Mutex, OnceLock},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::*,
    System::{Services::*, Threading::*},
};
type Callback = Box<dyn FnOnce() -> Result<(), AppError> + Send>;
static CALLBACK: OnceLock<Mutex<Option<Callback>>> = OnceLock::new();
static RESULT: Mutex<Option<Result<(), AppError>>> = Mutex::new(None);
static STATUS_HANDLE: AtomicUsize = AtomicUsize::new(0);

pub(super) fn status() -> ServiceStatus {
    unsafe {
        let manager = OpenSCManagerW(ptr::null(), ptr::null(), SC_MANAGER_CONNECT);
        if manager.is_null() {
            return ServiceStatus {
                installed: true,
                detail: Some(
                    "Service manager could not be queried; automatic fallback is disabled".into(),
                ),
                ..Default::default()
            };
        }
        let name = wide(WINDOWS_SERVICE_NAME);
        let service = OpenServiceW(
            manager,
            name.as_ptr(),
            SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG,
        );
        if service.is_null() {
            let missing = GetLastError() == ERROR_SERVICE_DOES_NOT_EXIST;
            CloseServiceHandle(manager);
            return ServiceStatus {
                installed: !missing,
                detail: if missing {
                    None
                } else {
                    Some("Cannot query the installed service".into())
                },
                ..Default::default()
            };
        }
        let mut state: SERVICE_STATUS_PROCESS = std::mem::zeroed();
        let mut needed = 0;
        let ok = QueryServiceStatusEx(
            service,
            SC_STATUS_PROCESS_INFO,
            (&mut state as *mut SERVICE_STATUS_PROCESS).cast(),
            std::mem::size_of_val(&state) as u32,
            &mut needed,
        );
        let mut length = 0;
        QueryServiceConfigW(service, ptr::null_mut(), 0, &mut length);
        let mut storage = vec![0u64; (length as usize).div_ceil(8)];
        let mut start_type = None;
        if length > 0
            && QueryServiceConfigW(service, storage.as_mut_ptr().cast(), length, &mut length) != 0
        {
            start_type = Some((*storage.as_ptr().cast::<QUERY_SERVICE_CONFIGW>()).dwStartType);
        }
        CloseServiceHandle(service);
        CloseServiceHandle(manager);
        ServiceStatus {
            installed: true,
            running: ok != 0 && state.dwCurrentState == SERVICE_RUNNING,
            enabled: start_type == Some(SERVICE_AUTO_START),
            detail: if ok == 0 {
                Some("Cannot query service runtime status".into())
            } else if !matches!(state.dwCurrentState, SERVICE_RUNNING | SERVICE_STOPPED) {
                Some("The system service is changing state".into())
            } else if start_type.is_none() {
                Some("Cannot query service startup configuration".into())
            } else {
                None
            },
        }
    }
}
pub(super) fn is_admin() -> bool {
    unsafe {
        let mut token = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut value: TOKEN_ELEVATION = std::mem::zeroed();
        let mut n = 0;
        let result = GetTokenInformation(
            token,
            TokenElevation,
            (&mut value as *mut TOKEN_ELEVATION).cast(),
            std::mem::size_of_val(&value) as u32,
            &mut n,
        ) != 0
            && value.TokenIsElevated != 0;
        CloseHandle(token);
        result
    }
}
fn report(state: u32, exit: u32) {
    unsafe {
        let handle = STATUS_HANDLE.load(Ordering::Acquire) as SERVICE_STATUS_HANDLE;
        if !handle.is_null() {
            let status = SERVICE_STATUS {
                dwServiceType: SERVICE_WIN32_OWN_PROCESS,
                dwCurrentState: state,
                dwControlsAccepted: if state == SERVICE_RUNNING {
                    SERVICE_ACCEPT_STOP | SERVICE_ACCEPT_SHUTDOWN
                } else {
                    0
                },
                dwWin32ExitCode: exit,
                dwServiceSpecificExitCode: 0,
                dwCheckPoint: 0,
                dwWaitHint: if state == SERVICE_STOP_PENDING {
                    15000
                } else {
                    0
                },
            };
            SetServiceStatus(handle, &status);
        }
    }
}
unsafe extern "system" fn handler(
    control: u32,
    _: u32,
    _: *mut std::ffi::c_void,
    _: *mut std::ffi::c_void,
) -> u32 {
    match control {
        SERVICE_CONTROL_STOP | SERVICE_CONTROL_SHUTDOWN => {
            STOPPING.store(true, Ordering::Release);
            report(SERVICE_STOP_PENDING, 0);
            0
        }
        SERVICE_CONTROL_INTERROGATE => 0,
        _ => ERROR_CALL_NOT_IMPLEMENTED,
    }
}
unsafe extern "system" fn main(_: u32, _: *mut *mut u16) {
    let name = wide(WINDOWS_SERVICE_NAME);
    let handle = RegisterServiceCtrlHandlerExW(name.as_ptr(), Some(handler), ptr::null());
    if handle.is_null() {
        *RESULT.lock().unwrap() = Some(Err(error(
            ErrorCode::Unavailable,
            "Cannot register service control handler",
        )));
        return;
    }
    STATUS_HANDLE.store(handle as usize, Ordering::Release);
    report(SERVICE_START_PENDING, 0);
    let callback = CALLBACK.get().and_then(|c| c.lock().ok()?.take());
    report(SERVICE_RUNNING, 0);
    let result = match callback {
        Some(c) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(c)).unwrap_or_else(|_| {
            Err(error(
                ErrorCode::Internal,
                "Service engine terminated unexpectedly",
            ))
        }),
        None => Err(error(
            ErrorCode::Internal,
            "Service engine was not configured",
        )),
    };
    report(
        SERVICE_STOPPED,
        if result.is_ok() {
            0
        } else {
            ERROR_PROCESS_ABORTED
        },
    );
    *RESULT.lock().unwrap() = Some(result);
    STATUS_HANDLE.store(0, Ordering::Release);
}
pub(super) fn run(
    callback: impl FnOnce() -> Result<(), AppError> + Send + 'static,
) -> Result<(), AppError> {
    CALLBACK
        .set(Mutex::new(Some(Box::new(callback))))
        .map_err(|_| {
            error(
                ErrorCode::Busy,
                "Service host can only run once per process",
            )
        })?;
    let mut name = wide(WINDOWS_SERVICE_NAME);
    let table = [
        SERVICE_TABLE_ENTRYW {
            lpServiceName: name.as_mut_ptr(),
            lpServiceProc: Some(main),
        },
        SERVICE_TABLE_ENTRYW {
            lpServiceName: ptr::null_mut(),
            lpServiceProc: None,
        },
    ];
    if unsafe { StartServiceCtrlDispatcherW(table.as_ptr()) } == 0 {
        return Err(error(
            ErrorCode::Unavailable,
            "The service host must be launched by Windows Service Control Manager",
        ));
    }
    RESULT
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| Err(error(ErrorCode::Unavailable, "Service host did not run")))
}
