//! What this host says about running out of things.
//!
//! A caller that fails to open a socket or write a file needs to know *which*
//! wall it hit -- descriptors, or space -- because the two call for different
//! responses: shed load and retry, or stop and free something. The number the
//! host returns to say so is different on each of them, and in the Windows
//! case there are three numbers rather than one.
//!
//! So the classification lives here and the response stays with the caller.
//! Nothing in these signatures names an errno or a Win32 code.

pub use crate::{
    resources_available_space as available_space,
    resources_fd_exhaustion_error as fd_exhaustion_error,
    resources_inode_capacity as inode_capacity,
    resources_signals_fd_exhaustion as signals_fd_exhaustion,
    resources_signals_storage_exhaustion as signals_storage_exhaustion,
    resources_storage_exhaustion_error as storage_exhaustion_error,
    resources_total_space as total_space,
};

/// How many inodes a filesystem has, and how many are still available.
///
/// Only filesystems with a fixed inode table have this to report, so callers
/// receive it as an `Option` and must decide what an absent answer means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InodeCapacity {
    /// Total inodes on the filesystem.
    pub total: u64,
    /// Inodes available to unprivileged users.
    pub free: u64,
}

#[cfg(test)]
mod tests {
    use std::io;

    use super::*;

    /// The classifiers must agree with the errors this host reports, or a
    /// caller keying on them retries the wrong wall -- or neither.
    #[test]
    fn this_hosts_exhaustion_errors_are_recognised() {
        assert!(
            signals_fd_exhaustion(&fd_exhaustion_error()),
            "a descriptor-exhaustion error must be recognised as one"
        );
        assert!(
            signals_storage_exhaustion(&storage_exhaustion_error()),
            "a storage-exhaustion error must be recognised as one"
        );
    }

    /// The two conditions are not the same wall, and must not be confused for
    /// one another.
    #[test]
    fn the_two_exhaustion_conditions_stay_distinct() {
        assert!(!signals_storage_exhaustion(&fd_exhaustion_error()));
        assert!(!signals_fd_exhaustion(&storage_exhaustion_error()));
    }

    /// An error carrying no OS code cannot be either condition; guessing from
    /// its text would make an unrelated failure look like exhaustion.
    #[test]
    fn an_error_without_an_os_code_signals_nothing() {
        let synthetic = io::Error::other("no operating system said this");
        assert!(!signals_fd_exhaustion(&synthetic));
        assert!(!signals_storage_exhaustion(&synthetic));
    }

    /// An ordinary failure is not exhaustion.
    #[test]
    fn an_unrelated_os_error_signals_nothing() {
        let denied = io::Error::new(io::ErrorKind::PermissionDenied, "denied");
        assert!(!signals_fd_exhaustion(&denied));
        assert!(!signals_storage_exhaustion(&denied));
    }

    /// Whatever this host reports for a directory that exists, it is either a
    /// coherent capacity or an explicit "not applicable" -- never an error.
    #[test]
    fn probing_an_existing_directory_answers_coherently() {
        let probed = inode_capacity(&std::env::temp_dir()).expect("probing temp dir must succeed");
        if let Some(capacity) = probed {
            assert!(capacity.total > 0, "a reported table is never empty");
            assert!(
                capacity.free <= capacity.total,
                "free inodes cannot exceed the table"
            );
        }
    }

    /// Space available to this caller never exceeds the volume's total
    /// capacity, and a real filesystem never reports zero total bytes.
    #[test]
    fn space_probes_are_coherent_for_an_existing_directory() {
        let directory = std::env::temp_dir();
        let total = total_space(&directory).expect("total space must succeed");
        let available = available_space(&directory).expect("available space must succeed");
        assert!(total > 0, "a mounted filesystem is never zero bytes");
        assert!(
            available <= total,
            "available space cannot exceed total capacity"
        );
    }
}
#[cfg(target_os = "linux")]
#[path = "resources/physical_linux.rs"]
mod physical;
#[cfg(target_os = "macos")]
#[path = "resources/physical_macos.rs"]
mod physical;
#[cfg(windows)]
#[path = "resources/physical_win.rs"]
mod physical;
pub use physical::physical_cores;
#[cfg(target_os = "linux")]
#[path = "resources/rss_linux.rs"]
mod rss;
#[cfg(target_os = "macos")]
#[path = "resources/rss_macos.rs"]
mod rss;
#[cfg(windows)]
#[path = "resources/rss_win.rs"]
mod rss;
pub use rss::process_rss_bytes;

#[path = "resources/cgroup.rs"]
mod cgroup;
pub use cgroup::cgroup_v2_dir;

#[path = "resources/snapshot.rs"]
mod snapshot;
pub use snapshot::HostResourceSnapshot;

#[cfg(windows)]
#[path = "resources/windows_telemetry.rs"]
mod windows_telemetry;
#[cfg(windows)]
pub use windows_telemetry::{commit_charge_mb, process_table};

/// Windows ToolHelp process snapshot; unsupported on other hosts.
#[cfg(not(windows))]
pub fn process_table() -> Option<Vec<(u32, String)>> {
    None
}

/// Windows commit charge in MiB; unsupported on other hosts.
#[cfg(not(windows))]
pub fn commit_charge_mb() -> Option<(u64, u64)> {
    None
}
