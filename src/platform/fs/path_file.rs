//! Path-observed file identity, preserving native identifier width.
//!
//! These observations follow links and do not retain handles. Identity can be
//! reused after deletion; equality is not a content or immutability guarantee.
//! Separate lookups may race with path replacement.

use std::path::Path;

#[cfg(unix)]
#[path = "path_file_unix.rs"]
mod native;
#[cfg(windows)]
#[path = "path_file_win.rs"]
mod native;

/// Opaque host identity: Unix device/inode, or Windows volume serial plus
/// 128-bit FILE_ID_INFO (legacy 64-bit index fallback when unavailable).
/// This is distinct from the older fs::FileIdentity and serde fingerprint.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileIdentity(native::RawFileIdentity);

/// Observe a path's native identity. Unix propagates metadata errors. Windows
/// requests metadata-only access with read/write/delete sharing, supports
/// directories, and maps unavailable identity to NotFound for compatibility.
pub fn file_identity(path: &Path) -> std::io::Result<FileIdentity> {
    native::file_identity(path).map(FileIdentity)
}

/// Compare two path observations. Unix propagates metadata errors; Windows
/// returns Ok(false) when either identity is unavailable, including both.
pub fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    native::same_file(a, b)
}
