/// Native resident-memory observation for a numeric PID, in bytes.
/// Returns None when unavailable; not a held process identity or cgroup charge.
/// Linux reads VmRSS, Windows WorkingSetSize, macOS invokes ps. PID reuse can
/// race the observation; use only for telemetry, never process control.
pub fn process_rss_bytes(pid: u32) -> Option<u64> {
    let mut command = std::process::Command::new("/bin/ps");
    command.args(["-o", "rss=", "-p", &pid.to_string()]);
    let output = crate::foreground::output(&mut command).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?
        .checked_mul(1024)
}
