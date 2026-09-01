#![doc = include_str!("../README.md")]

//! Facade-owned asynchronous systems operations.
//!
//! This crate is the public systems boundary shared by its clients. Backend
//! crates remain private implementation details: higher layers receive
//! `kernal_api` types and never name the underlying async runtime, allocator,
//! profiler, symbol parser, or native platform APIs directly.

use std::cfg_select;
use std::ffi::{OsStr, OsString};
use std::io;
use std::path::PathBuf;
use std::process::ExitStatus;

mod process_adapter;

/// Canonical async runtime, task, I/O, network, and synchronization facade.
pub mod async_engine;

/// Admission policy for opt-in Rust WebAssembly sketches.
///
/// This module privately admits and starts the bounded threaded-root profile.
/// Thread scheduling, generated async ABI, resources, and worker-process
/// containment land in later sketch-host slices.
#[cfg(feature = "wasm-sketch-host")]
pub mod wasm;

/// Cooperative all-thread stack capture and deferred unwinding.
#[cfg(feature = "snapshot")]
pub mod snapshot;

/// Bounded CPU/off-CPU profiling with pprof, Firefox, and collapsed exports.
#[cfg(feature = "profile")]
pub mod profile;

/// PDB/DWARF/Mach-O symbol discovery and resolution.
///
/// Parse untrusted symbol files in a disposable worker process; the API is
/// kept here so every client uses the same ASLR-independent capture schema.
#[cfg(feature = "symbolize")]
pub mod symbolize;

/// Dormant-by-default mimalloc heap profiling and dump support.
#[cfg(feature = "allocator")]
pub mod allocator;

/// Cooperative crash capture using the crate's single native handler.
#[cfg(feature = "crash")]
pub mod crash;

/// Runtime diagnostics implementation; use [`async_engine`] as the public path.
#[cfg(feature = "tokio-console")]
#[doc(hidden)]
mod runtime;

/// Neutral capability indexes for the eventual workspace-wide host boundary.
///
/// The indexes intentionally expose no operations yet: phase 2 establishes
/// ownership names before later phases move a capability behind them.
pub mod platform;

// This is deliberately the crate's only host selector.  Facade modules are
// neutral; native details live behind the selected private root.
cfg_select! {
    target_os = "windows" => {
        mod platform_win;
        pub(crate) use platform_win as platform_imp;
    }
    target_os = "linux" => {
        mod platform_linux;
        pub(crate) use platform_linux as platform_imp;
    }
    target_os = "macos" => {
        mod platform_macos;
        pub(crate) use platform_macos as platform_imp;
    }
}

// Re-export the selected implementation once from this allowed host-selector
// root. Neutral capability facades re-export only crate-root names and never
// name the private `platform_imp` alias themselves.
pub use platform_imp::{
    active_graphics_probe, assign_child_to_windows_job, cancel_capture_reader,
    canonical_environment_pairs, capture_reader_done, compat_shell_command, configure_exact_trace,
    configure_process_command, configure_sync_contained_command, configure_sync_daemon_command,
    configure_trampoline_command, current_executable_build_id, exact_trace_capability, exit_code,
    kill_tree, monitor_console_windows, parent_has_console, prepare_capture_reader,
    process_snapshot, process_snapshot_for_pid, set_process_name, set_window_icon_impl,
    shell_command, soft_terminate_process_group, spawn_sync, spawn_sync_daemon,
    start_descendant_monitor, start_exact_trace, sync_child_native_handle, trampoline_exit_code,
    unix_mark_extra_fds_close_on_exec, unix_set_priority, unix_signal_process,
    unix_signal_process_group, unix_signal_raw, window_icon_support_impl, CaptureCancellation,
    TracedChild, WindowsJobHandle,
};

pub use platform_imp::{autostart_register, autostart_render_registration, autostart_unregister};

pub use platform_imp::{process_install_owner_death_cleanup, process_owner_death_cleanup_target};

pub use platform_imp::process_install_shutdown_request_handler;

pub use platform_imp::fs_write_all_to_descriptor;

pub use platform_imp::{process_can_replace_current_image, process_replace_current_image};

pub use platform_imp::{
    process_executable_path, process_force_kill, process_same_executable_path,
    process_signal_terminate, ProcessLiveness,
};

