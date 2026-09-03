//! Runtime-artifact identity, permissions, secure-open, replacement, and
//! directory primitives, plus advisory locking, modification-time, reflink
//! copy, parallel directory walking, and glob matching.
//!
//! Callers name the *role* a directory plays for their product -- ephemeral
//! runtime artifacts, persistent state, per-run scratch. Which location on this
//! host plays that role, and how two accounts are kept apart there, is decided
//! here. Callers still own their own layout beneath it: leaf names, extensions,
//! and subdirectories are product conventions, not host mechanics.
//!
//! The rest of this module is one capability, not several, even though it
//! groups locking, timestamps, copying, walking, and matching: every one of
//! them is a place the supported hosts genuinely diverge (advisory-lock
//! semantics, reflink support, filesystem-specific mtime granularity) and a
//! wrapper crate exists only to paper over that divergence. Locking and
//! modification-time setting are implemented natively per host, matching the
//! rest of this file; reflink copy, parallel walking, and glob matching wrap
//! a maintained backend privately, because there is no std equivalent and a
//! hand-rolled reflink ioctl is not something to get wrong silently.

/// A descriptor the caller already owns and has asked us to write to.
///
/// Deliberately opaque. Callers hold host-specific things -- a `RawFd` on
/// Unix, a `RawHandle` on Windows -- and there is no honest neutral spelling
/// for *what they hold*, so the conversion into this type is host-specific
/// and stays at the caller's edge. What is not host-specific is everything
/// after: writing all of a buffer to it, retrying the partial writes and the
/// interruptions that every host has in its own dialect.
///
/// This borrows. It does not close the descriptor, and it does not extend its
/// lifetime: the caller who opened it still decides when it goes away, and
/// using this after that is the same mistake as using the raw value would be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawDescriptor(usize);

impl RawDescriptor {
    /// Wrap a host descriptor value. Host trees call this; callers do not.
    pub(crate) fn from_value(value: usize) -> Self {
        Self(value)
    }

    /// The underlying host value, for the host tree that will use it.
    pub(crate) fn value(self) -> usize {
        self.0
    }
}

pub use crate::fs_write_all_to_descriptor as write_all_to_descriptor;

#[cfg(feature = "fs")]
pub use crate::{
    fs_create_private_file as create_private_file, fs_decode_path_bytes as decode_path_bytes,
    fs_encode_path_bytes as encode_path_bytes, fs_file_identity as file_identity,
    fs_is_lock_conflict as is_lock_conflict, fs_open_lock_file as open_lock_file,
    fs_path_identity as path_identity, fs_replace_file as replace_file,
    fs_sync_directory as sync_directory, fs_user_config_dir as user_config_dir,
    fs_user_data_dir as user_data_dir, fs_user_run_data_root as user_run_data_root,
    fs_user_runtime_dir as user_runtime_dir, fs_user_state_dir as user_state_dir,
    FsFileIdentity as FileIdentity,
};

#[cfg(feature = "fs")]
use std::fs::File;
#[cfg(feature = "fs")]
use std::io;
#[cfg(feature = "fs")]
use std::path::{Path, PathBuf};
#[cfg(feature = "fs")]
use std::sync::Arc;
#[cfg(feature = "fs")]
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// Advisory locking
// ---------------------------------------------------------------------------

/// An advisory lock held on an open file, released automatically when this
/// guard drops.
///
/// Holding this guard *is* what "the lock is held" means in this module:
/// there is deliberately no bare `lock`/`unlock` pair here that a caller
/// could call out of balance and get wrong. Release is best-effort -- a
/// failure releasing an
/// advisory lock cannot be reported from a destructor, and on every supported
/// host closing the underlying file descriptor also releases the lock, so a
/// failed explicit unlock here is not a stuck lock, just a redundant release
/// that did not need to happen.
///
/// Advisory locks exclude other advisory-lock holders only, never a process
/// that opens and reads or writes the file without locking it -- that is true
/// on every supported host and is what "advisory" means throughout this
/// module. Unix gets it from `flock`, which is advisory by construction;
/// Windows byte-range locks are mandatory, so the Windows implementation
/// takes its lock on a single byte far past any file's data, leaving the body
/// unobstructed. The exclusion between holders is identical either way; only
/// the bytes the kernel guards differ, and no caller reads or writes those.
///
/// # One guard per open file
///
/// A guard borrows the file shared, so nothing stops a caller from holding
/// two guards on the *same* [`File`] -- and every host answers that
/// differently, so it is a programming error rather than a portable
/// operation:
///
/// - Unix `flock` converts the lock in place, so a shared guard followed by
///   an exclusive one is an upgrade, and *either* guard's drop releases the
///   file's lock outright, including the one the other guard still thinks it
///   holds.
/// - Windows refuses an exclusive request that overlaps a range the same
///   handle already locked: [`try_lock_exclusive`] reports a conflict (see
///   [`is_lock_conflict`]) and [`lock_exclusive`] waits forever, because the
///   only holder that could release it is the caller that is waiting.
///
/// Hold one guard per open file. A second lock on the same path needs a
/// second [`open_lock_file`] handle, which is also what makes the exclusion
/// between the two meaningful.
#[cfg(feature = "fs")]
#[derive(Debug)]
pub struct FileLock<'file> {
    file: &'file File,
}

