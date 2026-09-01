//! Process spawning, containment, inspection, termination, and stdio.

pub use crate::{
    assign_child_to_windows_job, cancel_capture_reader, canonical_environment_pairs,
    capture_reader_done, compat_shell_command, configure_exact_trace, configure_process_command,
    configure_sync_contained_command, configure_sync_daemon_command, configure_trampoline_command,
    current_executable_build_id, exact_trace_capability, exit_code, monitor_console_windows,
    parent_has_console, prepare_capture_reader, set_process_name, shell_command,
    soft_terminate_process_group, spawn_sync, spawn_sync_daemon, start_descendant_monitor,
    start_exact_trace, sync_child_native_handle, trampoline_exit_code,
    unix_mark_extra_fds_close_on_exec, CaptureCancellation, PlatformChild, ProcessCaptureError,
    ProcessOutput, SpawnSpec, StreamMode, TracedChild, WindowsJobHandle,
};

/// Host-neutral command options selected by the caller before spawning.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProcessCommandConfig {
    pub creation_flags: Option<u32>,
    pub create_process_group: bool,
    pub nice: Option<i32>,
    pub address_space_limit_bytes: Option<u64>,
}

/// Availability of an invasive, lossless launched-tree trace backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactTraceCapability {
    pub available: bool,
    pub backend: &'static str,
    pub reason: &'static str,
    pub non_invasive_backend: &'static str,
    pub non_invasive_grade: NonInvasiveObservationGrade,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NonInvasiveObservationGrade {
    KernelNotification,
    KernelHintReconciled,
    SnapshotInferred,
}

/// A raw, bounded spawning-thread capture collected while a tracee is stopped.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TraceOriginArtifact {
    pub origin_pid: u32,
    pub thread_id: u32,
    pub architecture: String,
    pub register_format: String,
    pub executable: Option<std::path::PathBuf>,
    pub registers: Vec<u8>,
    pub stack_pointer: Option<u64>,
    pub instruction_pointer: Option<u64>,
    pub stack: Vec<u8>,
    pub truncated: bool,
    pub module_map: Vec<u8>,
    pub module_map_truncated: bool,
}

/// Native launched-tree event produced by an exact trace backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactTraceEvent {
    pub sequence: u64,
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub parent_start_key: Option<u64>,
    pub start_key: Option<u64>,
    pub timestamp: std::time::SystemTime,
    pub kind: ExactTraceEventKind,
    pub executable: Option<std::path::PathBuf>,
    pub argv: Option<Vec<std::ffi::OsString>>,
    pub origin: Option<TraceOriginArtifact>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExactTraceEventKind {
    Spawn,
    Exec,
    Exit {
        exit_code: Option<i32>,
        signal: Option<i32>,
        raw_status: i64,
    },
    Loss {
        reason: String,
    },
}

/// A descendant lifecycle fact reported by the host monitor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescendantEvent {
    Started {
        pid: u32,
        /// Immediate parent of the new descendant, when the discovery
        /// mechanism knows it: the Linux `children`-file walk and the
        /// macOS process-snapshot inversion both do; the Windows job
        /// IOCP notification is PID-only, so it reports `None` rather
        /// than paying a racy toolhelp scan per event.
        parent_pid: Option<u32>,
    },
    Exited(u32),
    /// The platform backend has completed its final reconciliation and no
    /// further descendant events can arrive.
    Completed,
}

/// Shared cancellation handle for a host-native descendant monitor.
pub struct DescendantMonitorStop {
    stopped: std::sync::atomic::AtomicBool,
    mutex: std::sync::Mutex<()>,
    wake: std::sync::Condvar,
}

impl DescendantMonitorStop {
    /// Create an untriggered monitor cancellation handle.
    pub fn new() -> Self {
        Self {
            stopped: std::sync::atomic::AtomicBool::new(false),
            mutex: std::sync::Mutex::new(()),
            wake: std::sync::Condvar::new(),
        }
    }

    /// Report whether monitoring was cancelled.
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Cancel monitoring and wake a sleeping monitor immediately.
    pub fn stop(&self) {
        let _guard = self.mutex.lock().unwrap_or_else(|error| error.into_inner());
        if !self.stopped.swap(true, std::sync::atomic::Ordering::AcqRel) {
            self.wake.notify_all();
        }
    }

