//! Canonical asynchronous child/session primitives.
//!
//! These aliases expose the selected substrate's semantic process API without
//! exposing Tokio child or I/O types. They coexist with the higher-level
//! facade-owned `SpawnSpec`/`ProcessSession` policy for consumers that need its
//! owner binding and bounded lifecycle semantics.
//!
//! The canonical `output_bounded`/`capture_bounded` methods limit retained
//! bytes and drain excess output until completion; they do not kill on
//! overflow. Use [`crate::run_bounded_command`] when the requested policy is
//! to terminate on timeout or aggregate output overflow. A re-export preserves
//! this distinction rather than silently changing the substrate's behavior.
//!
//! With `async-process-client`, the explicitly transitional `spawn_tokio`
//! bridge also becomes available. Its native Tokio return type is an exception
//! to the semantic surface above, not evidence of completed client migration.

pub use running_process::{
    AsyncCapturedOutput, AsyncProcess, AsyncProcessBuilder, AsyncProcessSession,
    AsyncProcessSessionChunk, AsyncProcessSessionControl, AsyncProcessSessionEvent,
    AsyncProcessSessionOptions, AsyncProcessSessionOutput, AsyncStdio,
    ProcessError as AsyncProcessError, ProcessPriority, ProcessTreeKill, SpawnAdmission,
    StreamKind,
};

/// Tokio-compatible construction options retained for first-party migration.
/// Unlike this module's semantic builder/session aliases, this compatibility
/// function intentionally retains its native Tokio child return type.
#[cfg(feature = "async-process-client")]
pub use running_process::{spawn_tokio, TokioSpawnOptions};

#[cfg(test)]
mod native_exit_contract_tests {
    use super::*;

    async fn shell_status(script: &str) -> std::process::ExitStatus {
        #[cfg(unix)]
        let builder = AsyncProcessBuilder::new("/bin/sh").args(["-c", script]);
        #[cfg(windows)]
        let builder = AsyncProcessBuilder::new(
            std::path::PathBuf::from(std::env::var_os("SystemRoot").expect("Windows system root"))
                .join("System32")
                .join("cmd.exe"),
        )
        .args(["/D", "/C", script]);
        tokio::time::timeout(std::time::Duration::from_secs(5), async move {
            let mut process = builder
                .stdin(AsyncStdio::Null)
                .stdout(AsyncStdio::Null)
                .stderr(AsyncStdio::Null)
                .kill_on_drop(true)
                .build();
            process.start().await.expect("start exit-status fixture");
            process.wait().await.expect("reap exit-status fixture")
        })
        .await
        .expect("exit-status fixture must finish")
    }

    #[tokio::test]
    async fn normal_exit_is_identical_through_all_namespaces() {
        let status = shell_status("exit 37").await;
        assert_eq!(running_process::native_exit_code(status), 37);
        assert_eq!(crate::exit_code(status), 37);
        assert_eq!(crate::platform::process::exit_code(status), 37);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn signal_exit_is_identical_through_all_namespaces() {
        let status = shell_status("kill -TERM $$").await;
        assert_eq!(running_process::native_exit_code(status), -libc::SIGTERM);
        assert_eq!(crate::exit_code(status), -libc::SIGTERM);
        assert_eq!(crate::platform::process::exit_code(status), -libc::SIGTERM);
    }
}