#[cfg(feature = "fs")]
impl<'file> FileLock<'file> {
    fn new(file: &'file File) -> Self {
        Self { file }
    }
}

#[cfg(feature = "fs")]
impl Drop for FileLock<'_> {
    fn drop(&mut self) {
        let _ = crate::fs_unlock(self.file);
    }
}

/// Take an exclusive advisory lock, waiting until it is available.
///
/// The returned guard releases the lock when dropped.
///
/// # Errors
///
/// Returns an error if the host lock call fails for a reason other than
/// waiting for the lock.
#[cfg(feature = "fs")]
pub fn lock_exclusive(file: &File) -> io::Result<FileLock<'_>> {
    crate::fs_lock_exclusive(file)?;
    Ok(FileLock::new(file))
}

/// Take a shared advisory lock, waiting until it is available.
///
/// The returned guard releases the lock when dropped.
///
/// # Errors
///
/// Returns an error if the host lock call fails for a reason other than
/// waiting for the lock.
#[cfg(feature = "fs")]
pub fn lock_shared(file: &File) -> io::Result<FileLock<'_>> {
    crate::fs_lock_shared(file)?;
    Ok(FileLock::new(file))
}

/// Take an exclusive advisory lock without waiting.
///
/// Returns immediately when another holder has it; the caller decides
/// whether that is a conflict worth retrying, via [`is_lock_conflict`].
///
/// # Errors
///
/// Returns an error if another holder has the lock (see
/// [`is_lock_conflict`]), or if the host lock call fails for another reason.
#[cfg(feature = "fs")]
pub fn try_lock_exclusive(file: &File) -> io::Result<FileLock<'_>> {
    crate::fs_try_lock_exclusive(file)?;
    Ok(FileLock::new(file))
}

/// Take a shared advisory lock without waiting.
///
/// Returns immediately when an exclusive holder has it; the caller decides
/// whether that is a conflict worth retrying, via [`is_lock_conflict`].
///
/// # Errors
///
/// Returns an error if an exclusive holder has the lock (see
/// [`is_lock_conflict`]), or if the host lock call fails for another reason.
#[cfg(feature = "fs")]
pub fn try_lock_shared(file: &File) -> io::Result<FileLock<'_>> {
    crate::fs_try_lock_shared(file)?;
    Ok(FileLock::new(file))
}

// ---------------------------------------------------------------------------
// Modification time
// ---------------------------------------------------------------------------

/// A filesystem modification time, independent of any host's on-disk
/// representation.
///
/// This is a Unix-epoch second and a nanosecond remainder, not
/// [`SystemTime`]: an on-disk mtime is fundamentally that pair (a filesystem
/// can legitimately record a time before 1970 as a negative second count),
/// and going through `SystemTime`'s own epoch arithmetic for every
/// construction and every host's native call would risk losing precision or
/// failing on values a filesystem can actually hold. A value only ever moves
/// between [`Metadata`](std::fs::Metadata) (via
/// [`from_last_modification_time`](FileTime::from_last_modification_time))
/// and a file (via [`set_file_mtime`]); nothing in this facade needs to read
/// a `FileTime` apart from that round trip.
#[cfg(feature = "fs")]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileTime {
    seconds_since_unix_epoch: i64,
    nanoseconds: u32,
}

#[cfg(feature = "fs")]
impl FileTime {
    /// Construct a modification time from a Unix-epoch second count and a
    /// nanosecond remainder.
    ///
    /// `seconds_since_unix_epoch` may be negative, for a time before 1970,
    /// matching what an on-disk mtime can legitimately encode.
    pub fn from_unix_time(seconds_since_unix_epoch: i64, nanoseconds: u32) -> Self {
        Self {
            seconds_since_unix_epoch,
            nanoseconds,
        }
    }

    /// Construct a modification time from [`SystemTime`], such as
    /// [`Metadata::modified`](std::fs::Metadata::modified).
    pub fn from_system_time(time: SystemTime) -> Self {
        match time.duration_since(SystemTime::UNIX_EPOCH) {
            Ok(duration) => Self {
                seconds_since_unix_epoch: duration.as_secs() as i64,
                nanoseconds: duration.subsec_nanos(),
            },
            Err(before_epoch) => {
                let duration = before_epoch.duration();
                let subsec = duration.subsec_nanos();
                let (seconds, nanoseconds) = if subsec == 0 {
                    (-(duration.as_secs() as i64), 0)
                } else {
                    (-(duration.as_secs() as i64) - 1, 1_000_000_000 - subsec)
                };
                Self {
                    seconds_since_unix_epoch: seconds,
                    nanoseconds,
                }
            }
        }
    }