    /// Wait until cancelled or `timeout` expires, returning whether cancelled.
    pub fn wait_timeout(&self, timeout: std::time::Duration) -> bool {
        if self.is_stopped() {
            return true;
        }
        let guard = self.mutex.lock().unwrap_or_else(|error| error.into_inner());
        if self.is_stopped() {
            return true;
        }
        let (_guard, _wait_result) = self
            .wake
            .wait_timeout(guard, timeout)
            .unwrap_or_else(|error| error.into_inner());
        self.is_stopped()
    }
}

impl Default for DescendantMonitorStop {
    fn default() -> Self {
        Self::new()
    }
}

/// Identifies one captured child output stream.
#[derive(Clone, Copy)]
pub enum CaptureStream {
    Stdout,
    Stderr,
}

/// Metadata about one visible window observed by console-popup monitoring.
#[derive(Debug, Clone)]
pub struct ConsoleWindowInfo {
    pub pid: u32,
    pub title: String,
    pub hwnd: u64,
}

/// One live process instance, rather than merely an address in the PID table.
///
/// The creation key is deliberately opaque. It is a Windows `FILETIME`, Linux
/// `/proc` start-tick value, or macOS `proc_bsdinfo` timestamp depending on
/// the host, and is meaningful only for equality during this boot session.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ProcessIdentity {
    pid: u32,
    creation_key: [u64; 2],
}

impl ProcessIdentity {
    pub(crate) const fn from_native(pid: u32, creation_key: [u64; 2]) -> Self {
        Self { pid, creation_key }
    }

    /// The process-table address paired with this identity.
    pub const fn pid(self) -> u32 {
        self.pid
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn has_native_key(self, creation_key: [u64; 2]) -> bool {
        self.creation_key == creation_key
    }
}

/// A process was not observable well enough to capture a safe identity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessIdentityUnavailable {
    /// The host denied access to the native creation key.
    PermissionDenied,
    /// This target does not provide the required native observation primitive.
    Unsupported,
}

/// The outcome of resolving a PID to a generation-safe process identity.
#[derive(Debug)]
pub enum ProcessIdentityCapture {
    /// A live process and its exact creation generation.
    Found(ProcessIdentity),
    /// No process currently owns this PID. This is not an identity.
    Exited,
    /// The process may exist, but the host could not obtain its creation key.
    Unavailable(ProcessIdentityUnavailable),
    /// The host failed while obtaining the creation key.
    Error(std::io::Error),
}

/// The result of an action addressed by [`ProcessIdentity`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessIdentityAction {
    /// The action was delivered to the exact process instance.
    Performed,
    /// The original process has already exited. No replacement was touched.
    AlreadyExited,
}

/// Why a generation-safe action was refused.
#[derive(Debug)]
pub enum ProcessIdentityActionError {
    /// The PID is now owned by a different process instance.
    StaleIdentity,
    /// The host could no longer obtain an identity safely.
    Unavailable(ProcessIdentityUnavailable),
    /// The host failed before it could act.
    Host(std::io::Error),
}

impl std::fmt::Display for ProcessIdentityActionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleIdentity => f.write_str("process PID was reused by a different instance"),
            Self::Unavailable(reason) => write!(f, "process identity is unavailable: {reason:?}"),
            Self::Host(error) => write!(f, "process identity action failed: {error}"),
        }
    }
}

impl std::error::Error for ProcessIdentityActionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Host(error) => Some(error),
            Self::StaleIdentity | Self::Unavailable(_) => None,
        }
    }
}

/// Capture a process identity from the host's strongest native creation key.
///
/// Callers must retain this value, not only its [`ProcessIdentity::pid`], for
/// any deferred inspection or mutation.
pub fn capture_identity(pid: u32) -> ProcessIdentityCapture {
    crate::platform_imp::capture_process_identity(pid)
}

/// Forcibly terminate exactly `identity`.
///
/// The host resolves the PID and creation key immediately before signaling.
/// A replacement PID produces [`ProcessIdentityActionError::StaleIdentity`]
/// and is never signaled.
pub fn force_kill(
    identity: ProcessIdentity,
) -> Result<ProcessIdentityAction, ProcessIdentityActionError> {
    crate::platform_imp::force_kill_identity(identity)
}

/// Ask exactly `identity` to terminate gracefully where the host supports it.
pub fn signal_terminate(
    identity: ProcessIdentity,
) -> Result<ProcessIdentityAction, ProcessIdentityActionError> {
    crate::platform_imp::signal_terminate_identity(identity)
}

