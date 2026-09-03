//! Windows per-user directory placement for product runtime artifacts.

use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

/// Directory for `product`'s ephemeral runtime artifacts (pid files, run data).
///
/// `LOCALAPPDATA` is per-user and non-roaming, which is what machine-local
/// artifacts want: a roaming profile would carry another machine's state here.
/// Sockets are not placed by this — Windows named pipes live in a kernel
/// namespace with no directory at all.
pub fn user_runtime_dir(product: &str) -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join(product)
}

/// Directory for `product`'s persistent state (databases that outlive a boot).
///
/// Windows draws no line between runtime and state locations; both are
/// per-user under `LOCALAPPDATA`.
pub fn user_state_dir(product: &str) -> PathBuf {
    user_runtime_dir(product)
}

/// Root under which `product` keeps per-run scratch data.
pub fn user_run_data_root(product: &str) -> PathBuf {
    user_runtime_dir(product)
}

/// Stable identity of an open file on this host.
///
/// Two paths that resolve to the same bytes on disk report the same identity,
/// which is what lets a caller notice that the file it opened has since been
/// replaced. The two fields are whatever this host uses to say that: a device
/// and inode, a volume serial and file index, or an equivalent pair. Callers
/// compare them; they do not interpret them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileIdentity {
    /// Device, volume, or platform-equivalent file namespace.
    pub device: u64,
    /// Inode, file index, or platform-equivalent file number.
    pub file: u64,
}

/// Identity of an already-open file.
pub fn file_identity(file: &File) -> io::Result<Option<FileIdentity>> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawHandle;
    use winapi::um::fileapi::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION};
    use winapi::um::winnt::HANDLE;

    let mut info = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    let result =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, info.as_mut_ptr()) };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }

    let info = unsafe { info.assume_init() };
    Ok(Some(FileIdentity {
        device: info.dwVolumeSerialNumber as u64,
        file: ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
    }))
}

/// Identity of the file a path currently names.
///
/// Windows answers this from an open handle, so the file is opened here. The
/// share mode is permissive on purpose: asking who a file is must not evict a
/// writer that already holds it.
pub fn path_identity(path: &Path) -> io::Result<Option<FileIdentity>> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use winapi::um::winnt::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    file_identity(&file)
}