pub use platform_imp::{
    resources_fd_exhaustion_error, resources_inode_capacity, resources_signals_fd_exhaustion,
    resources_signals_storage_exhaustion, resources_storage_exhaustion_error,
};

pub use platform_imp::{
    executable_file_name, executable_sibling_of_current_image, EXECUTABLE_EXTENSION,
};

#[cfg(feature = "fs")]
pub use platform_imp::{
    fs_create_private_file, fs_decode_path_bytes, fs_encode_path_bytes, fs_file_identity,
    fs_is_lock_conflict, fs_open_lock_file, fs_path_identity, fs_replace_file, fs_sync_directory,
    fs_try_lock_exclusive, fs_unlock, fs_user_config_dir, fs_user_data_dir, fs_user_run_data_root,
    fs_user_runtime_dir, fs_user_state_dir, FsFileIdentity,
};

pub use platform_imp::{
    host_boot_id, host_current_process_privilege, host_environment_keys_are_case_insensitive,
    host_filesystem_device_id, host_hostname, host_login_environment, host_machine_id,
    host_namespace_id, host_user_machine_identity, HostPrivilegedIdentity,
};

pub use platform_imp::host_login_environment_block;

pub use platform_imp::terminal_input;

#[cfg(feature = "ipc")]
pub use platform_imp::{
    ipc_broker_endpoint_name as IpcBrokerEndpointName, ipc_broker_v1_endpoint_path,
    ipc_broker_v2_runtime_dir, ipc_current_user_id, ipc_endpoint_is_filesystem_backed,
    ipc_endpoint_name_limit, ipc_endpoint_scope_bytes, ipc_ensure_owner_private_directory,
    ipc_nonblocking_zero_read_is_pending, ipc_owner_private_directory, ipc_select_endpoint_address,
    IpcEndpoint, IpcInheritedListener, IpcListener, IpcListenerNonblockingMode, IpcPeerIdentity,
    IpcPeerIdentitySource, IpcStream,
};

/// Failure details for the deprecated 4.x raw descriptor/handle handoff API.
///
/// This type exists only at the crate-root compatibility boundary. New product
/// mechanics use opaque [`platform::ipc::Stream`] operations instead.
#[cfg(feature = "ipc")]
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LegacyHandoffError {
    kind: platform::ipc::HandoffTransferErrorKind,
    raw_os_error: Option<i32>,
    transferred_bytes: Option<usize>,
    expected_bytes: Option<usize>,
    detail: Option<String>,
}

#[cfg(feature = "ipc")]
impl LegacyHandoffError {
    pub(crate) fn new(
        kind: platform::ipc::HandoffTransferErrorKind,
        raw_os_error: Option<i32>,
    ) -> Self {
        Self {
            kind,
            raw_os_error,
            transferred_bytes: None,
            expected_bytes: None,
            detail: None,
        }
    }

    pub(crate) fn with_detail(
        kind: platform::ipc::HandoffTransferErrorKind,
        raw_os_error: Option<i32>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            raw_os_error,
            transferred_bytes: None,
            expected_bytes: None,
            detail: Some(detail.into()),
        }
    }

    #[doc(hidden)]
    pub fn partial(transferred_bytes: usize, expected_bytes: usize) -> Self {
        Self {
            kind: platform::ipc::HandoffTransferErrorKind::Failed,
            raw_os_error: None,
            transferred_bytes: Some(transferred_bytes),
            expected_bytes: Some(expected_bytes),
            detail: Some(format!(
                "SCM_RIGHTS connection transfer was partial ({transferred_bytes}/{expected_bytes} bytes)"
            )),
        }
    }

    /// Return the policy-neutral failure category.
    pub fn kind(&self) -> platform::ipc::HandoffTransferErrorKind {
        self.kind
    }

    /// Return the native error code retained for legacy public diagnostics.
    pub fn raw_os_error(&self) -> Option<i32> {
        self.raw_os_error
    }

    /// Return a partial payload count when the descriptor may have transferred.
    pub fn partial_counts(&self) -> Option<(usize, usize)> {
        self.transferred_bytes.zip(self.expected_bytes)
    }

    pub(crate) fn detail(&self) -> Option<&str> {
        self.detail.as_deref()
    }
}