/// Terminate the observed process and every discovered descendant.
///
/// Every discovered PID is captured as a [`ProcessIdentity`] and revalidated
/// before each signal attempt; a recycled member is skipped rather than
/// becoming a target.
pub fn kill_tree(
    identity: ProcessIdentity,
    timeout: std::time::Duration,
) -> Result<u32, ProcessIdentityActionError> {
    crate::platform_imp::kill_tree_identity(identity, timeout)
}

#[cfg(test)]
pub(crate) fn act_on_current_identity(
    identity: ProcessIdentity,
    capture: impl FnOnce(u32) -> ProcessIdentityCapture,
    action: impl FnOnce() -> Result<(), std::io::Error>,
) -> Result<ProcessIdentityAction, ProcessIdentityActionError> {
    match capture(identity.pid()) {
        ProcessIdentityCapture::Found(current) if current == identity => action()
            .map(|()| ProcessIdentityAction::Performed)
            .map_err(ProcessIdentityActionError::Host),
        ProcessIdentityCapture::Found(_) => Err(ProcessIdentityActionError::StaleIdentity),
        ProcessIdentityCapture::Exited => Ok(ProcessIdentityAction::AlreadyExited),
        ProcessIdentityCapture::Unavailable(reason) => {
            Err(ProcessIdentityActionError::Unavailable(reason))
        }
        ProcessIdentityCapture::Error(error) => Err(ProcessIdentityActionError::Host(error)),
    }
}

/// Private snapshot material for native tree discovery. It never crosses the
/// facade boundary; public callers use [`ProcessIdentity`] instead.
#[cfg(any(target_os = "macos", all(test, target_os = "linux")))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProcessSnapshot {
    pub(crate) identity: ProcessIdentity,
    pub(crate) parent_pid: u32,
}

#[cfg(test)]
mod identity_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn recycled_pid_is_refused_and_never_touches_the_replacement() {
        let original = ProcessIdentity::from_native(41, [10, 0]);
        let replacement = ProcessIdentity::from_native(41, [11, 0]);
        let touched = Cell::new(false);
        let outcome = act_on_current_identity(
            original,
            |_| ProcessIdentityCapture::Found(replacement),
            || {
                touched.set(true);
                Ok(())
            },
        );
        assert!(matches!(
            outcome,
            Err(ProcessIdentityActionError::StaleIdentity)
        ));
        assert!(
            !touched.get(),
            "the replacement process must never be touched"
        );
    }

    #[test]
    fn an_exited_identity_is_idempotent_without_attempting_a_mutation() {
        let identity = ProcessIdentity::from_native(41, [10, 0]);
        let touched = Cell::new(false);
        let outcome = act_on_current_identity(
            identity,
            |_| ProcessIdentityCapture::Exited,
            || {
                touched.set(true);
                Ok(())
            },
        );
        assert!(matches!(outcome, Ok(ProcessIdentityAction::AlreadyExited)));
        assert!(!touched.get());
    }

    #[test]
    fn matching_identity_performs_the_requested_action() {
        let identity = ProcessIdentity::from_native(41, [10, 0]);
        let touched = Cell::new(false);
        let outcome = act_on_current_identity(
            identity,
            |_| ProcessIdentityCapture::Found(identity),
            || {
                touched.set(true);
                Ok(())
            },
        );
        assert!(matches!(outcome, Ok(ProcessIdentityAction::Performed)));
        assert!(touched.get());
    }
}

/// Environment base selected by the shared caller for a synchronous spawn.
///
/// Explicit `Command::env` additions and removals remain on the command and
/// are applied after this base by the selected platform implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SyncEnvironment {
    /// Start with the spawning process's ambient environment.
    Inherit,
    /// Start with this complete, caller-assembled base environment.
    Explicit(Vec<(std::ffi::OsString, std::ffi::OsString)>),
}

/// Private, facade-owned bounds for one contained worker process tree.
///
/// The protocol layer selects these values; platform implementations translate
/// only the bounds their native containment primitive can enforce.
#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerLimits {
    pub(crate) active_processes: Option<u32>,
    pub(crate) process_memory_bytes: Option<u64>,
    pub(crate) job_memory_bytes: Option<u64>,
}

/// Semantic stage at which a contained-worker launch or cleanup failed.
#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerStage {
    Pipe,
    ConfigureContainment,
    Create,
    AssignContainment,
    Resume,
    Terminate,
    Reap,
}