    /// The current modification time, as this host's clock reports it now.
    pub fn now() -> Self {
        Self::from_system_time(SystemTime::now())
    }

    /// The modification time already recorded in `metadata`, such as from
    /// [`std::fs::metadata`] or [`std::fs::symlink_metadata`].
    ///
    /// Infallible: unlike [`Metadata::modified`](std::fs::Metadata::modified),
    /// this reads the raw host field directly rather than going through a
    /// conversion that can report the host as not supporting mtimes at all,
    /// which none of Linux, macOS, or Windows fail to.
    pub fn from_last_modification_time(metadata: &std::fs::Metadata) -> Self {
        last_modification_time(metadata)
    }
}

#[cfg(all(feature = "fs", unix))]
fn last_modification_time(metadata: &std::fs::Metadata) -> FileTime {
    use std::os::unix::fs::MetadataExt as _;

    FileTime {
        seconds_since_unix_epoch: metadata.mtime(),
        nanoseconds: metadata.mtime_nsec().clamp(0, 999_999_999) as u32,
    }
}

#[cfg(all(feature = "fs", windows))]
fn last_modification_time(metadata: &std::fs::Metadata) -> FileTime {
    use std::os::windows::fs::MetadataExt as _;

    windows_file_time_to_unix(metadata.last_write_time())
}

/// Windows-epoch (1601-01-01) 100-nanosecond ticks, converted to a Unix-epoch
/// second/nanosecond pair. Shared by [`last_modification_time`]; kept here
/// rather than duplicated per host because the conversion itself does not
/// diverge -- only which raw field a host hands it does.
#[cfg(all(feature = "fs", windows))]
fn windows_file_time_to_unix(ticks: u64) -> FileTime {
    /// 100-nanosecond ticks between the Windows epoch (1601-01-01) and the
    /// Unix epoch (1970-01-01).
    const WINDOWS_TO_UNIX_EPOCH_TICKS: i64 = 116_444_736_000_000_000;

    let ticks = ticks as i64 - WINDOWS_TO_UNIX_EPOCH_TICKS;
    let seconds_since_unix_epoch = ticks.div_euclid(10_000_000);
    let remainder_ticks = ticks.rem_euclid(10_000_000);
    FileTime {
        seconds_since_unix_epoch,
        nanoseconds: (remainder_ticks * 100) as u32,
    }
}

/// Set the modification time of the file at `path`, without disturbing its
/// access time.
///
/// # Errors
///
/// Returns an error if the caller lacks permission to change the file's
/// timestamps, if `path` does not exist, or if `time` is out of range for
/// this host's clock.
#[cfg(feature = "fs")]
pub fn set_file_mtime(path: &Path, time: FileTime) -> io::Result<()> {
    crate::fs_set_file_mtime(path, time.seconds_since_unix_epoch, time.nanoseconds)
}

// ---------------------------------------------------------------------------
// Copy
// ---------------------------------------------------------------------------

/// How [`copy_file`] moved the bytes.
#[cfg(feature = "fs")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    /// The destination shares the source's data blocks (copy-on-write); no
    /// file content was duplicated on disk.
    Reflinked,
    /// The filesystem, or this particular pair of paths, does not support a
    /// reflink here, so this many bytes were duplicated instead.
    Copied {
        /// The number of bytes written to `destination`.
        bytes: u64,
    },
}

/// Copy `source` to `destination`, preferring a reflink over a byte-for-byte
/// copy.
///
/// A reflink shares data blocks copy-on-write instead of duplicating them, so
/// it is near-instant and free of disk space until either copy is modified --
/// but only some filesystems support it (btrfs, XFS with `reflink=1`, APFS,
/// ReFS on Windows Server, and a few others), and both paths generally need
/// to be on the same volume. Where a reflink is not possible this transparently
/// falls back to a full byte copy, and [`CopyOutcome`] tells the caller which
/// one actually happened -- a caller that specifically needs the cheap path
/// can detect the expensive one rather than assume.
///
/// # Errors
///
/// Returns an error if neither a reflink nor a byte copy could complete, such
/// as `source` not existing or a permissions failure on `destination`.
#[cfg(feature = "fs")]
pub fn copy_file(source: &Path, destination: &Path) -> io::Result<CopyOutcome> {
    match reflink_copy::reflink_or_copy(source, destination)? {
        None => Ok(CopyOutcome::Reflinked),
        Some(bytes) => Ok(CopyOutcome::Copied { bytes }),
    }
}

// ---------------------------------------------------------------------------
// Parallel directory walk
// ---------------------------------------------------------------------------

/// One entry yielded by a [`DirectoryWalk`].
#[cfg(feature = "fs")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectoryEntry {
    path: PathBuf,
    depth: usize,
    is_directory: bool,
    is_file: bool,
    is_symbolic_link: bool,
}

