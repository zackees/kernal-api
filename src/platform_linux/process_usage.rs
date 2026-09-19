//! Linux per-process CPU and resident-memory readings from `/proc`.

/// Linux loses a reaped child's accounting, and its PID may be reissued.
pub const PEAK_RSS_READABLE_AFTER_EXIT: bool = false;

/// Upper bound on processes one tree reading visits.
pub const MAX_TREE_RSS_PROCESSES: usize = 4096;

/// `utime + stime` in clock ticks from `/proc/<pid>/stat`.
pub fn cpu_ticks_for_pid(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `comm` may contain spaces or ')'; fixed fields start after the last ')'.
    let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    let user = fields.get(11)?.parse::<u64>().ok()?;
    let system = fields.get(12)?.parse::<u64>().ok()?;
    Some(user.wrapping_add(system))
}

fn status_kib(pid: u32, key: &str) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix(key))?
        .trim()
        .strip_suffix("kB")?
        .trim();
    Some(kib.parse::<u64>().ok()?.saturating_mul(1024))
}

/// `VmHWM`, the resident high-water mark.
pub fn peak_rss_bytes_for_pid(pid: u32) -> Option<u64> {
    status_kib(pid, "VmHWM:")
}

/// Current `VmRSS` of `pid` plus every live descendant.
pub fn tree_rss_bytes_for_pid(pid: u32) -> Option<u64> {
    let mut total = status_kib(pid, "VmRSS:")?;
    let mut seen = std::collections::HashSet::from([pid]);
    let mut stack = vec![pid];
    while let Some(parent) = stack.pop() {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{parent}/task")) else {
            continue;
        };
        for task in tasks.flatten() {
            let Ok(children) = std::fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            for child in children
                .split_whitespace()
                .filter_map(|value| value.parse::<u32>().ok())
            {
                if seen.len() >= MAX_TREE_RSS_PROCESSES || !seen.insert(child) {
                    continue;
                }
                if let Some(bytes) = status_kib(child, "VmRSS:") {
                    total = total.saturating_add(bytes);
                }
                stack.push(child);
            }
        }
    }
    Some(total)
}