/// Private worker failure without exposing an OS handle or backend type.
#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
#[derive(Debug)]
pub(crate) struct WorkerError {
    stage: WorkerStage,
    source: std::io::Error,
}

#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
impl WorkerError {
    pub(crate) fn new(stage: WorkerStage, source: std::io::Error) -> Self {
        Self { stage, source }
    }

    pub(crate) fn stage(&self) -> WorkerStage {
        self.stage
    }
}

impl std::fmt::Display for WorkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "contained worker failed at {:?}: {}",
            self.stage, self.source
        )
    }
}

impl std::error::Error for WorkerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

/// Platform-private lifecycle implementation for [`WorkerChild`].
#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
pub(crate) trait WorkerChildControl: Send {
    fn try_wait(&mut self) -> std::io::Result<Option<i32>>;
    fn force_and_reap(&mut self, timeout: std::time::Duration) -> Result<(), WorkerError>;
    fn shutdown(&mut self);
}

/// Owned pipes and lifecycle capability for the explicit Wasm worker path.
///
/// This is deliberately crate-private: callers receive protocol semantic
/// outcomes, never native child, Job, process-group, or descriptor handles.
#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
pub(crate) struct WorkerChild {
    stdin: Option<std::process::ChildStdin>,
    stdout: Option<std::process::ChildStdout>,
    pid: u32,
    // `Option` permits the consuming split below without moving a field out
    // of a Drop type.  The sole remaining owner always performs containment.
    inner: Option<Box<dyn WorkerChildControl>>,
    contained: bool,
}

/// The lifecycle half of a split contained worker.
///
/// Pipes are intentionally not retained here: protocol I/O can be owned by
/// independent blocking reader/writer tasks without placing this native
/// lifecycle capability behind an async mutex.
#[allow(dead_code)] // Phase-D private worker supervisor owns it.
pub(crate) struct WorkerControl {
    pid: u32,
    inner: Option<Box<dyn WorkerChildControl>>,
    contained: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkerNormalReap {
    Clean,
    Nonzero,
}

#[allow(dead_code)] // Phase-A foundation; the phase-B supervisor owns it.
impl WorkerChild {
    pub(crate) fn new(
        stdin: Option<std::process::ChildStdin>,
        stdout: Option<std::process::ChildStdout>,
        pid: u32,
        inner: Box<dyn WorkerChildControl>,
    ) -> Self {
        Self {
            stdin,
            stdout,
            pid,
            inner: Some(inner),
            contained: false,
        }
    }

    pub(crate) fn id(&self) -> u32 {
        self.pid
    }
    pub(crate) fn take_stdin(&mut self) -> Option<std::process::ChildStdin> {
        self.stdin.take()
    }
    pub(crate) fn take_stdout(&mut self) -> Option<std::process::ChildStdout> {
        self.stdout.take()
    }
    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        let exit = self
            .inner
            .as_mut()
            .map_or(Ok(None), |inner| inner.try_wait())?;
        if exit.is_some() {
            self.contained = true;
        }
        Ok(exit)
    }

    /// Close the control writer before hard containment, then reap within the
    /// caller-selected bound. Repeated calls are delegated to the platform
    /// owner and remain idempotent.
    pub(crate) fn force_and_reap(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<(), WorkerError> {
        drop(self.stdin.take());
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        inner.force_and_reap(timeout)?;
        self.contained = true;
        Ok(())
    }

    /// Separates protocol pipes from the sole native lifecycle owner.
    ///
    /// `WorkerControl` remains responsible for best-effort containment even
    /// after both pipe owners have been dropped.  This consumes `self`, so no
    /// second Drop implementation can kill or reap the same process tree.
    pub(crate) fn into_parts(
        mut self,
    ) -> (
        WorkerControl,
        Option<std::process::ChildStdin>,
        Option<std::process::ChildStdout>,
    ) {
        let control = WorkerControl {
            pid: self.pid,
            inner: self.inner.take(),
            contained: self.contained,
        };
        let stdin = self.stdin.take();
        let stdout = self.stdout.take();
        (control, stdin, stdout)
    }
}

impl Drop for WorkerChild {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if !self.contained {
            if let Some(inner) = self.inner.as_mut() {
                inner.shutdown();
            }
        }
    }
}

#[allow(dead_code)] // Phase-D private worker supervisor owns it.
impl WorkerControl {
    pub(crate) fn id(&self) -> u32 {
        self.pid
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        let exit = self
            .inner
            .as_mut()
            .map_or(Ok(None), |inner| inner.try_wait())?;
        if exit.is_some() {
            self.contained = true;
        }
        Ok(exit)
    }

