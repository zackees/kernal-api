use std::sync::OnceLock;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpuinfo_counts_physical_pairs_not_smt_threads() {
        let text = "physical id: 0\ncore id: 0\n\nphysical id: 0\ncore id: 0\n\nphysical id: 1\ncore id: 0\n";
        assert_eq!(cores_from_cpuinfo(text), Some(2));
        assert_eq!(cores_from_cpuinfo("processor: 0\n"), None);
    }

    #[test]
    fn sysfs_deduplicates_thread_sibling_sets() {
        let temp = tempfile::tempdir().unwrap();
        for (cpu, siblings) in [(0, "0,2"), (1, "1,3"), (2, "0,2"), (3, "1,3")] {
            let dir = temp.path().join(format!("cpu{cpu}/topology"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("thread_siblings_list"), siblings).unwrap();
        }
        assert_eq!(cores_from_sysfs(temp.path()), Some(2));
    }
}

/// Physical CPU cores on this machine, or `None` when the topology could
/// not be read. Memoized: the daemon asks once at startup.
pub fn physical_cores() -> Option<usize> {
    static CACHED: OnceLock<Option<usize>> = OnceLock::new();
    *CACHED.get_or_init(|| {
        cores_from_sysfs(std::path::Path::new("/sys/devices/system/cpu"))
            .or_else(|| cores_from_cpuinfo(&std::fs::read_to_string("/proc/cpuinfo").ok()?))
            .filter(|cores| *cores > 0)
    })
}

/// Every hardware thread publishes the sibling set it belongs to, and
/// siblings of one physical core publish the *same* list. So the number
/// of distinct lists is the number of physical cores — no parsing of
/// the list contents required.
fn cores_from_sysfs(cpu_root: &std::path::Path) -> Option<usize> {
    use std::collections::HashSet;

    let mut sibling_sets: HashSet<String> = HashSet::new();
    for entry in std::fs::read_dir(cpu_root).ok()? {
        let path = entry.ok()?.path();
        let is_cpu_dir = path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with("cpu") && name[3..].chars().all(|c| c.is_ascii_digit())
            });
        if !is_cpu_dir {
            continue;
        }
        let siblings = std::fs::read_to_string(path.join("topology/thread_siblings_list")).ok()?;
        sibling_sets.insert(siblings.trim().to_owned());
    }
    (!sibling_sets.is_empty()).then_some(sibling_sets.len())
}

/// Fall back to `/proc/cpuinfo`: count distinct `(physical id, core id)`
/// pairs.
fn cores_from_cpuinfo(contents: &str) -> Option<usize> {
    use std::collections::HashSet;

    let mut cores = HashSet::new();
    let mut package: Option<String> = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim().to_owned();
        // A `(physical id, core id)` pair is complete only when the
        // core-id line arrives. Recording on the physical-id line would
        // pair the new package with a stale core id from the previous
        // processor section (and vice versa).
        match key.trim() {
            "physical id" => package = Some(value),
            "core id" => {
                if let Some(package) = package.as_ref() {
                    cores.insert((package.clone(), value));
                }
            }
            _ => continue,
        }
    }
    (!cores.is_empty()).then_some(cores.len())
}
