//! Linux links: symlink classification, creation/removal, and archive
//! extraction (tar handles symlinks natively here).

use std::io::Read;
use std::path::Path;

/// Unpack `archive` into `dest`. Tar creates symlinks natively on Linux;
/// the `filter` (when present) skips matching entries.
pub(super) fn unpack_archive_entries<R, F>(
    archive: &mut tar::Archive<R>,
    dest: &Path,
    mut filter: Option<F>,
) -> std::io::Result<()>
where
    R: Read,
    F: FnMut(&Path) -> std::io::Result<bool>,
{
    match filter.as_mut() {
        None => archive.unpack(dest),
        Some(filter) => {
            for entry in archive.entries()? {
                let mut entry = entry?;
                let path = entry.path()?.into_owned();
                if filter(&path)? {
                    std::io::copy(&mut entry, &mut std::io::sink())?;
                    continue;
                }
                entry.unpack_in(dest)?;
            }
            Ok(())
        }
    }
}