    /// Reap a child which is expected to exit normally.  This never sends a
    /// termination signal: callers decide separately whether containment is
    /// required after a timeout or an abnormal exit.
    pub(crate) fn reap_clean(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<WorkerNormalReap, WorkerError> {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match self
                .try_wait()
                .map_err(|source| WorkerError::new(WorkerStage::Reap, source))?
            {
                Some(0) => {
                    self.contained = true;
                    return Ok(WorkerNormalReap::Clean);
                }
                Some(code) => {
                    let _ = code;
                    self.contained = true;
                    return Ok(WorkerNormalReap::Nonzero);
                }
                None if std::time::Instant::now() >= deadline => {
                    return Err(WorkerError::new(
                        WorkerStage::Reap,
                        std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "worker clean reap timed out",
                        ),
                    ));
                }
                None => std::thread::sleep(std::time::Duration::from_millis(1)),
            }
        }
    }

    /// Force containment and reap on a caller-selected blocking lane.  A
    /// failure deliberately retains the backend owner for a later retry or
    /// bounded Drop cleanup.
    pub(crate) fn force_and_reap(
        &mut self,
        timeout: std::time::Duration,
    ) -> Result<(), WorkerError> {
        let Some(inner) = self.inner.as_mut() else {
            return Ok(());
        };
        inner.force_and_reap(timeout)?;
        self.contained = true;
        Ok(())
    }
}

impl Drop for WorkerControl {
    fn drop(&mut self) {
        if !self.contained {
            if let Some(inner) = self.inner.as_mut() {
                inner.shutdown();
            }
        }
    }
}

#[cfg(test)]
mod worker_child_tests {
    use super::{WorkerChild, WorkerChildControl, WorkerError, WorkerStage};
    use std::io;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Default)]
    struct Counts {
        waits: AtomicUsize,
        forces: AtomicUsize,
        shutdowns: AtomicUsize,
        failed_forces_remaining: AtomicUsize,
        observed_exit: AtomicBool,
    }

    struct FakeControl {
        counts: Arc<Counts>,
    }

    impl WorkerChildControl for FakeControl {
        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            self.counts.waits.fetch_add(1, Ordering::Relaxed);
            Ok(self
                .counts
                .observed_exit
                .load(Ordering::Relaxed)
                .then_some(0))
        }

        fn force_and_reap(&mut self, _timeout: Duration) -> Result<(), WorkerError> {
            self.counts.forces.fetch_add(1, Ordering::Relaxed);
            if self
                .counts
                .failed_forces_remaining
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                    remaining.checked_sub(1)
                })
                .is_ok()
            {
                return Err(WorkerError::new(
                    WorkerStage::Reap,
                    io::Error::new(io::ErrorKind::TimedOut, "fake reap timeout"),
                ));
            }
            Ok(())
        }

        fn shutdown(&mut self) {
            self.counts.shutdowns.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn fake_worker(counts: Arc<Counts>) -> WorkerChild {
        WorkerChild::new(None, None, 77, Box::new(FakeControl { counts }))
    }

    #[test]
    fn unsplit_drop_contains_exactly_once() {
        let counts = Arc::new(Counts::default());
        drop(fake_worker(Arc::clone(&counts)));
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn split_pipes_do_not_contain_before_control_drop() {
        let counts = Arc::new(Counts::default());
        let (control, stdin, stdout) = fake_worker(Arc::clone(&counts)).into_parts();
        drop(stdin);
        drop(stdout);
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 0);
        drop(control);
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn successful_force_is_not_repeated_by_drop() {
        let counts = Arc::new(Counts::default());
        let (mut control, _stdin, _stdout) = fake_worker(Arc::clone(&counts)).into_parts();
        control
            .force_and_reap(Duration::from_millis(1))
            .expect("fake force succeeds");
        drop(control);
        assert_eq!(counts.forces.load(Ordering::Relaxed), 1);
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn timed_out_force_retains_control_for_retry_and_drop() {
        let counts = Arc::new(Counts::default());
        counts.failed_forces_remaining.store(1, Ordering::Relaxed);
        let (mut control, _stdin, _stdout) = fake_worker(Arc::clone(&counts)).into_parts();
        let error = control
            .force_and_reap(Duration::from_millis(1))
            .expect_err("first fake force times out");
        assert_eq!(error.stage(), WorkerStage::Reap);
        assert_eq!(control.try_wait().expect("fake wait"), None);
        control
            .force_and_reap(Duration::from_millis(1))
            .expect("retry keeps the backend owner");
        drop(control);
        assert_eq!(counts.forces.load(Ordering::Relaxed), 2);
        assert_eq!(counts.waits.load(Ordering::Relaxed), 1);
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn clean_observed_exit_never_forces() {
        let counts = Arc::new(Counts::default());
        counts.observed_exit.store(true, Ordering::Relaxed);
        let (mut control, _stdin, _stdout) = fake_worker(Arc::clone(&counts)).into_parts();
        assert_eq!(control.try_wait().expect("fake exit"), Some(0));
        drop(control);
        assert_eq!(counts.forces.load(Ordering::Relaxed), 0);
        assert_eq!(counts.shutdowns.load(Ordering::Relaxed), 0);
    }
}

/// Caller-supplied stdio bindings for a contained synchronous child.
///
/// Each stream is independently configured. `drain_timeout` bounds how long
/// wrapper-owned pipe ends remain open after the child exits; `None` leaves
/// pipe closure entirely to the caller. `show_console` only affects Windows.
pub struct SpawnStdio<'a> {
    /// Child standard input source.
    pub stdin: StdioSource<'a>,
    /// Child standard output destination.
    pub stdout: StdioSource<'a>,
    /// Child standard error destination.
    pub stderr: StdioSource<'a>,
    /// Maximum post-exit pipe drain interval.
    pub drain_timeout: Option<std::time::Duration>,
    /// Whether a Windows child may inherit or allocate a visible console.
    pub show_console: bool,
}

impl Default for SpawnStdio<'_> {
    fn default() -> Self {
        Self {
            stdin: StdioSource::Null,
            stdout: StdioSource::Parent,
            stderr: StdioSource::Parent,
            drain_timeout: Some(std::time::Duration::from_secs(2)),
            show_console: false,
        }
    }
}

/// Caller-supplied output bindings for a detached synchronous child.
///
/// Detached children may write only to the platform null device or to a
/// caller-owned file. Parent stdio and anonymous pipes are intentionally not
/// available because either can retain or depend on the launching process.
pub struct DaemonStdio<'a> {
    /// Child standard output destination.
    pub stdout: DaemonStdioSource<'a>,
    /// Child standard error destination.
    pub stderr: DaemonStdioSource<'a>,
}