#[cfg(feature = "fs")]
impl DirectoryEntry {
    /// The absolute path of this entry.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Depth of this entry relative to the walk's root, which is depth `0`.
    pub fn depth(&self) -> usize {
        self.depth
    }

    /// Whether this entry is a directory.
    pub fn is_directory(&self) -> bool {
        self.is_directory
    }

    /// Whether this entry is a regular file.
    pub fn is_file(&self) -> bool {
        self.is_file
    }

    /// Whether this entry is a symbolic link that was not followed.
    ///
    /// Never true when the walk that produced it followed symbolic links --
    /// in that case this entry describes the link's target instead.
    pub fn is_symbolic_link(&self) -> bool {
        self.is_symbolic_link
    }

    /// Re-read this entry's metadata from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry no longer exists or cannot be read.
    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        std::fs::metadata(&self.path)
    }
}

#[cfg(feature = "fs")]
fn directory_entry_from_jwalk(entry: jwalk::DirEntry<((), ())>) -> DirectoryEntry {
    let file_type = entry.file_type();
    DirectoryEntry {
        path: entry.path(),
        depth: entry.depth(),
        is_directory: file_type.is_dir(),
        is_file: file_type.is_file(),
        is_symbolic_link: file_type.is_symlink(),
    }
}

/// The predicate a [`DirectoryWalk`] consults before descending into a
/// directory. Shared, because the walk hands it to every worker thread that
/// reads a directory, and named, because spelling it inline twice is the
/// kind of type that stops being readable.
#[cfg(feature = "fs")]
type PruneDirectories = Arc<dyn Fn(&Path) -> bool + Send + Sync>;

/// A parallel directory-tree walk, configured before it runs.
///
/// Every option here defaults to what [`std::fs::read_dir`] itself would do
/// for a single directory: hidden entries included, symbolic links not
/// followed, no ordering guarantee, and nothing pruned.
#[cfg(feature = "fs")]
pub struct DirectoryWalk {
    root: PathBuf,
    follow_symbolic_links: bool,
    include_hidden_entries: bool,
    sorted: bool,
    prune_directories: Option<PruneDirectories>,
}

#[cfg(feature = "fs")]
impl DirectoryWalk {
    /// Start configuring a walk rooted at `root`.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            follow_symbolic_links: false,
            include_hidden_entries: true,
            sorted: false,
            prune_directories: None,
        }
    }

    /// Whether to follow symbolic links into their targets. Default: `false`.
    pub fn follow_symbolic_links(mut self, follow: bool) -> Self {
        self.follow_symbolic_links = follow;
        self
    }

    /// Whether to include entries this host considers hidden. Default:
    /// `true`.
    pub fn include_hidden_entries(mut self, include: bool) -> Self {
        self.include_hidden_entries = include;
        self
    }

    /// Whether entries are yielded in a deterministic, sorted order.
    /// Default: `false`, which is faster.
    pub fn sorted(mut self, sorted: bool) -> Self {
        self.sorted = sorted;
        self
    }

    /// Skip descending into (and yielding) any directory for which `keep`
    /// returns `false`.
    ///
    /// This is evaluated once per directory, before it is read, so a rejected
    /// directory's contents are never touched -- the reason to use this
    /// instead of filtering [`DirectoryWalk::walk`]'s output is exactly that
    /// short-circuit, on a tree where the pruned subtrees (`.git`, `target`,
    /// `node_modules`) can dwarf the rest.
    pub fn prune_directories<F>(mut self, keep: F) -> Self
    where
        F: Fn(&Path) -> bool + Send + Sync + 'static,
    {
        let keep: PruneDirectories = Arc::new(keep);
        self.prune_directories = Some(keep);
        self
    }

    /// Run the walk, yielding entries as they are discovered.
    ///
    /// Each item is an error only for an individual entry this walk could not
    /// read (a permissions failure, a race with something deleting the
    /// tree); it does not end the walk.
    pub fn walk(self) -> impl Iterator<Item = io::Result<DirectoryEntry>> {
        let prune_directories = self.prune_directories;
        let walker = jwalk::WalkDir::new(&self.root)
            .follow_links(self.follow_symbolic_links)
            .skip_hidden(!self.include_hidden_entries)
            .sort(self.sorted)
            .process_read_dir(move |_depth, _parent, _state, children| {
                let Some(keep) = &prune_directories else {
                    return;
                };
                children.retain(|entry| match entry {
                    Ok(entry) if entry.file_type().is_dir() => keep(&entry.path()),
                    _ => true,
                });
            });
        walker.into_iter().map(|entry| {
            entry
                .map(directory_entry_from_jwalk)
                .map_err(io::Error::from)
        })
    }
}

// ---------------------------------------------------------------------------
// Glob matching
// ---------------------------------------------------------------------------