/// Whether the deprecated 4.x SCM_RIGHTS compatibility transport is available.
#[cfg(feature = "ipc")]
#[doc(hidden)]
pub const LEGACY_SCM_RIGHTS_TRANSPORT_SUPPORTED: bool =
    platform_imp::LEGACY_SCM_RIGHTS_TRANSPORT_SUPPORTED;

/// Whether the deprecated 4.x DuplicateHandle compatibility transport is available.
#[cfg(feature = "ipc")]
#[doc(hidden)]
pub const LEGACY_DUPLICATE_HANDLE_TRANSPORT_SUPPORTED: bool =
    platform_imp::LEGACY_DUPLICATE_HANDLE_TRANSPORT_SUPPORTED;

/// Root-only adapter for the deprecated raw-descriptor handoff API.
#[cfg(feature = "ipc")]
#[doc(hidden)]
pub fn legacy_send_fd_to(
    socket: &std::path::Path,
    sent_fd: i32,
    payload: &[u8],
) -> Result<(), LegacyHandoffError> {
    platform_imp::legacy_send_fd_to(socket, sent_fd, payload)
}

/// Root-only adapter for the deprecated connected raw-descriptor handoff API.
#[cfg(feature = "ipc")]
#[doc(hidden)]
pub fn legacy_send_fd_over(
    socket_fd: i32,
    sent_fd: i32,
    payload: &[u8],
) -> Result<(), LegacyHandoffError> {
    platform_imp::legacy_send_fd_over(socket_fd, sent_fd, payload)
}

/// Root-only adapter for the deprecated raw-handle duplication API.
#[cfg(feature = "ipc")]
#[doc(hidden)]
pub fn legacy_duplicate_handle(
    source_handle: usize,
    backend_pid: u32,
) -> Result<usize, LegacyHandoffError> {
    platform_imp::legacy_duplicate_handle(source_handle, backend_pid)
}

#[cfg(feature = "ipc-async")]
pub use platform_imp::{
    IpcAsyncListener, IpcAsyncStream, IpcIntoAsyncListener, IpcIntoAsyncStream,
};

#[cfg(feature = "pty")]
pub use platform_imp::terminal::{
    before_pty_spawn, current_backend_kind, find_child_processes, find_orphan_conhosts,
    input_payload, is_ignorable_process_control_error, prepare_unmanaged_pty_child,
    query_responses, resize_pty, shell_argv, signal_pty_tree, terminate_pty_child,
    wait_before_pty_close_supported, Backend, ChildProcessInfo, ConPtyBackendKind,
    OrphanConhostInfo, PtyProcessGuard, PtySpawnContext, TerminalInputSession,
};

#[cfg(feature = "session-relay")]
pub use platform_imp::relay_local_socket_session;

/// Stdio policy for one child stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamMode {
    /// Leave the stream connected to the parent process.
    Inherit,
    /// Create an asynchronous pipe owned by the child handle.
    Piped,
    /// Connect the stream to the platform null device.
    Null,
}

/// Typed spawn description accepted by the blessed process boundary.
#[derive(Debug, Clone)]
pub struct SpawnSpec {
    program: OsString,
    args: Vec<OsString>,
    current_dir: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
    clear_env: bool,
    stdin: StreamMode,
    stdout: StreamMode,
    stderr: StreamMode,
    create_process_group: bool,
    kill_when_owner_dies: bool,
}

