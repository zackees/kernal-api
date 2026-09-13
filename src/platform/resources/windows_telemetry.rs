/// Walk the live process table once via ToolHelp, returning `(pid, image
/// name)` rows. `None` if the snapshot could not be taken.
///
/// Windows-only telemetry, not held process identity. An iteration failure
/// returns the rows collected so far, possibly empty. Other platforms return `None`.
pub fn process_table() -> Option<Vec<(u32, String)>> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    // SAFETY: FFI call with documented args; returns a handle we validate
    // against INVALID_HANDLE_VALUE before use and CloseHandle on every path.
    let handle = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return None;
    }

    let mut rows = Vec::new();
    let mut entry: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

    // SAFETY: `entry` is a fully-owned, correctly-sized PROCESSENTRY32W; the
    // API fills `szExeFile` in-place. `handle` is valid (checked above).
    let mut ok = unsafe { Process32FirstW(handle, &mut entry) };
    while ok != 0 {
        rows.push((entry.th32ProcessID, image_name_from_entry(&entry.szExeFile)));
        // SAFETY: same invariants as Process32FirstW; iterates the snapshot.
        ok = unsafe { Process32NextW(handle, &mut entry) };
    }

    // SAFETY: `handle` came from CreateToolhelp32Snapshot and is closed exactly
    // once here, including when iteration stopped early.
    unsafe {
        CloseHandle(handle);
    }

    Some(rows)
}

/// Decode a NUL-terminated UTF-16 `szExeFile` field into a Rust `String`.
fn image_name_from_entry(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

#[cfg(test)]
mod tests {
    use super::image_name_from_entry;

    #[test]
    fn image_name_stops_at_first_nul() {
        assert_eq!(image_name_from_entry(&[65, 0, 66]), "A");
        assert_eq!(image_name_from_entry(&[65, 66]), "AB");
        assert_eq!(image_name_from_entry(&[]), "");
    }

    #[test]
    fn malformed_utf16_is_lossy_not_a_snapshot_failure() {
        assert_eq!(image_name_from_entry(&[0xd800, 0]), "\u{fffd}");
        assert_eq!(image_name_from_entry(&[0xd83d, 0xde80, 0]), "🚀");
    }
}

/// Read the system commit charge via `GlobalMemoryStatusEx`, returning
/// `(used_mb, limit_mb)` in binary megabytes (MiB). `None` if the call failed.
/// Other platforms return `None`. This is not a worker or cgroup memory charge.
pub fn commit_charge_mb() -> Option<(u64, u64)> {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;

    // SAFETY: `status` is a correctly-sized, dwLength-initialized MEMORYSTATUSEX
    // that the API fills in-place; no handles or allocations are involved.
    let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
    if ok == 0 {
        return None;
    }

    const MB: u64 = 1024 * 1024;
    // ullTotalPageFile is the commit limit; ullAvailPageFile what remains.
    let limit = status.ullTotalPageFile / MB;
    let used = status
        .ullTotalPageFile
        .saturating_sub(status.ullAvailPageFile)
        / MB;
    Some((used, limit))
}