impl Default for DaemonStdio<'_> {
    fn default() -> Self {
        Self {
            stdout: DaemonStdioSource::Null,
            stderr: DaemonStdioSource::Null,
        }
    }
}

/// Output destination accepted by the detached-child path.
pub enum DaemonStdioSource<'a> {
    /// Route output to the platform null device.
    Null,
    /// Duplicate a caller-owned file into the child.
    File(&'a std::fs::File),
}

/// Standard-stream source or destination for a contained child.
pub enum StdioSource<'a> {
    /// Route the stream to the platform null device.
    Null,
    /// Inherit the matching stream from the parent process.
    Parent,
    /// Duplicate a caller-owned file into the child.
    File(&'a std::fs::File),
    /// Create and return an anonymous parent/child pipe pair.
    Pipe,
}

/// Handle for a detached child that is not terminated when dropped.
pub struct DaemonChild {
    pub(crate) pid: u32,
    pub(crate) inner: Box<dyn DaemonChildControl>,
}

pub(crate) trait DaemonChildControl:
    Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe
{
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<i32>;
    fn try_wait(&mut self) -> std::io::Result<Option<i32>>;
}

impl DaemonChild {
    /// Return the operating-system process identifier.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Terminate the child process.
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.inner.kill()
    }

    /// Wait for the child and return its numeric exit code.
    pub fn wait(&mut self) -> std::io::Result<i32> {
        self.inner.wait()
    }

    /// Return the exit code if the child has finished without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        self.inner.try_wait()
    }
}

/// Handle and optional parent pipe ends for a contained child.
///
/// Dropping this value shuts down the contained process group.
pub struct SpawnedChild {
    /// Writable parent end when standard input was configured as a pipe.
    pub stdin: Option<std::process::ChildStdin>,
    /// Readable parent end when standard output was configured as a pipe.
    pub stdout: Option<std::process::ChildStdout>,
    /// Readable parent end when standard error was configured as a pipe.
    pub stderr: Option<std::process::ChildStderr>,
    pub(crate) pid: u32,
    pub(crate) inner: Box<dyn SpawnedChildControl>,
}