impl SpawnSpec {
    /// Create a direct (non-shell) command description.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
            current_dir: None,
            env: Vec::new(),
            clear_env: false,
            stdin: StreamMode::Inherit,
            stdout: StreamMode::Inherit,
            stderr: StreamMode::Inherit,
            create_process_group: false,
            kill_when_owner_dies: false,
        }
    }

    /// Append one argument without requiring UTF-8.
    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Set the child working directory.
    pub fn current_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(path.into());
        self
    }

    /// Add an environment override.
    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Start with an empty inherited environment before applying overrides.
    pub fn clear_env(mut self, clear: bool) -> Self {
        self.clear_env = clear;
        self
    }

    /// Configure child stdin.
    pub fn stdin(mut self, mode: StreamMode) -> Self {
        self.stdin = mode;
        self
    }

    /// Configure child stdout.
    pub fn stdout(mut self, mode: StreamMode) -> Self {
        self.stdout = mode;
        self
    }

    /// Configure child stderr.
    pub fn stderr(mut self, mode: StreamMode) -> Self {
        self.stderr = mode;
        self
    }

    /// Put the child in its own process group.
    ///
    /// This is what makes a group-wide soft signal addressable at all:
    /// [`PlatformChild::terminate_group_soft`] is a no-op without
    /// it, because on POSIX the negative-PID signal would otherwise reach the
    /// caller's own group, and on Windows `GenerateConsoleCtrlEvent` only
    /// routes to children spawned with `CREATE_NEW_PROCESS_GROUP`. It also
    /// detaches the child from the parent's console Ctrl+C, so it is opt-in.
    pub fn create_process_group(mut self, create: bool) -> Self {
        self.create_process_group = create;
        self
    }

    /// Kill this child when the spawning process exits unexpectedly.
    ///
    /// Linux uses `PR_SET_PDEATHSIG(SIGTERM)`. Windows assigns the child to a
    /// process-wide kill-on-close Job Object. macOS forks a kqueue supervisor
    /// before exec and reports spawn success only after its owner and child
    /// watches are registered.
    pub fn kill_when_owner_dies(mut self, kill: bool) -> Self {
        self.kill_when_owner_dies = kill;
        self
    }

    /// Spawn using the canonical asynchronous platform operation.
    ///
    /// The private native substrate owns process creation, containment, and
    /// owner-death installation. This facade retains the semantic command
    /// description and never exposes the substrate's child or runtime types.
    pub async fn spawn(self) -> io::Result<PlatformChild> {
        process_adapter::spawn(self).await
    }
}

/// Owned child handle returned by [`SpawnSpec::spawn`].
pub struct PlatformChild {
    inner: process_adapter::ProcessAdapter,
}

impl PlatformChild {
    pub(crate) fn new(inner: process_adapter::ProcessAdapter) -> Self {
        Self { inner }
    }

    /// Return the operating-system process identifier while this handle has
    /// not observed a terminal lifecycle result.
    ///
    /// A successful [`Self::wait`] or [`Self::kill`] clears this value so a
    /// completed child PID is not presented as a reusable live identity.
    pub fn id(&self) -> Option<u32> {
        self.inner.pid()
    }

    /// Wait for completion without capturing output.
    pub async fn wait(&mut self) -> io::Result<ExitStatus> {
        self.inner.wait().await
    }

    /// Terminate the child and wait for its exit.
    pub async fn kill(&mut self) -> io::Result<()> {
        self.inner.kill().await
    }

    /// Request graceful termination for the child-owned process group.
    ///
    /// Returns `Ok(false)` unless [`SpawnSpec::create_process_group`] was
    /// enabled, or when the child has already exited.
    pub async fn terminate_group_soft(&self) -> io::Result<bool> {
        self.inner.terminate_group_soft().await
    }

    /// Write bytes to piped stdin.
    pub async fn write_stdin(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.inner.write_stdin(bytes).await
    }

    /// Close the piped stdin handle, delivering EOF to the child.
    ///
    /// The substrate makes a successful close idempotent. An inherited or
    /// null stdin reports that no writable child pipe exists.
    pub async fn close_stdin(&mut self) -> io::Result<()> {
        self.inner.close_stdin().await
    }

    /// Capture piped stdout and stderr after the child exits, retaining at
    /// most one aggregate byte limit in the returned [`ProcessOutput`].
    ///
    /// The limit covers both returned streams together: on success,
    /// `stdout.len() + stderr.len()` is at most `limit`. Excess bytes are not
    /// returned, and the completed capture returns
    /// [`ProcessCaptureError::OutputLimitExceeded`] without partial output.
    /// The running-process 4.10.9 actor separately keeps a fixed 16 MiB
    /// diagnostic output log while draining. That log evicts older records,
    /// is not configurable through this substrate API, and is independent of
    /// this method's returned-output limit.
    /// This operation waits for both streams to reach EOF and the child to
    /// exit naturally; it does not terminate a child merely because retained
    /// output reached the limit.
    pub async fn wait_with_output_bounded(
        mut self,
        limit: usize,
    ) -> Result<ProcessOutput, ProcessCaptureError> {
        self.inner.capture_bounded(limit).await
    }
}

