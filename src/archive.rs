//! Bounded archive extraction. Callers own staging publication and cleanup.
//!
//! The destination must be absent or empty and remain exclusively controlled
//! by the caller throughout extraction. Failure may leave partial output.

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;

struct MetadataBudget {
    file: File,
    remaining: Rc<Cell<u64>>,
}

impl Read for MetadataBudget {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        let allowed = self.remaining.get().min(buffer.len() as u64) as usize;
        if allowed == 0 {
            return Err(invalid("ZIP metadata read budget exhausted"));
        }
        let count = self.file.read(&mut buffer[..allowed])?;
        self.remaining.set(self.remaining.get() - count as u64);
        Ok(count)
    }
}

impl Seek for MetadataBudget {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.file.seek(position)
    }
}

fn copy_bounded(
    reader: &mut impl Read,
    writer: &mut impl Write,
    remaining: &mut u64,
) -> io::Result<()> {
    let mut buffer = [0; 64 * 1024];
    loop {
        let capacity = remaining.saturating_add(1).min(buffer.len() as u64) as usize;
        let count = match reader.read(&mut buffer[..capacity]) {
            Ok(0) => return Ok(()),
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        if count as u64 > *remaining {
            return Err(invalid("archive output exceeds byte limit"));
        }
        writer.write_all(&buffer[..count])?;
        *remaining -= count as u64;
    }
}

/// Supported archive encoding.
#[derive(Clone, Copy, Debug)]
pub enum ArchiveFormat {
    /// Standard ZIP, including ZIP64. Self-extracting prefixes are unsupported.
    Zip,
}

/// Resource ceilings, checked before allocation or output where possible.
#[derive(Clone, Copy, Debug)]
pub struct ExtractionLimits {
    /// Maximum source archive size.
    pub max_input_bytes: u64,
    /// Maximum sum of extracted file bytes.
    pub max_output_bytes: u64,
    /// Maximum archive entries.
    pub max_entries: u64,
    /// Maximum central-directory metadata bytes.
    pub max_metadata_bytes: u64,
    /// Maximum entry or link path encoding length.
    pub max_path_bytes: usize,
}

impl Default for ExtractionLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 16 * 1024 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024 * 1024,
            max_entries: 1_000_000,
            max_metadata_bytes: 64 * 1024 * 1024,
            max_path_bytes: 4096,
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn relative_path(name: &str, limit: usize) -> io::Result<PathBuf> {
    if name.len() > limit || name.contains(['\\', ':', '\0']) {
        return Err(invalid("unsafe or oversized archive path"));
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid("archive path escapes destination"));
    }
    let path: PathBuf = path
        .components()
        .filter(|c| matches!(c, Component::Normal(_)))
        .collect();
    if path.as_os_str().is_empty() {
        return Err(invalid("empty archive path"));
    }
    Ok(path)
}

fn prepare_destination(dest: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(dest) {
        Ok(meta) if !meta.is_dir() || meta.file_type().is_symlink() => {
            return Err(invalid("destination is not a real directory"))
        }
        Ok(_) if fs::read_dir(dest)?.next().is_some() => {
            return Err(invalid("destination is not empty"))
        }
        Ok(_) => (),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(dest)?,
        Err(error) => return Err(error),
    }
    fs::canonicalize(dest)
}

/// Extract an archive under explicit ceilings. Does not promote wrapper dirs.
pub fn extract(
    archive: &Path,
    dest: &Path,
    format: ArchiveFormat,
    limits: ExtractionLimits,
) -> io::Result<()> {
    let file = File::open(archive)?;
    if file.metadata()?.len() > limits.max_input_bytes {
        return Err(invalid("archive input exceeds byte limit"));
    }
    match format {
        ArchiveFormat::Zip => extract_zip(file, dest, limits),
    }
}

// Check advertised ZIP/ZIP64 metadata before the backend sizes its entry vec.
fn preflight_zip(file: &mut File, limits: ExtractionLimits) -> io::Result<()> {
    let length = file.metadata()?.len();
    let tail_length = length.min(65557) as usize;
    file.seek(SeekFrom::End(-(tail_length as i64)))?;
    let mut tail = vec![0; tail_length];
    file.read_exact(&mut tail)?;
    let offset = (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&i| {
            tail[i..].starts_with(b"PK\x05\x06")
                && i + 22 + u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize == tail.len()
        })
        .ok_or_else(|| invalid("missing ZIP end directory"))?;
    let end = &tail[offset..];
    if end[4..8] != [0; 4] {
        return Err(invalid("multi-disk ZIP is unsupported"));
    }
    let mut count = u16::from_le_bytes([end[10], end[11]]) as u64;
    let mut metadata_size = u32::from_le_bytes(end[12..16].try_into().unwrap()) as u64;
    if count == u16::MAX as u64 || metadata_size == u32::MAX as u64 {
        let position = length - tail_length as u64 + offset as u64;
        if position < 20 {
            return Err(invalid("missing ZIP64 locator"));
        }
        file.seek(SeekFrom::Start(position - 20))?;
        let mut locator = [0; 20];
        file.read_exact(&mut locator)?;
        if !locator.starts_with(b"PK\x06\x07") {
            return Err(invalid("bad ZIP64 locator"));
        }
        let record = u64::from_le_bytes(locator[8..16].try_into().unwrap());
        file.seek(SeekFrom::Start(record))?;
        let mut header = [0; 56];
        file.read_exact(&mut header)?;
        if !header.starts_with(b"PK\x06\x06") {
            return Err(invalid("bad ZIP64 directory"));
        }
        count = u64::from_le_bytes(header[32..40].try_into().unwrap());
        metadata_size = u64::from_le_bytes(header[40..48].try_into().unwrap());
    }
    if count > limits.max_entries || metadata_size > limits.max_metadata_bytes {
        return Err(invalid("ZIP metadata exceeds resource limits"));
    }
    file.rewind()
}

fn extract_zip(mut file: File, dest: &Path, limits: ExtractionLimits) -> io::Result<()> {
    preflight_zip(&mut file, limits)?;
    let budget = Rc::new(Cell::new(limits.max_metadata_bytes.saturating_add(65557)));
    let reader = MetadataBudget {
        file,
        remaining: Rc::clone(&budget),
    };
    let mut archive = zip::ZipArchive::new(reader).map_err(io::Error::other)?;
    // Metadata parsing is complete. File data is bounded by compressed file
    // size and the independent decompressed-output counter below.
    budget.set(u64::MAX);
    if archive.len() as u64 > limits.max_entries {
        return Err(invalid("too many archive entries"));
    }
    let root = prepare_destination(dest)?;
    let mut remaining = limits.max_output_bytes;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(io::Error::other)?;
        let relative = relative_path(entry.name(), limits.max_path_bytes)?;
        let output = root.join(relative);
        if entry.is_symlink() {
            return Err(invalid("ZIP symlink extraction is not yet implemented"));
        }
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        if entry.size() > remaining {
            return Err(invalid("archive output exceeds byte limit"));
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut target = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output)?;
        copy_bounded(&mut entry, &mut target, &mut remaining)?;
        target.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&output, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}
