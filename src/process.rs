//! Canonical synchronous process, stdio, observer, and hash primitives.
//!
//! This migration namespace preserves the selected substrate's type identity
//! for first-party clients moving off direct `running-process` imports. It is
//! intentionally a reviewed list rather than a crate-wide re-export.

pub use running_process::observer::*;
pub use running_process::{
    run_std_command_bounded, spawn, spawn_daemon, spawn_daemon_with_environment,
    spawn_daemon_with_explicit_environment, spawn_daemon_with_stdio_and_env_policy,
    spawn_with_environment, spawn_with_explicit_environment, DaemonChild, DaemonStdio,
    DaemonStdioSource, EnvironmentPolicy, ProcessError, RunOutput, SpawnStdio, SpawnedChild,
    StdioSource, SyncEnvironment,
};

/// Canonical client-enabled BLAKE3 helper.
#[cfg(feature = "broker")]
pub use running_process::blake3_file;
