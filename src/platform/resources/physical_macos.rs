use std::sync::OnceLock;

/// Physical CPU cores on this machine, or `None` when the topology could
/// not be read. Memoized: the daemon asks once at startup.
pub fn physical_cores() -> Option<usize> {
    static CACHED: OnceLock<Option<usize>> = OnceLock::new();
    *CACHED.get_or_init(|| detect_cores().filter(|cores| *cores > 0))
}

/// Query the current physical-core count rather than the maximum count.
fn detect_cores() -> Option<usize> {
    let mut command = std::process::Command::new("/usr/sbin/sysctl");
    command.args(["-n", "hw.physicalcpu"]);
    let output = crate::foreground::output(&mut command).ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}
