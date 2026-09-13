/// Native resident-memory observation for a numeric PID, in bytes.
/// Returns None when unavailable; not a held process identity or cgroup charge.
/// Linux reads VmRSS, Windows WorkingSetSize, macOS invokes ps. PID reuse can
/// race the observation; use only for telemetry, never process control.
pub fn process_rss_bytes(pid: u32) -> Option<u64> {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    // SAFETY: FFI call with documented args; the returned handle is
    // null-checked before use and closed on every path below.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }

    let mut counters: PROCESS_MEMORY_COUNTERS = unsafe { std::mem::zeroed() };
    counters.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: `counters` is correctly-sized/`cb`-initialized; the API fills
    // it in-place. `handle` was null-checked above.
    let ok = unsafe { K32GetProcessMemoryInfo(handle, &mut counters, counters.cb) };

    // SAFETY: `handle` came from the successful `OpenProcess` above and is
    // closed exactly once here, on every path including the failure below.
    unsafe {
        CloseHandle(handle);
    }

    if ok == 0 {
        return None;
    }
    Some(counters.WorkingSetSize as u64)
}