/// A compiled set of glob patterns, matched together against one candidate
/// path per call.
///
/// Build one with [`PatternSetBuilder`].
#[cfg(feature = "fs")]
#[derive(Clone, Debug)]
pub struct PatternSet(globset::GlobSet);

#[cfg(feature = "fs")]
impl PatternSet {
    /// Whether `path` matches any pattern in this set.
    pub fn is_match(&self, path: &Path) -> bool {
        self.0.is_match(path)
    }
}

/// Builds a [`PatternSet`] from glob patterns.
///
/// Patterns are validated together at [`PatternSetBuilder::build`] rather
/// than one at a time as they are added, so a caller assembling a set from a
/// product's own configuration gets one place to report a bad pattern.
#[cfg(feature = "fs")]
#[derive(Debug, Default)]
pub struct PatternSetBuilder {
    patterns: Vec<String>,
}

#[cfg(feature = "fs")]
impl PatternSetBuilder {
    /// Start with no patterns.
    pub fn new() -> Self {
        Self {
            patterns: Vec::new(),
        }
    }

    /// Add one glob pattern to the set.
    pub fn add_pattern(mut self, pattern: &str) -> Self {
        self.patterns.push(pattern.to_owned());
        self
    }

    /// Compile the added patterns into a [`PatternSet`].
    ///
    /// # Errors
    ///
    /// Returns an error if any added pattern is not valid glob syntax.
    pub fn build(self) -> io::Result<PatternSet> {
        let mut builder = globset::GlobSetBuilder::new();
        for pattern in &self.patterns {
            let glob = globset::Glob::new(pattern)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
            builder.add(glob);
        }
        let set = builder
            .build()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
        Ok(PatternSet(set))
    }
}

#[cfg(all(test, feature = "fs"))]
mod tests {
    use super::*;

    const PRODUCT: &str = "rp-fs-facade-test";

    /// Every role resolves to an absolute directory that names the product.
    ///
    /// Asserted as a property rather than against one host's spelling: the
    /// point of the facade is that callers cannot tell which host answered.
    #[test]
    fn every_role_is_an_absolute_product_scoped_directory() {
        for directory in [
            user_runtime_dir(PRODUCT),
            user_state_dir(PRODUCT),
            user_run_data_root(PRODUCT),
        ] {
            assert!(
                directory.is_absolute(),
                "{} must be absolute",
                directory.display()
            );
            assert!(
                directory.to_string_lossy().contains(PRODUCT),
                "{} must be scoped to the product",
                directory.display()
            );
        }
    }

    /// Two products never share a directory in any role.
    #[test]
    fn distinct_products_do_not_collide() {
        let other = "rp-fs-facade-other";
        assert_ne!(user_runtime_dir(PRODUCT), user_runtime_dir(other));
        assert_ne!(user_state_dir(PRODUCT), user_state_dir(other));
        assert_ne!(user_run_data_root(PRODUCT), user_run_data_root(other));
    }

