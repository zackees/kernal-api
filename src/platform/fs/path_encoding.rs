//! Native path encoding boundaries.
use std::path::{Path, PathBuf};

/// Convert compiler-emitted bytes to a host path. Unix preserves arbitrary
/// bytes; Windows accepts UTF-8, not lossy conversion to UTF-16. This does not
/// validate a path for filesystem use (embedded NUL is retained).
pub fn path_from_raw_bytes(bytes: &[u8]) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        Some(std::ffi::OsString::from_vec(bytes.to_vec()).into())
    }
    #[cfg(windows)]
    {
        std::str::from_utf8(bytes).ok().map(PathBuf::from)
    }
}

/// Prepare a file path for direct native calls. On Windows, canonicalize the
/// existing parent and append the unchanged filename, yielding an extended
/// absolute path without requiring the final file to exist. Unix is a no-op.
/// This follows parent links and is not a secure-open or lexical-only operation.
pub fn native_call_path(path: &Path) -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    {
        let name = path.file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("path has no filename: {}", path.display()),
            )
        })?;
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        Ok(std::fs::canonicalize(parent)?.join(name))
    }
    #[cfg(unix)]
    {
        Ok(path.to_path_buf())
    }
}
