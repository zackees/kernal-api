//! Native offset-write compatibility without a seek-plus-write fallback.

/// Perform one native write at the supplied offset, returning bytes written.
/// Short writes and interrupted-operation errors are returned to the caller;
/// this does not retry or flush. Open-file flags retain their native semantics.
///
/// Unix uses `FileExt::write_at`; Windows uses `FileExt::seek_write`.
/// Cursor behavior is host-defined: callers must not rely on a portable shared
/// cursor position after this operation, or interleave cursor-based I/O.
pub fn write_at(file: &std::fs::File, buf: &[u8], offset: u64) -> std::io::Result<usize> {
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::write_at(file, buf, offset)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_write(file, buf, offset)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (file, buf, offset);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "native offset writes unavailable",
        ))
    }
}
