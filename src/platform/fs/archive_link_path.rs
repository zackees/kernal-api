//! Pure lexical resolution; this does not inspect symlinks or authorize I/O.
use std::path::{Component, Path, PathBuf};

/// Resolve a relative archive link target within a lexical destination root.
/// Rejects absolute targets, escaping parent components, and link names with
/// parent components. Existing filesystem links can still point outside this
/// root: callers must separately enforce containment when performing I/O.
pub fn resolve_archive_link_target(
    dest: &Path,
    link_path: &Path,
    target: &Path,
) -> Option<PathBuf> {
    let link_rel = link_path.strip_prefix(dest).ok()?;
    let mut stack: Vec<std::ffi::OsString> = Vec::new();
    for component in link_rel.components() {
        match component {
            Component::Normal(part) => stack.push(part.to_os_string()),
            Component::CurDir => {}
            _ => return None,
        }
    }
    // Drop the link's own file name; targets are relative to its parent.
    stack.pop()?;
    for component in target.components() {
        match component {
            Component::Normal(part) => stack.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                stack.pop()?;
            }
            // Absolute targets (RootDir / Prefix) never resolve inside dest.
            _ => return None,
        }
    }
    let mut resolved = dest.to_path_buf();
    for part in stack {
        resolved.push(part);
    }
    Some(resolved)
}