/// Open `path` for use as an advisory lock file, creating it if absent.
///
/// The share mode is permissive on purpose: exclusion must come from the lock,
/// not from the open, or a second opener fails before it can even ask.
pub fn open_lock_file(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use winapi::um::winnt::{FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE};

    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        // Never truncate: an existing lock file may be held right now,
        // and its contents are not ours to clear.
        .truncate(false)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

/// The offset of the one byte every lock in this module is taken on.
///
/// A `LockFileEx` range is *mandatory*: the kernel denies `ReadFile` and
/// `WriteFile` from every other handle inside it, not just competing lock
/// requests. Locking the whole file would therefore make this module's
/// advisory promise false on this host alone -- a second process that opened
/// the lock file only to read who holds it would fail with
/// `ERROR_LOCK_VIOLATION`, and a shared holder would block every
/// non-participating writer. Confining the lock to a single byte far outside
/// any real file's data keeps the exclusion between lock holders exactly as
/// it was and leaves the file body readable and writable, which is what the
/// facade means by "advisory" and the same trick SQLite uses for its
/// cross-platform locking.
///
/// `1 << 62` is the offset because it is past anything a filesystem can hold
/// while still positive when the kernel reads it as the signed
/// `LARGE_INTEGER` that `NtLockFile` takes underneath `LockFileEx`. Locking a
/// range beyond end-of-file is legal and does not extend the file.
const LOCK_BYTE_OFFSET: u64 = 1 << 62;

/// Fill the `OVERLAPPED` offset pair that names [`LOCK_BYTE_OFFSET`].
///
/// Both `LockFileEx` and `UnlockFileEx` take the range's start this way and
/// its length in the two `DWORD` arguments, so the pair has to agree between
/// them or a release silently fails with `ERROR_NOT_LOCKED`.
fn lock_byte_overlapped() -> winapi::um::minwinbase::OVERLAPPED {
    use std::mem;
    use winapi::um::minwinbase::OVERLAPPED;

    let mut overlapped: OVERLAPPED = unsafe { mem::zeroed() };
    let offsets = unsafe { overlapped.u.s_mut() };
    offsets.Offset = LOCK_BYTE_OFFSET as u32;
    offsets.OffsetHigh = (LOCK_BYTE_OFFSET >> 32) as u32;
    overlapped
}

/// Take or wait for a lock on [`LOCK_BYTE_OFFSET`], with the given
/// `LockFileEx` flags. Private: every public lock entry point below is one
/// flag combination of this call.
fn lock_file(file: &File, flags: winapi::shared::minwindef::DWORD) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;
    use winapi::um::fileapi::LockFileEx;
    use winapi::um::winnt::HANDLE;

    let mut overlapped = lock_byte_overlapped();
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            flags,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Take an exclusive advisory lock without waiting.
///
/// This host refuses an exclusive request that overlaps a range the *same*
/// handle already locked, so a caller that still holds a shared lock on this
/// file gets a conflict here rather than an upgrade. That is the one place
/// the hosts genuinely differ, and the facade documents it as a rule callers
/// keep: one lock guard per open file.
pub fn try_lock_exclusive(file: &File) -> io::Result<()> {
    use winapi::um::minwinbase::{LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY};

    lock_file(file, LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY)
}

/// Take an exclusive advisory lock, waiting until it is available.
///
/// The waiting form of the same refusal [`try_lock_exclusive`] describes:
/// asked to upgrade a lock the same handle already holds, this waits for a
/// release that only this caller could perform, and so never returns.
pub fn lock_exclusive(file: &File) -> io::Result<()> {
    use winapi::um::minwinbase::LOCKFILE_EXCLUSIVE_LOCK;

    lock_file(file, LOCKFILE_EXCLUSIVE_LOCK)
}

/// Take a shared advisory lock, waiting until it is available.
///
/// A `LockFileEx` call without `LOCKFILE_EXCLUSIVE_LOCK` set is what this
/// host spells "shared" -- there is no separate shared-lock flag.
pub fn lock_shared(file: &File) -> io::Result<()> {
    lock_file(file, 0)
}

/// Take a shared advisory lock without waiting.
///
/// Returns immediately when an exclusive holder has it; the caller decides
/// whether that is a conflict worth retrying, via [`is_lock_conflict`].
pub fn try_lock_shared(file: &File) -> io::Result<()> {
    use winapi::um::minwinbase::LOCKFILE_FAIL_IMMEDIATELY;

    lock_file(file, LOCKFILE_FAIL_IMMEDIATELY)
}

/// Release a lock taken by [`try_lock_exclusive`], [`try_lock_shared`],
/// [`lock_exclusive`], or [`lock_shared`].
///
/// The range must be the one `lock_file` took, byte for byte: this host
/// releases a named range, not "whatever this handle holds", and a mismatched
/// range fails with `ERROR_NOT_LOCKED` while the lock stays held.
pub fn unlock(file: &File) -> io::Result<()> {
    use std::os::windows::io::AsRawHandle as _;
    use winapi::um::fileapi::UnlockFileEx;
    use winapi::um::winnt::HANDLE;

    let mut overlapped = lock_byte_overlapped();
    let result = unsafe { UnlockFileEx(file.as_raw_handle() as HANDLE, 0, 1, 0, &mut overlapped) };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Whether `error` means "someone else holds it", rather than a real failure.
pub fn is_lock_conflict(error: &io::Error) -> bool {
    use winapi::shared::winerror::ERROR_LOCK_VIOLATION;

    error.raw_os_error() == Some(ERROR_LOCK_VIOLATION as i32)
}

/// Set the modification time of the file at `path`, without disturbing its
/// access or creation time.
///
/// Opened with only `FILE_WRITE_ATTRIBUTES`, not the read/write access a
/// plain open would request: Windows denies an attribute-only change to a
/// handle opened for full write access to a file this process has marked
/// read-only, but grants it to a handle that only asked for attributes --
/// and content-addressed cache entries are commonly left read-only. From
/// there this is the same stable `File::set_modified` every caller of this
/// crate could use directly; the access mode is the only part that needed
/// deciding per host.
pub fn set_file_mtime(
    path: &Path,
    seconds_since_unix_epoch: i64,
    nanoseconds: u32,
) -> io::Result<()> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use winapi::um::winnt::{
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_WRITE_ATTRIBUTES,
    };

    let time = unix_time_to_system_time(seconds_since_unix_epoch, nanoseconds)?;
    let file = std::fs::OpenOptions::new()
        .access_mode(FILE_WRITE_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)?;
    file.set_modified(time)
}

/// Convert a Unix-epoch second/nanosecond pair to [`std::time::SystemTime`],
/// reporting out-of-range input rather than silently wrapping it.
fn unix_time_to_system_time(
    seconds_since_unix_epoch: i64,
    nanoseconds: u32,
) -> io::Result<std::time::SystemTime> {
    use std::time::{Duration, SystemTime};

    let out_of_range = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "modification time is out of range for this host's clock",
        )
    };

    if seconds_since_unix_epoch >= 0 {
        SystemTime::UNIX_EPOCH
            .checked_add(Duration::new(seconds_since_unix_epoch as u64, nanoseconds))
            .ok_or_else(out_of_range)
    } else {
        let magnitude = seconds_since_unix_epoch
            .checked_neg()
            .ok_or_else(out_of_range)? as u64;
        SystemTime::UNIX_EPOCH
            .checked_sub(Duration::new(magnitude, 0))
            .and_then(|time| time.checked_add(Duration::new(0, nanoseconds)))
            .ok_or_else(out_of_range)
    }
}

