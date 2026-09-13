//! Native replacement compatibility operations for staged artifacts.
//!
//! Paths and parent directories must be caller-controlled. These operations
//! are not secure-open transactions or cross-filesystem copy fallbacks.

#[cfg(target_os = "linux")]
#[path = "replacement_linux.rs"]
mod native;
#[cfg(target_os = "macos")]
#[path = "replacement_macos.rs"]
mod native;
#[cfg(windows)]
#[path = "replacement_win.rs"]
mod native;

/// Replace a file by native rename: Unix rename, or Windows MoveFileExW with
/// REPLACE_EXISTING and WRITE_THROUGH. Windows retries permission/share errors
/// on a fixed 50/100/250/500 ms delay ladder (five attempts total).
/// Success consumes the source path; this does not add Unix fsync durability.
pub use native::atomic_replace;

/// Rename a generation using host-native collision behavior. Windows refuses
/// an existing destination and applies the AV retry ladder. Unix uses rename
/// and MAY REPLACE an existing destination. This is not portable no-clobber.
pub use native::rename_generation;

/// Unix rename; Windows first tries atomic_replace, then, after ANY error
/// when the destination exists, deletes that file and retries generation
/// rename. This opt-in compatibility path is non-atomic and may lose the old
/// destination even when the final rename fails (including missing source).
pub use native::replace_with_delete_fallback;

/// Install a staged directory. Existing Unix destinations use native atomic
/// exchange and then remove the old tree; unsupported exchange returns error.
/// Windows renames the old tree to a backup, installs the new tree, and removes
/// the backup. Failure attempts rollback but does not guarantee it succeeds.
/// Cleanup failure can return an error after the new directory is installed.
/// No crash-durability or concurrent-installer isolation is promised.
pub use native::install_directory;

/// Windows native errors 5 or 32 only; always false on Unix.
/// Unlike the internal AV retry rule, synthetic PermissionDenied is false.
pub use native::is_transient_share_error;

/// Windows native error 33 only; always false on Unix. This narrow legacy
/// classifier is distinct from the broader fs::is_lock_contention API.
pub use native::is_lock_contention;
