//! Windows atomic replace: `MoveFileExW` with verbatim paths and the
//! AV-scanner retry ladder.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use crate::platform::fs::native_call_path as verbatim_path;

/// Uniquifies intermediate backup names in `install_directory`.
static DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Transient sharing errors an antivirus scanner causes around rename
/// (ERROR_ACCESS_DENIED / ERROR_SHARING_VIOLATION), plus any
/// `PermissionDenied`-shaped error.
pub(crate) fn is_av_scan_transient(err: &std::io::Error) -> bool {
    if err.kind() == std::io::ErrorKind::PermissionDenied {
        return true;
    }
    matches!(err.raw_os_error(), Some(5) | Some(32))
}

pub fn is_transient_share_error(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(5) | Some(32))
}

pub fn is_lock_contention(error: &std::io::Error) -> bool {
    matches!(error.raw_os_error(), Some(33))
}

/// Retries `op` across the fixed AV-scan delay ladder, five attempts.
pub(crate) fn av_scan_retry<T, F: FnMut() -> std::io::Result<T>>(op: F) -> std::io::Result<T> {
    retry_with_delay(op, |ms| {
        std::thread::sleep(std::time::Duration::from_millis(ms))
    })
}

fn retry_with_delay<T>(
    mut op: impl FnMut() -> std::io::Result<T>,
    mut delay_ms: impl FnMut(u64),
) -> std::io::Result<T> {
    const DELAYS_MS: [u64; 4] = [50, 100, 250, 500];
    let mut last = op();
    for delay in DELAYS_MS {
        match &last {
            Ok(_) => return last,
            Err(err) if !is_av_scan_transient(err) => return last,
            Err(_) => {
                delay_ms(delay);
                last = op();
            }
        }
    }
    last
}

