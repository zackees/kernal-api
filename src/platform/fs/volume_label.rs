//! Legacy volume labels, distinct from canonical opaque volume identity.
use std::path::Path;

/// Return a compatibility label: Unix decimal device id or Windows uppercase
/// drive letter. Canonicalization is best-effort. Windows UNC paths without a
/// drive letter return None; a drive spelling need not exist. This is not a
/// durable unique volume identifier or proof that two paths share storage.
pub fn volume_label(path: &Path) -> Option<String> {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(std::fs::metadata(&canonical).ok()?.dev().to_string())
    }
    #[cfg(windows)]
    {
        let value = canonical.to_string_lossy();
        let trimmed = value.trim_start_matches(r"\\?\");
        let bytes = trimmed.as_bytes();
        if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            Some((bytes[0] as char).to_ascii_uppercase().to_string())
        } else {
            None
        }
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = canonical;
        None
    }
}
