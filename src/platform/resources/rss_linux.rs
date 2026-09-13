/// Native resident-memory observation for a numeric PID, in bytes.
/// Returns None when unavailable; not a held process identity or cgroup charge.
/// Linux reads VmRSS, Windows WorkingSetSize, macOS invokes ps. PID reuse can
/// race the observation; use only for telemetry, never process control.
pub fn process_rss_bytes(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    parse_vm_rss_kb(&status)?.checked_mul(1024)
}

/// Pure parser split out of [`process_rss_bytes`] so the `VmRSS:` line
/// format can be unit-tested without a real `/proc/<pid>/status` file.
fn parse_vm_rss_kb(status: &str) -> Option<u64> {
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rss_parser_distinguishes_missing_zero_and_malformed_values() {
        assert_eq!(parse_vm_rss_kb("VmSize: 900 kB\nVmRSS: 12 kB\n"), Some(12));
        assert_eq!(parse_vm_rss_kb("VmRSS: 0 kB\n"), Some(0));
        assert_eq!(parse_vm_rss_kb("VmRSS: unknown\n"), None);
        assert_eq!(parse_vm_rss_kb("VmSize: 900 kB\n"), None);
    }
}
