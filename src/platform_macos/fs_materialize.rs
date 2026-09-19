//! macOS mechanics for cache materialization: replacement, link
//! classification, permission bits, path identity, and volume facts.
//!
//! Every operation here is the host half of a neutral operation in
//! `crate::platform::fs`; the documentation that states the contract lives
//! there.

use std::io;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

use crate::platform::fs::LinkKind;

// ---------------------------------------------------------------------------
// Replacement
// ---------------------------------------------------------------------------

pub fn is_transient_share_error(_error: &io::Error) -> bool {
    false
}

pub fn is_lock_contention(_error: &io::Error) -> bool {
    false
}

pub fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

pub fn rename_generation(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

pub fn replace_with_delete_fallback(source: &Path, destination: &Path) -> io::Result<()> {
    std::fs::rename(source, destination)
}

/// Exchange the two trees in place (renamex_np `RENAME_SWAP`) when
/// `requested` already exists, so the old tree is never missing under the
/// requested path.
pub fn install_directory(staged: &Path, requested: &Path) -> io::Result<()> {
    let parent = requested.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    if !requested.exists() {
        return std::fs::rename(staged, requested);
    }
    atomic_exchange_directories(staged, requested)?;
    remove_directory_if_present(staged)
}

fn atomic_exchange_directories(left: &Path, right: &Path) -> io::Result<()> {
    use std::os::unix::ffi::OsStrExt as _;

    let left = std::ffi::CString::new(left.as_os_str().as_bytes()).map_err(invalid_path)?;
    let right = std::ffi::CString::new(right.as_os_str().as_bytes()).map_err(invalid_path)?;
    // SAFETY: both pointers come from live CStrings, and renamex_np does not
    // retain them.
    let result = unsafe { libc::renamex_np(left.as_ptr(), right.as_ptr(), libc::RENAME_SWAP) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn remove_directory_if_present(path: &Path) -> io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn invalid_path(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

// ---------------------------------------------------------------------------
// Path-observed identity
// ---------------------------------------------------------------------------

/// The `(st_dev, st_ino)` pair of the object a path names.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PathFileIdentity {
    device: u64,
    inode: u64,
}

pub fn path_file_identity(path: &Path) -> io::Result<PathFileIdentity> {
    let metadata = std::fs::metadata(path)?;
    Ok(PathFileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

pub fn same_file(a: &Path, b: &Path) -> io::Result<bool> {
    Ok(path_file_identity(a)? == path_file_identity(b)?)
}

/// macOS keeps no per-file change journal this crate can read.
pub fn file_change_marker(_path: &Path) -> Option<i128> {
    None
}

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

pub fn hard_link_count(path: &Path) -> io::Result<u64> {
    Ok(std::fs::metadata(path)?.nlink())
}

pub fn classify(path: &Path) -> io::Result<LinkKind> {
    Ok(
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            LinkKind::Symlink
        } else {
            LinkKind::Regular
        },
    )
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

pub fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    metadata.permissions().mode()
}

pub fn apply_metadata_mode(path: &Path, mode: u32) -> io::Result<()> {
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
}

pub fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    let mode = if readonly {
        permissions.mode() & !0o222
    } else {
        permissions.mode() | 0o200
    };
    permissions.set_mode(mode);
    std::fs::set_permissions(path, permissions)
}

pub fn make_executable(path: &Path) -> io::Result<()> {
    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o111);
    std::fs::set_permissions(path, permissions)
}

// ---------------------------------------------------------------------------
// Volume facts
// ---------------------------------------------------------------------------

pub fn volume_identity_u128(path: &Path) -> Option<u128> {
    std::fs::metadata(path)
        .ok()
        .map(|metadata| u128::from(metadata.dev()))
}

pub const fn file_id_width() -> u32 {
    64
}

pub fn allocated_bytes(_path: &Path, metadata: &std::fs::Metadata) -> u64 {
    metadata.blocks().saturating_mul(512)
}

// ---------------------------------------------------------------------------
// Paths and durability
// ---------------------------------------------------------------------------

pub fn path_from_raw_bytes(bytes: &[u8]) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;

    Some(std::ffi::OsString::from_vec(bytes.to_vec()).into())
}

pub fn native_call_path(path: &Path) -> io::Result<PathBuf> {
    Ok(path.to_path_buf())
}

pub fn sync_directory_if_supported(directory: &Path) -> io::Result<()> {
    std::fs::File::open(directory)?.sync_all()
}
