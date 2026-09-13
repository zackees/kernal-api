//! Native directory permission compatibility policy.
//! Path-based, follows links; callers must control parent paths. This is not
//! a secure-open primitive or a guarantee against concurrent path replacement.

//! Linux permission mechanics: mode-bit based, owner-only = 0700.

use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::path::Path;

pub fn ensure_dir_private(path: &Path) -> std::io::Result<bool> {
    let metadata = std::fs::metadata(path)?;
    let full_mode = metadata.permissions().mode();
    // Preserve shared-root policy. The sticky bit
    // is the OS's marker for a *shared* temp root — `/tmp` and `/var/tmp`
    // are `1777` by design. Tightening one would be wrong for every other
    // process on the machine, and as root it would succeed.
    const STICKY: u32 = 0o1000;
    if full_mode & STICKY != 0 {
        return Ok(false);
    }
    let mode = full_mode & 0o777;
    if mode & 0o022 == 0 {
        return Ok(false);
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    let after = std::fs::metadata(path)?;
    if after.permissions().mode() & 0o022 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "{} is writable by others and could not be tightened",
                path.display()
            ),
        ));
    }
    Ok(true)
}

pub fn create_dir_all_private(path: &Path) -> std::io::Result<()> {
    // Mode at creation time, matching the original unix arm: no window
    // where the directory is live with an inherited mode.
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
}