/// Encode `path` as the bytes this host uses to spell it.
///
/// Faithful, not canonical: this is the encoding a path is carried in so the
/// other end can reconstruct exactly the path that was named. It is
/// deliberately not `ipc::endpoint_scope_bytes`, which folds away differences
/// a host considers meaningless in order to hash two spellings to one identity.
/// Round-tripping through the pair here must return the original path;
/// round-tripping through that one need not.
pub fn encode_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Reconstruct a path from [`encode_path_bytes`] output produced on this host.
///
/// Windows paths are UTF-16, so an odd byte count cannot have come from this
/// encoder and is rejected rather than silently truncated.
pub fn decode_path_bytes(bytes: &[u8]) -> io::Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt as _;

    if !bytes.len().is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Windows path bytes must be little-endian UTF-16",
        ));
    }
    let wide = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    Ok(PathBuf::from(std::ffi::OsString::from_wide(&wide)))
}

/// Directory for `product`'s shared application data.
///
/// Distinct from [`user_state_dir`]: state is this machine's private
/// bookkeeping, while this is the data a user expects to follow their account.
/// That is exactly the roaming/local split, so this uses the roaming root
/// while the state and runtime roles use the local one.
pub fn user_data_dir(product: &str) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join(product)
}

/// The directory this host keeps a product's *configuration* in.
///
/// Distinct from [`user_data_dir`] on this host: Windows separates roaming
/// configuration from local application data, and a setting a user expects to
/// follow them between machines belongs in the former.
pub fn user_config_dir(product: &str) -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join(product)
}

/// Move `tmp` onto `target`, replacing it, without a window where neither is
/// readable.
///
/// `ReplaceFileW` is the call that gives that guarantee here; a bare rename
/// onto an existing file fails on Windows. With no file to replace there is
/// nothing for it to do, so a rename is both correct and cheaper.
pub fn replace_file(tmp: &Path, target: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::{ReplaceFileW, REPLACEFILE_WRITE_THROUGH};

    if !target.exists() {
        return std::fs::rename(tmp, target);
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    let target_w = wide(target);
    let tmp_w = wide(tmp);
    let ok = unsafe {
        ReplaceFileW(
            target_w.as_ptr(),
            tmp_w.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Make a directory entry created by [`replace_file`] durable.
///
/// Nothing to do here: `REPLACEFILE_WRITE_THROUGH` already committed the
/// change, and Windows does not expose a directory handle to flush.
pub fn sync_directory(_directory: &Path) -> io::Result<()> {
    Ok(())
}

/// Create a new file that only its owner can read, failing if it exists.
///
/// `create_new` is part of the contract, not a convenience: a private file
/// opened over one that already exists inherits whatever that one allowed.
///
/// Windows carries no mode bits here; the file inherits its directory's ACL.
/// That is the same protection by a different route as long as the caller
/// creates it somewhere already scoped to one user -- a per-user temp
/// directory, or one of the [`user_runtime_dir`] roles -- which is why the
/// directory choice is the caller's and matters.
pub fn create_private_file(path: &Path) -> io::Result<File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}
