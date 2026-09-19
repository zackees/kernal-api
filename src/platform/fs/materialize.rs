//! Link classification, permission bits, change markers, volume facts, and
//! native path spellings for callers that materialize cached artifacts.
//!
//! These are path-based compatibility operations: they follow links unless
//! stated otherwise, retain no handles, and are not secure-open or ACL
//! isolation primitives. The caller controls the paths involved.

use std::io;
use std::path::{Path, PathBuf};

use crate::native_fs_materialize as native;

// ---------------------------------------------------------------------------
// Links
// ---------------------------------------------------------------------------

/// A no-follow classification of one directory entry, without native
/// attribute bits.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LinkKind {
    /// An ordinary file or directory.
    Regular,
    /// A symbolic link: on Windows, a name-surrogate symlink reparse point.
    Symlink,
    /// A reparse point that is not a symbolic link (a Windows junction,
    /// mount point, cloud placeholder, ...). Callers must not traverse it as
    /// a regular directory entry. Never reported on Linux or macOS.
    Reparse,
}

/// Classify the entry at `path` without following its final component.
///
/// On Windows a reparse point whose tag cannot be read classifies as
/// [`LinkKind::Reparse`], never as a symlink.
///
/// # Errors
///
/// Returns the no-follow metadata error, including `NotFound` for a missing
/// entry. A dangling symbolic link is still classified.
pub fn classify(path: &Path) -> io::Result<LinkKind> {
    native::classify(path)
}

/// Number of hard links to the object `path` currently names.
///
/// Follows symbolic links, so a dangling link reports `NotFound`. On Windows
/// the handle is opened metadata-only with every share mode, so observing a
/// count never evicts an existing writer.
///
/// # Errors
///
/// Returns the metadata or native query error.
pub fn hard_link_count(path: &Path) -> io::Result<u64> {
    native::hard_link_count(path)
}

/// Create a symbolic link at `link` to the file `target`, which may not
/// exist yet.
///
/// Windows creates the file flavor of symbolic link, which may require the
/// Developer Mode or `SeCreateSymbolicLinkPrivilege` the host grants.
///
/// # Errors
///
/// Returns the native error, including `AlreadyExists` when `link` exists.
pub fn symlink_file(target: &Path, link: &Path) -> io::Result<()> {
    native::symlink_file(target, link)
}

// ---------------------------------------------------------------------------
// Permissions
// ---------------------------------------------------------------------------

/// Capture the host permission representation from already-fetched
/// metadata: the full Unix mode bits, or the Windows readonly attribute as
/// `0`/`1`. Values are host-specific, not portable archive modes; restore one
/// only with [`apply_metadata_mode`] on the same host.
pub fn metadata_mode(metadata: &std::fs::Metadata) -> u32 {
    native::metadata_mode(metadata)
}

/// Restore a value returned by [`metadata_mode`]. Unix applies the exact
/// mode; Windows treats any nonzero value as readonly and preserves every
/// other attribute.
///
/// # Errors
///
/// Returns the metadata or permission-change error.
pub fn apply_metadata_mode(path: &Path, mode: u32) -> io::Result<()> {
    native::apply_metadata_mode(path, mode)
}

/// Toggle write permission while retaining unrelated mode bits and
/// attributes.
///
/// Unix `readonly = true` clears every write bit; `false` adds only the
/// owner-write bit. Windows toggles its readonly attribute and skips the
/// write when it already has the requested value. Follows symbolic links.
///
/// # Errors
///
/// Returns the metadata error for a missing path (including a dangling
/// link), or the permission-change error.
pub fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    native::set_readonly(path, readonly)
}

/// Add every Unix execute bit, retaining the other mode bits. Windows has no
/// per-file execute bit and performs no filesystem operation.
///
/// # Errors
///
/// On Unix, returns the metadata or permission-change error.
pub fn make_executable(path: &Path) -> io::Result<()> {
    native::make_executable(path)
}

// ---------------------------------------------------------------------------
// Change markers
// ---------------------------------------------------------------------------

/// An opaque change-journal observation of one file: its Windows USN.
///
/// Compare only observations of the same file within one journal epoch. It
/// is not a content hash, a file identity, or proof against a journal
/// reset, a concurrent write, or path replacement.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileChangeMarker(i128);

/// Observe `path`'s change-journal position.
///
/// Windows reads the file's USN record (record versions 2 and 3). Missing
/// journals, query failures, and unsupported record versions return `None`,
/// never a timestamp substitute. Linux and macOS keep no such journal and
/// return `None` without probing the path. `None` means "cannot prove the
/// file is unchanged".
pub fn file_change_marker(path: &Path) -> Option<FileChangeMarker> {
    native::file_change_marker(path).map(FileChangeMarker)
}

// ---------------------------------------------------------------------------
// Volume facts
// ---------------------------------------------------------------------------

/// The raw identity of the volume hosting `path`: `st_dev` on Unix, the
/// volume serial number on Windows. For callers whose persisted keys need
/// the raw value; compare values from the same host only. Follows symbolic
/// links. `None` when the path cannot be inspected.
pub fn volume_identity_u128(path: &Path) -> Option<u128> {
    native::volume_identity_u128(path)
}

/// Width in bits of the host's native file identifier: 128 on Windows
/// (NTFS/ReFS `FILE_ID_INFO`), 64 (the inode) on Linux and macOS.
pub const fn file_id_width() -> u32 {
    native::file_id_width()
}

/// Bytes of storage the file described by `metadata` actually occupies.
///
/// Unix reports allocated 512-byte blocks, so a sparse file can report less
/// than its length. Windows reports the compressed size
/// (`GetCompressedFileSizeW` on `path`), falling back to the logical length
/// when the volume cannot answer. Never fails.
pub fn allocated_bytes(path: &Path, metadata: &std::fs::Metadata) -> u64 {
    native::allocated_bytes(path, metadata)
}

// ---------------------------------------------------------------------------
// Paths and durability
// ---------------------------------------------------------------------------

/// Convert tool-emitted path bytes to a host path.
///
/// Unix preserves arbitrary bytes. Windows accepts UTF-8 only, because a
/// byte stream cannot represent native UTF-16 losslessly, and returns `None`
/// otherwise rather than converting lossily. This does not validate the path
/// for filesystem use (an embedded NUL is retained).
pub fn path_from_raw_bytes(bytes: &[u8]) -> Option<PathBuf> {
    native::path_from_raw_bytes(bytes)
}

/// Prepare a file path for direct native calls.
///
/// Windows canonicalizes the existing parent and appends the unchanged file
/// name, yielding an extended-length absolute path that escapes `MAX_PATH`
/// without requiring the final file to exist. Linux and macOS return the
/// path unchanged. Follows parent links; this is not a secure-open or a
/// lexical-only operation.
///
/// # Errors
///
/// On Windows, `InvalidInput` when `path` has no file name, or the parent's
/// canonicalization error.
pub fn native_call_path(path: &Path) -> io::Result<PathBuf> {
    native::native_call_path(path)
}

/// Flush `directory`'s entries to stable storage where the host exposes a
/// directory flush.
///
/// Linux and macOS open and fsync the directory. Windows has no directory
/// handle to flush and succeeds without resolving `directory`; unlike
/// [`sync_directory`](super::sync_directory), a missing directory is not an
/// error there. Callers whose durability protocol can use directory fsync
/// where it exists, but must stay portable to hosts without it, use this.
///
/// # Errors
///
/// On Linux and macOS, the open or fsync error.
pub fn sync_directory_if_supported(directory: &Path) -> io::Result<()> {
    native::sync_directory_if_supported(directory)
}