pub(crate) trait SpawnedChildControl:
    Send + Sync + std::panic::UnwindSafe + std::panic::RefUnwindSafe
{
    fn kill(&mut self) -> std::io::Result<()>;
    fn wait(&mut self) -> std::io::Result<i32>;
    fn try_wait(&mut self) -> std::io::Result<Option<i32>>;
    fn shutdown(&mut self);
}

impl SpawnedChild {
    /// Transfer the private process owner and the parent protocol pipes to a
    /// more specialized contained-child facade without running this wrapper's
    /// shutdown-on-drop path.
    #[allow(dead_code)] // Used only by Unix platform worker adapters.
    pub(crate) fn into_worker_parts(
        self,
    ) -> (
        Option<std::process::ChildStdin>,
        Option<std::process::ChildStdout>,
        u32,
        Box<dyn SpawnedChildControl>,
    ) {
        let mut child = std::mem::ManuallyDrop::new(self);
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        drop(child.stderr.take());
        let pid = child.pid;
        // SAFETY: `child` is ManuallyDrop so its Drop implementation cannot
        // shut down `inner`; this is the one ownership transfer of `inner`.
        let inner = unsafe { std::ptr::read(&child.inner) };
        (stdin, stdout, pid, inner)
    }

    /// Return the operating-system process identifier.
    pub fn id(&self) -> u32 {
        self.pid
    }

    /// Forcibly terminate the child on a best-effort basis.
    pub fn kill(&mut self) -> std::io::Result<()> {
        self.inner.kill()
    }

    /// Wait for the child and return its numeric exit code.
    pub fn wait(&mut self) -> std::io::Result<i32> {
        self.inner.wait()
    }

    /// Return the exit code if the child has finished without blocking.
    pub fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        self.inner.try_wait()
    }
}

impl Drop for SpawnedChild {
    fn drop(&mut self) {
        self.inner.shutdown();
    }
}

#[derive(Clone, Copy)]
pub enum ObserverScope {
    SystemWide,
    LaunchedProcessTree,
}
#[derive(Clone, Copy)]
pub enum ObserverCategory {
    File,
    Network,
    Process,
}
#[derive(Clone, Copy)]
pub enum ObserverSupport {
    Supported,
    Partial,
    Unavailable,
}
#[derive(Clone, Copy)]
pub struct ObserverBackend {
    pub support: ObserverSupport,
    pub backend: &'static str,
    pub reason: &'static str,
}
pub use crate::platform_imp::observer_backend;
pub use crate::platform_imp::read_process_cmdline;
pub use crate::platform_imp::read_process_file_handles;

/// Platform-neutral Unix signal selectors used by the compatibility facade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnixSignalKind {
    Interrupt,
    Terminate,
    Kill,
}

pub use crate::{
    unix_set_priority, unix_signal_process, unix_signal_process_group, unix_signal_raw,
};

/// What this host installed so a child outlives its owner no longer than it
/// should.
///
/// The variants name the *guarantee*, not the call that produced it. A caller
/// deciding whether to spawn a supervisor cares that the kernel will not do
/// the reaping for it; whether the kernel would have used a parent-death
/// signal or a job object is not a distinction it can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerDeathCleanup {
    /// The kernel signals this process when its owner exits.
    OwnerDeathSignal,
    /// This process belongs to a container the kernel destroys with its owner.
    KillOnOwnerHandleClose,
    /// This process was already in such a container, installed by someone else.
    AlreadyContained,
    /// The host offers no kernel mechanism; a supervisor must do the reaping.
    SupervisorRequired,
    /// The host offers nothing and no supervisor contract is defined here.
    Unsupported,
}

/// Which step of installing owner-death containment failed.
///
/// The caller's operator-facing messages distinguish these, and rightly: not
/// being allowed to *build* a container is a different situation from
/// building one and not being allowed to *join* it. Collapsing both into one
/// error would make the two indistinguishable in a log, so the stage travels
/// with the error rather than being inferred from the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnerDeathCleanupStage {
    /// Asking the kernel to signal this process when its owner exits.
    RequestSignal,
    /// Creating the container that the kernel destroys with its owner.
    CreateContainer,
    /// Placing this process inside that container.
    JoinContainer,
}

