//! Journal-backed change observations, separate from file identity.

/// Opaque Windows USN sequence observation.
///
/// Compare only observations of the same file in the same journal epoch.
/// This is not a content hash, file identity, or proof against journal reset,
/// concurrent writes, or path replacement. Unsupported/unreadable observations
/// are represented by None, never by a timestamp substitute.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FileChangeMarker(i128);

#[cfg(windows)]
#[path = "change_marker_windows.rs"]
mod native;

/// Query a path's Windows USN record (versions 2/3). Missing journals, query
/// failures and unsupported record versions return None. Linux/macOS return
/// None without probing the path. Paths are followed; no handle is retained.
#[cfg(windows)]
pub use native::file_change_marker;

/// No journal-backed observation is implemented on this host.
#[cfg(not(windows))]
pub fn file_change_marker(_path: &std::path::Path) -> Option<FileChangeMarker> {
    None
}