/// Status-preserving result of bounded child-output capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// The operating system's unmodified child status.
    pub status: ExitStatus,
    /// Bytes captured from stdout.
    pub stdout: Vec<u8>,
    /// Bytes captured from stderr.
    pub stderr: Vec<u8>,
}

/// Failure returned by [`PlatformChild::wait_with_output_bounded`].
#[derive(Debug)]
pub enum ProcessCaptureError {
    /// The returned stdout/stderr pair would have exceeded its shared limit.
    OutputLimitExceeded {
        /// Aggregate stdout/stderr capture limit.
        limit: usize,
    },
    /// Waiting for or capturing from the started child failed.
    Io(io::Error),
}

impl std::fmt::Display for ProcessCaptureError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutputLimitExceeded { limit } => {
                write!(
                    formatter,
                    "captured process output exceeded the {limit}-byte limit"
                )
            }
            Self::Io(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ProcessCaptureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::OutputLimitExceeded { .. } => None,
            Self::Io(error) => Some(error),
        }
    }
}

/// Build a shell command using the host platform's supported shell.
pub fn shell_spec(command: impl AsRef<OsStr>) -> SpawnSpec {
    platform_imp::shell_spec(command.as_ref())
}

#[cfg(test)]
mod tests {
    use super::{
        run_bounded_command, run_bounded_command_async, shell_spec, BoundedProcessAsyncError,
        BoundedProcessError, SpawnSpec, StreamMode,
    };
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn fixture_command() -> SpawnSpec {
        #[cfg(windows)]
        {
            shell_spec("echo async-platform-internal")
        }
        #[cfg(not(windows))]
        {
            shell_spec("printf async-platform-internal")
        }
    }

    #[tokio::test]
    async fn blessed_spawn_captures_output_without_sync_wait() {
        let output = fixture_command()
            .stdout(StreamMode::Piped)
            .stderr(StreamMode::Piped)
            .spawn()
            .await
            .expect("spawn")
            .wait_with_output_bounded(1024)
            .await
            .expect("wait with output");

        assert!(output.status.success());
        let expected = if cfg!(windows) {
            b"async-platform-internal\r\n".as_slice()
        } else {
            b"async-platform-internal".as_slice()
        };
        assert_eq!(output.stdout, expected);
        assert!(output.stderr.is_empty());
    }

    #[tokio::test]
    async fn blessed_spawn_reports_missing_program() {
        let result = SpawnSpec::new("kernal-api-program-that-does-not-exist")
            .spawn()
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn bounded_output_closes_owned_stdin() {
        #[cfg(windows)]
        let spec = shell_spec("more > nul & echo done");
        #[cfg(not(windows))]
        let spec = shell_spec("cat > /dev/null; printf done");

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            spec.stdin(StreamMode::Piped)
                .stdout(StreamMode::Piped)
                .stderr(StreamMode::Piped)
                .spawn()
                .await
                .expect("spawn")
                .wait_with_output_bounded(1024),
        )
        .await
        .expect("stdin is closed for one-shot output")
        .expect("output succeeds");

        let expected = if cfg!(windows) {
            b"done\r\n".as_slice()
        } else {
            b"done".as_slice()
        };
        assert_eq!(output.stdout, expected);
    }

    fn nonzero_capture_command() -> SpawnSpec {
        #[cfg(windows)]
        {
            shell_spec("echo bounded-stdout & echo bounded-stderr 1>&2 & exit /b 7")
        }
        #[cfg(not(windows))]
        {
            shell_spec("printf bounded-stdout; printf bounded-stderr >&2; exit 7")
        }
    }

    fn slow_command() -> SpawnSpec {
        #[cfg(windows)]
        {
            shell_spec("ping 127.0.0.1 -n 31 > nul")
        }
        #[cfg(not(windows))]
        {
            shell_spec("sleep 30")
        }
    }