/// A failure to install owner-death containment, and the step it failed at.
#[derive(Debug)]
pub struct OwnerDeathCleanupError {
    /// The step that failed.
    pub stage: OwnerDeathCleanupStage,
    /// What the host reported.
    pub source: std::io::Error,
}

impl std::fmt::Display for OwnerDeathCleanupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.stage, self.source)
    }
}

impl std::error::Error for OwnerDeathCleanupError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub use crate::{
    process_install_owner_death_cleanup as install_owner_death_cleanup,
    process_owner_death_cleanup_target as owner_death_cleanup_target,
};

/// Why a host could not answer a question about a process.
///
/// The three named cases are the ones a caller can act on: a PID that could
/// never name a process, a process that is not there, and a question this
/// host does not answer. Everything else is the host's own report, kept
/// whole rather than flattened into one of the three.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessInspectErrorKind {
    /// The PID is outside the range this host issues.
    InvalidPid,
    /// No process on this host currently has that PID.
    NotFound,
    /// This host has no such primitive.
    Unsupported,
    /// The host was asked and refused, or failed.
    Host,
}

/// A failure to inspect or signal a process, and what kind of failure it was.
#[derive(Debug)]
pub struct ProcessInspectError {
    /// Which of the four situations this is.
    pub kind: ProcessInspectErrorKind,
    /// What the host reported.
    pub source: std::io::Error,
}

impl ProcessInspectError {
    /// Build an error of `kind` carrying the host's last reported error.
    pub fn last_os_error(kind: ProcessInspectErrorKind) -> Self {
        Self {
            kind,
            source: std::io::Error::last_os_error(),
        }
    }

    /// Build an error of `kind` with a message this crate composed itself.
    pub fn stated(kind: ProcessInspectErrorKind, message: &str) -> Self {
        Self {
            kind,
            source: std::io::Error::other(message.to_string()),
        }
    }
}

impl std::fmt::Display for ProcessInspectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.kind, self.source)
    }
}

impl std::error::Error for ProcessInspectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

pub use crate::{process_same_executable_path as same_executable_path, ProcessLiveness};

/// A standing request from the host that this process shut down.
///
/// Hosts deliver this differently -- a POSIX signal, a Windows console
/// control event injected on a thread of the OS's choosing -- but both arrive
/// in a context where almost nothing is safe to do. A handler may not
/// allocate, log, take a lock, or join a thread. So neither host runs the
/// caller's code: each sets one flag, and the caller reads it whenever it is
/// somewhere it can act.
///
/// That is why this is a poll rather than a callback. A callback would invite
/// exactly the work the delivery context forbids.
pub struct ShutdownRequest {
    flag: &'static std::sync::atomic::AtomicBool,
}

impl ShutdownRequest {
    /// Build a handle watching a flag the caller already owns.
    ///
    /// The host implementations use this to hand out a view of their own
    /// static. It is public because a caller that already has a shutdown flag
    /// -- one set by a supervisor protocol, or by a test -- can present it
    /// through the same type rather than the loop it feeds needing two shapes
    /// of "should I stop".
    ///
    /// `'static` is not incidental: a handler set by the OS outlives any
    /// scope, so the flag it writes has to as well.
    pub fn watching(flag: &'static std::sync::atomic::AtomicBool) -> Self {
        Self { flag }
    }

    /// Whether the host has asked this process to shut down.
    ///
    /// Latching, not edge-triggered: once true it stays true, so a caller that
    /// checks between two pieces of work cannot miss a request delivered while
    /// it was busy.
    pub fn requested(&self) -> bool {
        self.flag.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl std::fmt::Debug for ShutdownRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShutdownRequest")
            .field("requested", &self.requested())
            .finish()
    }
}

pub use crate::process_install_shutdown_request_handler as install_shutdown_request_handler;

/// Whether this host can replace the running image with another program.
///
/// Unix can: `execve` keeps the process -- its PID, its open descriptors,
/// its place in the process tree -- and swaps the program underneath.
/// Windows has no equivalent; the nearest thing is starting a successor and
/// exiting, which is a *different* process with a different PID and does not
/// keep anything a parent or supervisor was holding onto.
///
/// Callers that can accept a successor should ask this and fall back. Callers
/// that genuinely need the same process to continue have no fallback, and
/// should treat `false` as unsupported rather than approximating it.
pub use crate::{
    process_can_replace_current_image as can_replace_current_image,
    process_replace_current_image as replace_current_image,
};
