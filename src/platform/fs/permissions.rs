//! Path-based compatibility permission policies for materialized files.
//! These follow symlinks; they are not secure-open or ACL isolation primitives.
use std::path::Path;

/// Capture the host permission representation: full Unix mode bits, or the
/// Windows readonly attribute as 0/1. Values are host-specific, not portable
/// archive Unix modes. This observes supplied metadata without another lookup.
pub fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        u32::from(metadata.permissions().readonly())
    }
}

/// Restore the host representation returned by [`metadata_mode`]. Unix applies
/// the exact mode; Windows treats any nonzero value as readonly and otherwise
/// preserves attributes. Unlike [`restore_mode`], this is not an archive mode.
pub fn apply_metadata_mode(path: &Path, mode: u32) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        restore_mode(path, Some(mode))
    }
    #[cfg(not(unix))]
    {
        set_readonly(path, mode != 0)
    }
}

/// Toggle write permission while retaining unrelated mode bits/attributes.
/// Unix readonly clears all write bits; writable adds only owner-write.
/// Windows toggles its readonly attribute, avoiding an unchanged permission
/// write. This follows symlinks and returns metadata errors for missing paths.
pub fn set_readonly(path: &Path, readonly: bool) -> std::io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if readonly {
            permissions.mode() & !0o222
        } else {
            permissions.mode() | 0o200
        };
        permissions.set_mode(mode);
    }
    #[cfg(not(unix))]
    {
        if permissions.readonly() == readonly {
            return Ok(());
        }
        permissions.set_readonly(readonly);
    }
    std::fs::set_permissions(path, permissions)
}

/// Apply an archived Unix mode verbatim. `None` does not access the path.
/// Windows ignores Unix archive modes and performs no filesystem operation.
pub fn restore_mode(path: &Path, mode: Option<u32>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = mode {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (path, mode);
        Ok(())
    }
}

/// Add all Unix execute bits, retaining other mode bits. Windows is a no-op.
pub fn make_executable(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(path)?.permissions().mode();
        restore_mode(path, Some(mode | 0o111))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Publish Unix executables with fixed mode 0755, deliberately ignoring source
/// mode. Windows copies source permissions (including its readonly attribute).
pub fn make_executable_from(path: &Path, source: &std::fs::Permissions) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let _ = source;
        restore_mode(path, Some(0o755))
    }
    #[cfg(not(unix))]
    {
        std::fs::set_permissions(path, source.clone())
    }
}

/// Set Unix mode 0700. On Windows this is a compatibility no-op: it does not
/// establish privacy, modify ACLs, or validate existing access restrictions.
pub fn make_private(path: &Path) -> std::io::Result<()> {
    restore_mode(path, Some(0o700))
}

/// Read Unix mode bits, returning None on metadata failure or non-Unix hosts.
pub fn mode(path: &Path) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .ok()
            .map(|metadata| metadata.permissions().mode())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}