    fn overflow_command() -> SpawnSpec {
        #[cfg(windows)]
        {
            shell_spec("for /L %i in (1,1,1000000000) do @echo x")
        }
        #[cfg(not(windows))]
        {
            shell_spec("yes x")
        }
    }

    #[test]
    fn bounded_command_preserves_separate_streams_and_raw_nonzero_exit() {
        let output = run_bounded_command(
            nonzero_capture_command(),
            Some(Duration::from_secs(2)),
            1024,
        )
        .expect("bounded nonzero command completes");

        assert_eq!(output.exit.raw_code(), 7);
        assert!(!output.exit.is_success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("bounded-stdout"));
        assert!(String::from_utf8_lossy(&output.stderr).contains("bounded-stderr"));
    }

    #[test]
    fn bounded_command_timeout_hard_kills_and_reaps_promptly() {
        let started = Instant::now();
        let error = run_bounded_command(
            slow_command(),
            Some(Duration::from_millis(100)),
            1024,
        )
        .expect_err("slow command must time out after hard cleanup");

        assert!(matches!(error, BoundedProcessError::TimedOut));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "timeout cleanup was not bounded: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bounded_command_overflow_hard_kills_and_reaps_promptly() {
        let started = Instant::now();
        let error = run_bounded_command(overflow_command(), Some(Duration::from_secs(2)), 16)
            .expect_err("unbounded output must exceed the aggregate capture limit");

        assert!(matches!(
            error,
            BoundedProcessError::OutputLimitExceeded { limit: 16 }
        ));
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "overflow cleanup was not bounded: {:?}",
            started.elapsed()
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_command_timeout_cancels_readers_held_by_an_escaped_descendant() {
        let started = Instant::now();
        let error = run_bounded_command(
            shell_spec("setsid sh -c 'sleep 3' & sleep 30"),
            Some(Duration::from_millis(100)),
            1024,
        )
        .expect_err("outer process must time out");

        assert!(matches!(error, BoundedProcessError::TimedOut));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "escaped descendant retained bounded-capture readers for {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn bounded_command_reports_a_missing_program_as_a_facade_io_error() {
        let error = run_bounded_command(
            SpawnSpec::new("kernal-api-bounded-program-that-does-not-exist"),
            Some(Duration::from_secs(2)),
            1024,
        )
        .expect_err("missing program must fail before capture");

        assert!(matches!(error, BoundedProcessError::Io(_)));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn bounded_command_preserves_non_utf8_program_args_and_environment() {
        use std::os::unix::ffi::OsStringExt;

        let output = run_bounded_command(
            SpawnSpec::new(std::ffi::OsString::from_vec(b"/bin/sh".to_vec()))
                .arg("-c")
                .arg("printf %s \"$KERNAL_API_BOUNDED_BYTES\"")
                .env(
                    std::ffi::OsString::from_vec(b"KERNAL_API_BOUNDED_BYTES".to_vec()),
                    std::ffi::OsString::from_vec(b"value-\xff".to_vec()),
                ),
            Some(Duration::from_secs(2)),
            1024,
        )
        .expect("non-UTF-8 command inputs are preserved");

        assert_eq!(output.stdout, b"value-\xff");
    }

    #[test]
    fn bounded_command_async_uses_the_current_runtime_blocking_lane() {
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        runtime.run(async {
            let started = Instant::now();
            let ticker_at = Arc::new(Mutex::new(None));
            let ticker_at_for_task = Arc::clone(&ticker_at);
            let _ticker = crate::async_engine::launch(async move {
                crate::async_engine::sleep(Duration::from_millis(20)).await;
                *ticker_at_for_task.lock().expect("ticker lock") = Some(started.elapsed());
            });

            let error = run_bounded_command_async(
                slow_command(),
                Some(Duration::from_millis(100)),
                1024,
            )
            .await
            .expect_err("slow command must time out through the blocking lane");

            assert!(matches!(
                error,
                BoundedProcessAsyncError::Bounded(BoundedProcessError::TimedOut)
            ));
            let tick = ticker_at
                .lock()
                .expect("ticker lock")
                .expect("current-thread runtime must keep driving the ticker");
            assert!(
                tick < Duration::from_millis(80),
                "blocking command occupied the current-thread runtime for {tick:?}"
            );
        });
    }
}
