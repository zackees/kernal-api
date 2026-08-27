//! Private hooks for the ignored native containment proof.  This module is
//! feature-gated so ordinary worker builds cannot observe its environment.


const MARKER_ENV: &str = "KERNAL_API_WASM_WORKER_IDENTITY_MARKER";
const VERSION: &str = "kernal-api-worker-identity-v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WorkerIdentity {
    pub(super) pid: u32,
    pub(super) creation_a: u64,
    pub(super) creation_b: u64,
}

/// Capture an identity which is meaningful only for equality on this host.
pub(super) fn capture(pid: u32) -> std::io::Result<WorkerIdentity> {
    platform_capture(pid)
}

/// The parent alone reads this opt-in variable.  `spawn_contained_worker`
/// supplies an explicit empty environment, so the child cannot inherit it.
pub(super) fn publish_worker_identity(pid: u32) -> Result<(), ()> {
    let Some(path) = std::env::var_os(MARKER_ENV) else { return Ok(()); };
    let identity = capture(pid).map_err(|_| ())?;
    let text = encode(identity);
    if text.len() > 256 { return Err(()); }
    let path = std::path::PathBuf::from(path);
    let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temporary, text).map_err(|_| ())?;
    std::fs::rename(&temporary, &path).map_err(|_| {
        let _ = std::fs::remove_file(temporary);
    })
}

fn encode(identity: WorkerIdentity) -> String {
    format!("{VERSION}\npid={}\ncreation-a={}\ncreation-b={}\n", identity.pid, identity.creation_a, identity.creation_b)
}

fn decode(text: &str) -> Option<WorkerIdentity> {
    let mut lines = text.lines();
    if lines.next()? != VERSION { return None; }
    let mut parse = |prefix| -> Option<u64> { lines.next()?.strip_prefix(prefix)?.parse().ok() };
    let identity = WorkerIdentity { pid: parse("pid=")?.try_into().ok()?, creation_a: parse("creation-a=")?, creation_b: parse("creation-b=")? };
    if lines.next().is_some() { return None; }
    Some(identity)
}

#[cfg(target_os = "linux")]
fn platform_capture(pid: u32) -> std::io::Result<WorkerIdentity> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let start = linux_starttime(&text).ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid /proc stat"))?;
    Ok(WorkerIdentity { pid, creation_a: start, creation_b: 0 })
}

/// Field 22 follows the final `)` of `comm`; command names may contain spaces
/// and parentheses, so whitespace splitting the whole record is incorrect.
#[cfg(target_os = "linux")]
fn linux_starttime(stat: &str) -> Option<u64> {
    let close = stat.rfind(')')?;
    let fields: Vec<_> = stat.get(close + 1..)?.split_whitespace().collect();
    // `fields[0]` is field 3 (state), therefore field 22 is offset 19.
    fields.get(19)?.parse().ok()
}

#[cfg(target_os = "windows")]
fn platform_capture(pid: u32) -> std::io::Result<WorkerIdentity> {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() { return Err(std::io::Error::last_os_error()); }
    let mut creation = std::mem::zeroed();
    let mut exit = std::mem::zeroed(); let mut kernel = std::mem::zeroed(); let mut user = std::mem::zeroed();
    let ok = unsafe { GetProcessTimes(process as HANDLE, &mut creation, &mut exit, &mut kernel, &mut user) };
    unsafe { CloseHandle(process as HANDLE); }
    if ok == 0 { return Err(std::io::Error::last_os_error()); }
    let ticks = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
    Ok(WorkerIdentity { pid, creation_a: ticks, creation_b: 0 })
}

// `proc_pidinfo(PROC_PIDTBSDINFO)` returns `proc_bsdinfo`; its start seconds
// and microseconds are copied as opaque equality components.  Keep the ABI
// declaration local because this is not a facade API.
#[cfg(target_os = "macos")]
fn platform_capture(pid: u32) -> std::io::Result<WorkerIdentity> {
    #[repr(C)] struct BsdInfo { pbi_flags: u32, pbi_status: u32, pbi_xstatus: u32, pbi_pid: u32, pbi_ppid: u32, pbi_uid: u32, pbi_gid: u32, pbi_ruid: u32, pbi_rgid: u32, pbi_svuid: u32, pbi_svgid: u32, rfu_1: u32, pbi_comm: [u8; 17], pbi_name: [u8; 33], pbi_nfiles: u32, pbi_pgid: u32, pbi_pjobc: u32, e_tdev: u32, e_tpgid: u32, pbi_nice: i32, pbi_start_tvsec: u64, pbi_start_tvusec: u64 }
    unsafe extern "C" { fn proc_pidinfo(pid: i32, flavor: i32, arg: u64, buffer: *mut core::ffi::c_void, buffersize: i32) -> i32; }
    let mut info: BsdInfo = unsafe { std::mem::zeroed() };
    let written = unsafe { proc_pidinfo(pid as i32, 3, 0, (&mut info as *mut BsdInfo).cast(), std::mem::size_of::<BsdInfo>() as i32) };
    if written as usize != std::mem::size_of::<BsdInfo>() { return Err(std::io::Error::last_os_error()); }
    Ok(WorkerIdentity { pid, creation_a: info.pbi_start_tvsec, creation_b: info.pbi_start_tvusec })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn marker_round_trip_and_rejects_extra_fields() {
        let value = WorkerIdentity { pid: 41, creation_a: 2, creation_b: 3 };
        assert_eq!(decode(&encode(value)), Some(value));
        assert_eq!(decode("wrong\npid=1\ncreation-a=2\ncreation-b=3\n"), None);
        assert_eq!(decode("kernal-api-worker-identity-v1\npid=1\ncreation-a=2\ncreation-b=3\nextra=x\n"), None);
    }
    #[cfg(target_os = "linux")]
    #[test] fn linux_stat_parser_uses_final_comm_delimiter() {
        assert_eq!(linux_starttime("7 (has ) spaces) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19"), Some(19));
    }
}