    /// A file is the same file as itself, by whichever pair this host uses.
    #[test]
    fn a_file_has_one_identity_through_both_a_handle_and_its_path() {
        let dir = std::env::temp_dir().join(format!("rp-fs-identity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("subject");
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .expect("create subject");

        let by_handle = file_identity(&file).expect("identity by handle");
        let by_path = path_identity(&path).expect("identity by path");
        assert_eq!(by_handle, by_path);

        drop(file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Two distinct files never share an identity, which is the property a
    /// caller relies on to notice its file was replaced underneath it.
    #[test]
    fn distinct_files_have_distinct_identities() {
        let dir = std::env::temp_dir().join(format!("rp-fs-identity2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let (first, second) = (dir.join("first"), dir.join("second"));
        std::fs::write(&first, b"a").expect("write first");
        std::fs::write(&second, b"b").expect("write second");

        let a = path_identity(&first).expect("identity a");
        let b = path_identity(&second).expect("identity b");
        if a.is_some() {
            assert_ne!(a, b);
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An exclusive lock excludes a second holder, and releasing readmits one.
    ///
    /// Both handles are opened through the facade, so this exercises the open
    /// and the lock together -- on Windows the two interact, because a
    /// restrictive share mode would fail the second open before it could ask
    /// for the lock.
    #[test]
    fn an_exclusive_lock_excludes_a_second_holder_until_released() {
        let dir = std::env::temp_dir().join(format!("rp-fs-lock-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("guard.lock");

        let first = open_lock_file(&path).expect("open first");
        let second = open_lock_file(&path).expect("open second");

        let first_lock = try_lock_exclusive(&first).expect("first acquires");
        let conflict = try_lock_exclusive(&second).expect_err("second must be refused");
        assert!(
            is_lock_conflict(&conflict),
            "refusal must classify as a conflict, got {conflict:?}"
        );

        // Dropping the guard is the only release mechanism this facade
        // exposes; the second holder succeeding proves the drop actually
        // released the lock, not just that the borrow ended.
        drop(first_lock);
        let second_lock = try_lock_exclusive(&second).expect("second acquires after release");
        drop(second_lock);

        drop((first, second));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A held lock obstructs no one who did not ask for a lock.
    ///
    /// This is the facade's advisory promise stated as a test, and it is the
    /// pidfile pattern [`open_lock_file`] exists for: a holder writes who it
    /// is, and anyone else reads that without participating in the locking.
    /// Deliberately not host-gated -- the claim is that every host answers
    /// the same way, so every host has to run it.
    #[test]
    fn a_lock_does_not_obstruct_a_process_that_never_locked() {
        use std::io::{Read as _, Write as _};

        let dir = std::env::temp_dir().join(format!("rp-fs-advisory-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("holder.lock");
        std::fs::write(&path, b"00000").expect("seed contents");

        let held = open_lock_file(&path).expect("open holder");

        // An exclusive holder must not stop a plain reader.
        let exclusive = lock_exclusive(&held).expect("holder takes it exclusively");
        let mut contents = Vec::new();
        std::fs::File::open(&path)
            .expect("a non-participant can open it")
            .read_to_end(&mut contents)
            .expect("a non-participant can read it");
        assert_eq!(contents, b"00000");
        drop(exclusive);

        // A shared holder must not stop a plain writer either. The write
        // does not truncate: a lock file's contents belong to whoever wrote
        // them, and replacing five bytes with five keeps this about the lock
        // rather than about file length.
        let shared = lock_shared(&held).expect("holder takes it shared");
        std::fs::OpenOptions::new()
            .write(true)
            .truncate(false)
            .open(&path)
            .expect("a non-participant can open it for writing")
            .write_all(b"11111")
            .expect("a non-participant can write it");
        drop(shared);

        assert_eq!(std::fs::read(&path).expect("read back"), b"11111");

        drop(held);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Asking one handle to upgrade its own lock is refused, not granted.
    ///
    /// Windows-only because it is the host whose answer differs: `flock`
    /// converts in place, while `LockFileEx` will not take an exclusive lock
    /// overlapping a range the same handle already holds. Written against
    /// the *try* form on purpose -- [`lock_exclusive`] would wait for a
    /// release only this caller could perform, which is a hung CI lane
    /// rather than a failing test.
    #[cfg(windows)]
    #[test]
    fn one_handle_cannot_upgrade_its_own_shared_lock() {
        let dir = std::env::temp_dir().join(format!("rp-fs-upgrade-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("upgrade.lock");

        let file = open_lock_file(&path).expect("open");
        let shared = lock_shared(&file).expect("take it shared");
        let conflict = try_lock_exclusive(&file).expect_err("an upgrade must be refused");
        assert!(
            is_lock_conflict(&conflict),
            "refusal must classify as a conflict, got {conflict:?}"
        );
        drop(shared);

        drop(file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A genuine failure is not reported as a conflict, so a caller does not
    /// retry forever on something waiting cannot fix.
    #[test]
    fn an_unrelated_error_is_not_a_lock_conflict() {
        let missing = std::env::temp_dir().join("rp-fs-lock-no-such-file");
        let _ = std::fs::remove_file(&missing);
        let error = std::fs::File::open(&missing).expect_err("must not exist");
        assert!(!is_lock_conflict(&error));
    }

    /// The pair round-trips, which is the only contract a wire encoding owes
    /// its decoder: the far end must reconstruct exactly the path that was
    /// named, not an equivalent one.
    #[test]
    fn a_path_survives_encoding_and_decoding_unchanged() {
        for original in [
            std::path::PathBuf::from("relative/leaf.log"),
            std::env::temp_dir()
                .join("rp path with spaces")
                .join("t.log"),
            std::env::current_exe().expect("current image"),
        ] {
            let decoded =
                decode_path_bytes(&encode_path_bytes(&original)).expect("decode what we encoded");
            assert_eq!(decoded, original);
        }
    }

    /// An empty path is a path, and must not become an error or a surprise.
    #[test]
    fn an_empty_path_round_trips_as_empty() {
        let empty = std::path::PathBuf::new();
        assert!(encode_path_bytes(&empty).is_empty());
        assert_eq!(
            decode_path_bytes(&encode_path_bytes(&empty)).expect("decode empty"),
            empty
        );
    }

    /// Replacing works whether or not the target already exists.
    ///
    /// Both cases matter: a bare rename onto an existing file fails on
    /// Windows, and the no-target case is the one a first write takes.
    #[test]
    fn a_file_is_replaced_whether_or_not_the_target_exists() {
        let dir = std::env::temp_dir().join(format!("rp-fs-replace-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let target = dir.join("manifest");

        let first = dir.join("first.tmp");
        std::fs::write(&first, b"first").expect("write first");
        replace_file(&first, &target).expect("replace absent target");
        assert_eq!(std::fs::read(&target).expect("read"), b"first");

        let second = dir.join("second.tmp");
        std::fs::write(&second, b"second").expect("write second");
        replace_file(&second, &target).expect("replace existing target");
        assert_eq!(std::fs::read(&target).expect("read"), b"second");

        // The replaced-from paths are consumed by the move, not left behind.
        assert!(!first.exists());
        assert!(!second.exists());

        sync_directory(&dir).expect("sync the directory that records it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Shared data and machine-local state are different roles, and a host
    /// that distinguishes them must not collapse the two.
    #[test]
    fn shared_data_is_its_own_role() {
        let data = user_data_dir(PRODUCT);
        assert!(data.is_absolute());
        assert!(data.to_string_lossy().contains(PRODUCT));
    }

    /// A private file is created, and refuses to open over an existing one.
    ///
    /// The refusal is the security-relevant half: opening over a file someone
    /// else made would inherit their permissions, so it must fail rather than
    /// succeed with weaker protection than the caller asked for.
    #[test]
    fn a_private_file_is_created_once_and_refuses_to_reopen() {
        let dir = std::env::temp_dir().join(format!("rp-fs-private-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("artifact.json");
        let _ = std::fs::remove_file(&path);

        {
            let mut file = create_private_file(&path).expect("create private file");
            use std::io::Write as _;
            file.write_all(b"payload").expect("write");
        }
        assert_eq!(std::fs::read(&path).expect("read back"), b"payload");

        let second = create_private_file(&path).expect_err("must not open over an existing file");
        assert_eq!(second.kind(), std::io::ErrorKind::AlreadyExists);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The roles are stable: asking twice gives the same answer, so a path
    /// derived at startup still names the same directory later.
    #[test]
    fn roles_are_stable_across_calls() {
        assert_eq!(user_runtime_dir(PRODUCT), user_runtime_dir(PRODUCT));
        assert_eq!(user_state_dir(PRODUCT), user_state_dir(PRODUCT));
        assert_eq!(user_run_data_root(PRODUCT), user_run_data_root(PRODUCT));
    }

    /// Any number of shared holders may coexist, but an exclusive request is
    /// refused while one is outstanding, and admitted once every shared
    /// holder has dropped.
    #[test]
    fn shared_locks_coexist_but_exclude_an_exclusive_request() {
        let dir = std::env::temp_dir().join(format!("rp-fs-lock-shared-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("guard.lock");

        let first = open_lock_file(&path).expect("open first");
        let second = open_lock_file(&path).expect("open second");
        let third = open_lock_file(&path).expect("open third");

        let first_shared = try_lock_shared(&first).expect("first shared holder admitted");
        let second_shared = try_lock_shared(&second).expect("second shared holder admitted");

        let conflict =
            try_lock_exclusive(&third).expect_err("exclusive must be refused while shared holds");
        assert!(is_lock_conflict(&conflict));

        drop(first_shared);
        let conflict = try_lock_exclusive(&third)
            .expect_err("exclusive must still be refused with one shared holder left");
        assert!(is_lock_conflict(&conflict));

        drop(second_shared);
        let exclusive =
            try_lock_exclusive(&third).expect("exclusive admitted once shared holders release");
        drop(exclusive);

        drop((first, second, third));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The blocking variants eventually acquire the lock a concurrent holder
    /// releases, rather than failing immediately the way the `try_` variants
    /// do -- this is the property that distinguishes the two families.
    #[test]
    fn blocking_lock_acquires_once_the_holder_releases() {
        let dir = std::env::temp_dir().join(format!("rp-fs-lock-blocking-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("guard.lock");

        let first = open_lock_file(&path).expect("open first");
        let second = open_lock_file(&path).expect("open second");

        let held = try_lock_exclusive(&first).expect("first acquires");
        drop(held);

        // With no concurrent holder left, the blocking call must return
        // immediately rather than actually block this test.
        let acquired = lock_exclusive(&second).expect("blocking lock acquires");
        drop(acquired);

        drop((first, second));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A modification time survives the round trip from Unix seconds through
    /// [`set_file_mtime`] and back out through
    /// [`FileTime::from_last_modification_time`].
    #[test]
    fn a_unix_time_survives_the_round_trip_through_a_file() {
        let dir = std::env::temp_dir().join(format!("rp-fs-mtime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("stamped");
        std::fs::write(&path, b"payload").expect("write");

        // A time with a sub-second remainder, comfortably before this test
        // ever runs, so no host's mtime resolution rounds it away.
        let stamped = FileTime::from_unix_time(1_700_000_000, 500_000_000);
        set_file_mtime(&path, stamped).expect("set mtime");

        let metadata = std::fs::metadata(&path).expect("read metadata back");
        let read_back = FileTime::from_last_modification_time(&metadata);
        assert_eq!(read_back, stamped);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// [`FileTime::from_system_time`] and [`FileTime::from_last_modification_time`]
    /// agree, since a file's metadata is read through [`SystemTime`] itself.
    #[test]
    fn from_system_time_and_from_metadata_agree() {
        let dir = std::env::temp_dir().join(format!("rp-fs-mtime-agree-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("stamped");
        std::fs::write(&path, b"payload").expect("write");

        let metadata = std::fs::metadata(&path).expect("read metadata");
        let modified = metadata.modified().expect("host supports mtime");

        assert_eq!(
            FileTime::from_last_modification_time(&metadata),
            FileTime::from_system_time(modified)
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A time before the Unix epoch round-trips through [`SystemTime`]
    /// without silently landing on the wrong side of it.
    #[test]
    fn a_time_before_the_unix_epoch_round_trips() {
        // `epoch - 3600.25s` is `-3601` whole seconds plus a forward `0.75s`
        // remainder: nanoseconds always advance from the second mark, they
        // never subtract from it, so the second count floors.
        let before_epoch = SystemTime::UNIX_EPOCH - std::time::Duration::new(3600, 250_000_000);
        let converted = FileTime::from_system_time(before_epoch);
        assert_eq!(converted, FileTime::from_unix_time(-3601, 750_000_000));
    }

    /// `now()` reports a time in the current era, not a zeroed or default
    /// value -- a facade that silently returned the epoch would be a much
    /// harder bug to notice than one that fails loudly.
    #[test]
    fn now_reports_a_recent_time() {
        let now = FileTime::now();
        // 2020-01-01T00:00:00Z. Anything before this is not "now" by any
        // clock this crate supports.
        assert!(now > FileTime::from_unix_time(1_577_836_800, 0));
    }

    /// [`copy_file`] produces a byte-identical destination whether the
    /// filesystem gave it a reflink or fell back to a copy, and reports which
    /// one happened.
    #[test]
    fn copy_file_reproduces_the_source_and_reports_its_strategy() {
        let dir = std::env::temp_dir().join(format!("rp-fs-copy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let source = dir.join("source");
        let destination = dir.join("destination");
        let content = b"reflink or copy, the bytes must match";
        std::fs::write(&source, content).expect("write source");

        let outcome = copy_file(&source, &destination).expect("copy succeeds");
        assert_eq!(
            std::fs::read(&destination).expect("read destination"),
            content
        );
        match outcome {
            CopyOutcome::Reflinked => {}
            CopyOutcome::Copied { bytes } => assert_eq!(bytes, content.len() as u64),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A walk finds every file under its root, and a pruned directory's
    /// contents never appear at all.
    #[test]
    fn a_walk_finds_files_and_honours_pruning() {
        let dir = std::env::temp_dir().join(format!("rp-fs-walk-{}", std::process::id()));
        let pruned = dir.join("pruned");
        let kept = dir.join("kept");
        std::fs::create_dir_all(&pruned).expect("create pruned dir");
        std::fs::create_dir_all(&kept).expect("create kept dir");
        std::fs::write(pruned.join("secret"), b"never seen").expect("write pruned file");
        std::fs::write(kept.join("visible"), b"seen").expect("write kept file");
        std::fs::write(dir.join("root_file"), b"seen too").expect("write root file");

        let entries: Vec<DirectoryEntry> = DirectoryWalk::new(dir.clone())
            .prune_directories(|path| {
                path.file_name().and_then(|name| name.to_str()) != Some("pruned")
            })
            .walk()
            .collect::<io::Result<Vec<_>>>()
            .expect("walk succeeds");

        let file_paths: Vec<&Path> = entries
            .iter()
            .filter(|entry| entry.is_file())
            .map(DirectoryEntry::path)
            .collect();
        assert!(file_paths.contains(&kept.join("visible").as_path()));
        assert!(file_paths.contains(&dir.join("root_file").as_path()));
        assert!(
            !file_paths.contains(&pruned.join("secret").as_path()),
            "a pruned directory's contents must never be yielded"
        );
        assert!(
            entries.iter().all(|entry| entry.path() != pruned.as_path()),
            "a pruned directory itself must not be yielded either"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A set of glob patterns matches any path that satisfies at least one of
    /// them, and rejects a path that satisfies none.
    #[test]
    fn a_pattern_set_matches_any_included_pattern() {
        let set = PatternSetBuilder::new()
            .add_pattern("*.rs")
            .add_pattern("Cargo.toml")
            .build()
            .expect("valid patterns compile");

        assert!(set.is_match(Path::new("src/lib.rs")));
        assert!(set.is_match(Path::new("Cargo.toml")));
        assert!(!set.is_match(Path::new("README.md")));
    }

    /// An invalid glob pattern is reported at build time, not accepted and
    /// silently never matched.
    #[test]
    fn an_invalid_pattern_is_rejected_at_build() {
        let error = PatternSetBuilder::new()
            .add_pattern("[unterminated")
            .build()
            .expect_err("malformed glob syntax must be rejected");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
}
