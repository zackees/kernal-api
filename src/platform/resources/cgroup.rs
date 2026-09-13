//! Current Linux cgroup-v2 membership, resolved in the caller's mount namespace.

use std::path::PathBuf;

/// Resolve this process's visible cgroup-v2 directory.
///
/// Uses both procfs membership and mount information; does not assume a fixed
/// mount point. Returns `None` off Linux, when procfs is unavailable, or when
/// the membership has no matching visible mount. This is a best-effort
/// observation, not a held identity or an effective ancestor-limit calculation.
pub fn cgroup_v2_dir() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        resolve(
            &std::fs::read("/proc/self/cgroup").ok()?,
            &std::fs::read("/proc/self/mountinfo").ok()?,
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(target_os = "linux")]
fn resolve(membership: &[u8], mountinfo: &[u8]) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::{Component, Path};

    let member = membership
        .split(|b| *b == b'\n')
        .find_map(|line| line.strip_prefix(b"0::"))?;
    let member = PathBuf::from(OsString::from_vec(member.to_vec()));
    // A namespace may expose an unreachable membership with ".."; never
    // let that escape a visible mount and read an unrelated directory.
    fn absolute_clean(path: &Path) -> bool {
        path.is_absolute() && !path.components().any(|c| matches!(c, Component::ParentDir))
    }
    if !absolute_clean(&member) {
        return None;
    }
    let mut best: Option<(usize, PathBuf)> = None;
    for line in mountinfo.split(|b| *b == b'\n') {
        let fields: Vec<_> = line.split(|b| *b == b' ').collect();
        let Some(separator) = fields.iter().position(|field| *field == b"-") else {
            continue;
        };
        if separator < 6 || fields.get(separator + 1).copied() != Some(b"cgroup2".as_slice()) {
            continue;
        }
        let Some(root) = unescape(fields[3]) else {
            continue;
        };
        let Some(mount) = unescape(fields[4]) else {
            continue;
        };
        let root = PathBuf::from(OsString::from_vec(root));
        let mount = PathBuf::from(OsString::from_vec(mount));
        if !absolute_clean(&root) || !absolute_clean(&mount) {
            continue;
        }
        let Ok(relative) = member.strip_prefix(&root) else {
            continue;
        };
        let depth = root.components().count();
        if best
            .as_ref()
            .is_none_or(|(old_depth, _)| depth > *old_depth)
        {
            best = Some((depth, mount.join(relative)));
        }
    }
    best.map(|(_, path)| path)
}

#[cfg(target_os = "linux")]
fn unescape(raw: &[u8]) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\\' {
            let code = raw.get(i + 1..i + 4)?;
            let value = match code {
                b"040" => b' ',
                b"011" => b'\t',
                b"012" => b'\n',
                b"134" => b'\\',
                _ => return None,
            };
            result.push(value);
            i += 4;
        } else {
            result.push(raw[i]);
            i += 1;
        }
    }
    Some(result)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn resolves_host_and_namespaced_membership() {
        let mounts = b"1 0 0:1 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n";
        assert_eq!(
            resolve(b"0::/job/step\n", mounts),
            Some("/sys/fs/cgroup/job/step".into())
        );
        assert_eq!(resolve(b"0::/\n", mounts), Some("/sys/fs/cgroup/".into()));
    }

    #[test]
    fn subtree_mount_uses_relative_membership_and_decodes_mount_path() {
        let mounts = b"1 0 0:1 /job /run/cgroup\\040view rw - cgroup2 cgroup rw\n";
        assert_eq!(
            resolve(b"0::/job/step\n", mounts),
            Some("/run/cgroup view/step".into())
        );
        assert_eq!(resolve(b"0::/job-other/step\n", mounts), None);
    }

    #[test]
    fn unavailable_or_escaping_membership_is_not_a_root_fallback() {
        let mounts = b"1 0 0:1 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n";
        assert_eq!(resolve(b"0::/../outside\n", mounts), None);
        assert_eq!(resolve(b"2:memory:/job\n", mounts), None);
        assert_eq!(resolve(b"0::/job\n", b""), None);
    }

    #[test]
    fn most_specific_visible_subtree_wins() {
        let mounts =
            b"1 0 0:1 / /root rw - cgroup2 cgroup rw\n2 0 0:1 /job /sub rw - cgroup2 cgroup rw\n";
        assert_eq!(resolve(b"0::/job/step\n", mounts), Some("/sub/step".into()));
    }

    #[test]
    fn preserves_non_utf8_paths_and_optional_mount_fields() {
        use std::os::unix::ffi::OsStrExt;
        let mounts = b"1 0 0:1 / /cg\xff rw shared:4 - cgroup2 cgroup rw\n";
        let path = resolve(b"0::/job\xfe\n", mounts).unwrap();
        assert_eq!(path.as_os_str().as_bytes(), b"/cg\xff/job\xfe");
    }

    #[test]
    fn malformed_mounts_and_other_filesystems_are_ignored() {
        let mounts = b"bad\n1 0 0:1 / /bad\\777 rw - cgroup2 cgroup rw\n2 0 0:2 / /v1 rw - cgroup cgroup rw\n";
        assert_eq!(resolve(b"0::/job\n", mounts), None);
    }
}