/// Atomically replaces `destination` with `source` (verbatim long paths,
/// `MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH`, retried past AV
/// scanners). On success `source` no longer exists.
pub fn atomic_replace(source: &Path, destination: &Path) -> std::io::Result<()> {
    let src = verbatim_path(source)?;
    let dst = verbatim_path(destination)?;
    let src_wide = wide_path(&src)?;
    let dst_wide = wide_path(&dst)?;
    av_scan_retry(|| unsafe {
        let ok = MoveFileExW(
            src_wide.as_ptr(),
            dst_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        );
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

/// Renames `source` to `destination` where the destination must NOT exist
/// (generation rename), retried past AV scanners.
pub fn rename_generation(source: &Path, destination: &Path) -> std::io::Result<()> {
    let src = verbatim_path(source)?;
    let dst = verbatim_path(destination)?;
    let src_wide = wide_path(&src)?;
    let dst_wide = wide_path(&dst)?;
    av_scan_retry(|| unsafe {
        let ok = MoveFileExW(src_wide.as_ptr(), dst_wide.as_ptr(), 0);
        if ok == 0 {
            Err(std::io::Error::last_os_error())
        } else {
            Ok(())
        }
    })
}

/// Replaces `destination` with `source`, falling back to delete-then-rename
/// after any replacement error when the destination exists. This is not
/// atomic: deletion may succeed and the subsequent rename may fail.
pub fn replace_with_delete_fallback(source: &Path, destination: &Path) -> std::io::Result<()> {
    match atomic_replace(source, destination) {
        Ok(()) => Ok(()),
        Err(_) if destination.exists() => {
            std::fs::remove_file(destination)?;
            rename_generation(source, destination)
        }
        Err(err) => Err(err),
    }
}

/// Installs the staged directory tree `staged` over `requested`, which may
/// already exist. Windows has no atomic directory exchange, so the existing
/// tree is first renamed aside to a backup name; failure attempts restoration.
/// Rollback is best-effort and may itself fail, leaving the backup in place.
pub fn install_directory(staged: &Path, requested: &Path) -> std::io::Result<()> {
    let parent = requested.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    if !requested.exists() {
        return std::fs::rename(staged, requested);
    }
    let backup = parent.join(format!(
        ".zccache-directory-backup-{}-{}",
        std::process::id(),
        DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    install_with_backup(
        staged,
        requested,
        &backup,
        |from, to| std::fs::rename(from, to),
        remove_directory_if_present,
    )
}

fn install_with_backup(
    staged: &Path,
    requested: &Path,
    backup: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    mut remove: impl FnMut(&Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    rename(requested, backup)?;
    if let Err(error) = rename(staged, requested) {
        let _ = rename(backup, requested);
        return Err(error);
    }
    remove(backup)
}

fn remove_directory_if_present(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn wide_path(path: &Path) -> std::io::Result<Vec<u16>> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path contains NUL",
        ));
    }
    wide.push(0);
    Ok(wide)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_install_attempts_rollback_without_masking_the_install_error() {
        for rollback_fails in [false, true] {
            let mut calls = Vec::new();
            let error = install_with_backup(
                Path::new("staged"),
                Path::new("requested"),
                Path::new("backup"),
                |from, to| {
                    calls.push((from.to_path_buf(), to.to_path_buf()));
                    match calls.len() {
                        1 => Ok(()),
                        2 => Err(std::io::Error::from_raw_os_error(32)),
                        3 if rollback_fails => Err(std::io::Error::from_raw_os_error(5)),
                        3 => Ok(()),
                        _ => panic!("unexpected rename"),
                    }
                },
                |_| panic!("failed installation must retain the backup, not clean it"),
            )
            .unwrap_err();
            assert_eq!(error.raw_os_error(), Some(32));
            assert_eq!(
                calls,
                [
                    ("requested".into(), "backup".into()),
                    ("staged".into(), "requested".into()),
                    ("backup".into(), "requested".into()),
                ]
            );
        }
    }

    #[test]
    fn failed_initial_backup_stops_before_installation() {
        let mut calls = 0;
        let error = install_with_backup(
            Path::new("staged"),
            Path::new("requested"),
            Path::new("backup"),
            |_, _| {
                calls += 1;
                Err(std::io::Error::from_raw_os_error(5))
            },
            |_| panic!("no backup was created"),
        )
        .unwrap_err();
        assert_eq!(calls, 1);
        assert_eq!(error.raw_os_error(), Some(5));
    }

    #[test]
    fn cleanup_failure_is_reported_after_successful_install_without_rollback() {
        let mut renames = 0;
        let error = install_with_backup(
            Path::new("staged"),
            Path::new("requested"),
            Path::new("backup"),
            |_, _| {
                renames += 1;
                Ok(())
            },
            |path| {
                assert_eq!(path, Path::new("backup"));
                Err(std::io::Error::from_raw_os_error(5))
            },
        )
        .unwrap_err();
        assert_eq!(renames, 2);
        assert_eq!(error.raw_os_error(), Some(5));
    }

    #[test]
    fn retry_exhaustion_preserves_exact_attempts_delays_and_last_error() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let error = retry_with_delay::<()>(
            || {
                attempts += 1;
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    attempts.to_string(),
                ))
            },
            |delay| delays.push(delay),
        )
        .unwrap_err();
        assert_eq!(attempts, 5);
        assert_eq!(delays, [50, 100, 250, 500]);
        assert_eq!(error.to_string(), "5");
    }

    #[test]
    fn retry_stops_on_success_or_nontransient_error() {
        let mut attempts = 0;
        let mut delays = Vec::new();
        let result = retry_with_delay(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(std::io::Error::from_raw_os_error(32))
                } else {
                    Ok(42)
                }
            },
            |delay| delays.push(delay),
        )
        .unwrap();
        assert_eq!(result, 42);
        assert_eq!(attempts, 3);
        assert_eq!(delays, [50, 100]);
        let error = retry_with_delay::<()>(
            || Err(std::io::Error::from_raw_os_error(2)),
            |_| panic!("nontransient failure must not wait"),
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(2));
        assert_eq!(
            retry_with_delay(|| Ok(7), |_| panic!("success must not wait")).unwrap(),
            7
        );
    }

    #[test]
    fn nul_path_is_rejected_and_unpaired_surrogates_are_preserved() {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt;
        assert_eq!(
            wide_path(Path::new("prefix\0suffix")).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        let path = OsString::from_wide(&[65, 0xd800, 66]);
        assert_eq!(wide_path(Path::new(&path)).unwrap(), [65, 0xd800, 66, 0]);
    }
}
