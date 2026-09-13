//! Streaming tar compatibility for trusted, caller-controlled staging trees.
//! Unlike bounded extraction this API imposes no resource ceilings. Callers
//! must exclusively control the destination and validate archive provenance.
//! Windows replay retains legacy all-error copy fallback and follows resolved
//! filesystem links: lexical checks are not a race-safe containment guarantee.

use std::io::{self, Read};
use std::path::Path;

#[cfg(unix)]
#[path = "tar_stream_unix.rs"]
mod native;
#[cfg(windows)]
#[path = "tar_stream_windows.rs"]
mod native;

/// Extract a tar reader with the backend's standard archive options.
/// A filter returning true skips an entry and drains its payload; callback
/// errors abort extraction, which may leave partial output. None retains the
/// native unfiltered extraction path, including its directory metadata order.
/// Windows defers symbolic links until regular entries have been extracted.
/// No tar implementation type crosses this API boundary.
///
/// # Trust and resource requirements
///
/// This compatibility API is for trusted archives and exclusively controlled
/// staging trees. It imposes no byte, entry, recursion, or link-expansion limit.
/// Windows copies targets after any link creation/verification failure, not
/// only privilege failures. Copy traversal follows filesystem links and has no
/// cycle detection; existing links can redirect reads outside the lexical root.
/// Do not use it as a sandbox for untrusted archives or destinations. Callers
/// needing bounded extraction should use the separate `archive` capability.
pub fn extract_tar_stream<R, F>(reader: R, dest: &Path, filter: Option<F>) -> io::Result<()>
where
    R: Read,
    F: FnMut(&Path) -> io::Result<bool>,
{
    let mut archive = tar::Archive::new(reader);
    native::unpack_archive_entries(&mut archive, dest, filter)
}
