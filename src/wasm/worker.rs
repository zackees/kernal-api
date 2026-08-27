//! Facade vocabulary and parent-only bookkeeping for phase-D worker supervision.
//!
//! Process launch, protocol I/O, and forced containment remain intentionally
//! absent here; this module only owns semantic configuration and lifecycle data.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::{
    AdmittedSketch, RootExecutionPermit, SketchCompilerConfig, SketchExecutionError,
    SketchModulePolicy, ThreadedRootOutcome,
};

const MAX_COOPERATIVE_CANCEL_GRACE: Duration = Duration::from_secs(60);

/// Explicit worker executable and bounded grace before a future supervisor
/// escalates to process containment. No executable discovery occurs here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SketchWorkerConfig {
    executable: PathBuf,
    cooperative_cancel_grace: Duration,
}
impl SketchWorkerConfig {
    pub fn new(
        executable: PathBuf,
        cooperative_cancel_grace: Duration,
    ) -> Result<Self, SketchWorkerFailure> {
        if executable.as_os_str().is_empty()
            || !executable.is_absolute()
            || cooperative_cancel_grace.is_zero()
            || cooperative_cancel_grace > MAX_COOPERATIVE_CANCEL_GRACE
        {
            return Err(SketchWorkerFailure::InvalidConfiguration);
        }
        Ok(Self {
            executable,
            cooperative_cancel_grace,
        })
    }
    pub fn executable(&self) -> &Path {
        &self.executable
    }
    pub fn cooperative_cancel_grace(&self) -> Duration {
        self.cooperative_cancel_grace
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchWorkerStopReason {
    Cancelled,
    DeadlineExceeded,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchWorkerFailure {
    InvalidConfiguration,
    Launch,
    Protocol,
    UnexpectedExit,
    ContainmentCleanup,
}
impl SketchWorkerFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "worker-invalid-configuration",
            Self::Launch => "worker-launch",
            Self::Protocol => "worker-protocol",
            Self::UnexpectedExit => "worker-unexpected-exit",
            Self::ContainmentCleanup => "worker-containment-cleanup",
        }
    }
}
impl fmt::Display for SketchWorkerFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for SketchWorkerFailure {}

/// Semantic terminal vocabulary for a future worker supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchWorkerTerminal {
    Completed(ThreadedRootOutcome),
    Stopped(SketchWorkerStopReason),
    ForcedContainment { trigger: SketchWorkerStopReason },
    Execution(SketchExecutionError),
    Failure(SketchWorkerFailure),
}
impl SketchWorkerTerminal {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Completed(_) => "worker-completed",
            Self::Stopped(SketchWorkerStopReason::Cancelled) => "cancelled",
            Self::Stopped(SketchWorkerStopReason::DeadlineExceeded) => "deadline-exceeded",
            Self::ForcedContainment {
                trigger: SketchWorkerStopReason::Cancelled,
            } => "forced-containment-cancelled",
            Self::ForcedContainment {
                trigger: SketchWorkerStopReason::DeadlineExceeded,
            } => "forced-containment-deadline-exceeded",
            Self::Execution(error) => error.code(),
            Self::Failure(error) => error.code(),
        }
    }
}
impl fmt::Display for SketchWorkerTerminal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Parent-only counters. They never merge into the compiler/guest ledger.
#[allow(dead_code)] // Consumed by the phase-D supervisor; unit tests cover its accounting now.
#[derive(Default)]
pub(crate) struct WorkerExecutionLedger {
    spawned: AtomicU64,
    cancel_sent: AtomicU64,
    grace_expired: AtomicU64,
    forced: AtomicU64,
    reaped: AtomicU64,
    protocol_failures: AtomicU64,
}
#[allow(dead_code)] // Snapshot remains private until the phase-D supervisor owns it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerExecutionSnapshot {
    pub(crate) spawned: u64,
    pub(crate) cancel_sent: u64,
    pub(crate) grace_expired: u64,
    pub(crate) forced: u64,
    pub(crate) reaped: u64,
    pub(crate) protocol_failures: u64,
}
#[allow(dead_code)] // See WorkerExecutionLedger.
impl WorkerExecutionLedger {
    pub(crate) fn snapshot(&self) -> WorkerExecutionSnapshot {
        WorkerExecutionSnapshot {
            spawned: self.spawned.load(Ordering::Relaxed),
            cancel_sent: self.cancel_sent.load(Ordering::Relaxed),
            grace_expired: self.grace_expired.load(Ordering::Relaxed),
            forced: self.forced.load(Ordering::Relaxed),
            reaped: self.reaped.load(Ordering::Relaxed),
            protocol_failures: self.protocol_failures.load(Ordering::Relaxed),
        }
    }
    pub(crate) fn record_spawned(&self) {
        self.spawned.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn record_cancel_sent(&self) {
        self.cancel_sent.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn record_grace_expired(&self) {
        self.grace_expired.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn record_forced(&self) {
        self.forced.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn record_reaped(&self) {
        self.reaped.fetch_add(1, Ordering::Relaxed);
    }
    pub(crate) fn record_protocol_failure(&self) {
        self.protocol_failures.fetch_add(1, Ordering::Relaxed);
    }
}

/// A logical parent admission permit only; it never prepares Wasmtime state.
#[allow(dead_code)] // Phase-D acquires this parent-only logical root permit.
pub(crate) struct WorkerParentRootLease {
    _permit: RootExecutionPermit,
}
#[allow(dead_code)] // Retained source/config/policy are consumed by the phase-D worker supervisor.
impl AdmittedSketch {
    pub(crate) fn worker_source(&self) -> Arc<[u8]> {
        Arc::clone(&self.worker_source)
    }
    pub(crate) fn worker_compiler_config(&self) -> SketchCompilerConfig {
        self.worker_compiler_config
    }
    pub(crate) fn worker_policy(&self) -> SketchModulePolicy {
        self.worker_policy
    }
    pub(crate) fn acquire_worker_parent_root_lease(
        &self,
    ) -> Result<WorkerParentRootLease, SketchExecutionError> {
        Ok(WorkerParentRootLease {
            _permit: self.execution_ledger.acquire_root()?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::{SketchCompiler, SketchExecutionSnapshot, THREADED_RUST_MAX_PAGES};
    use super::*;
    #[test]
    fn worker_config_requires_absolute_path_and_bounded_nonzero_grace() {
        assert_eq!(
            SketchWorkerConfig::new(PathBuf::from("worker"), Duration::from_secs(1)),
            Err(SketchWorkerFailure::InvalidConfiguration)
        );
        let absolute = std::env::current_dir()
            .expect("current directory")
            .join("worker");
        assert_eq!(
            SketchWorkerConfig::new(absolute.clone(), Duration::ZERO),
            Err(SketchWorkerFailure::InvalidConfiguration)
        );
        assert!(SketchWorkerConfig::new(absolute, Duration::from_millis(1)).is_ok());
    }
    #[test]
    fn terminal_codes_preserve_semantic_categories() {
        assert_eq!(
            SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled).code(),
            "cancelled"
        );
        assert_eq!(
            SketchWorkerTerminal::ForcedContainment {
                trigger: SketchWorkerStopReason::Cancelled
            }
            .code(),
            "forced-containment-cancelled"
        );
        assert_eq!(
            SketchWorkerTerminal::ForcedContainment {
                trigger: SketchWorkerStopReason::DeadlineExceeded
            }
            .code(),
            "forced-containment-deadline-exceeded"
        );
        assert_eq!(
            SketchWorkerTerminal::Failure(SketchWorkerFailure::Protocol).code(),
            "worker-protocol"
        );
        assert_eq!(
            SketchWorkerFailure::InvalidConfiguration.code(),
            "worker-invalid-configuration"
        );
    }
    #[test]
    fn parent_lifecycle_ledger_is_separate_and_bounded() {
        let ledger = WorkerExecutionLedger::default();
        ledger.record_spawned();
        ledger.record_cancel_sent();
        ledger.record_grace_expired();
        ledger.record_forced();
        ledger.record_reaped();
        ledger.record_protocol_failure();
        assert_eq!(
            ledger.snapshot(),
            WorkerExecutionSnapshot {
                spawned: 1,
                cancel_sent: 1,
                grace_expired: 1,
                forced: 1,
                reaped: 1,
                protocol_failures: 1
            }
        );
    }
    #[test]
    fn admitted_worker_retains_one_shared_source_allocation() {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let bytes = super::super::threaded_root_observation_tests::threaded_yield_fixture();
        let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
            .expect("policy");
        let sketch = compiler.admit(&bytes, policy).expect("admission");
        let first = sketch.worker_source();
        let second = sketch.worker_source();
        assert_eq!(&*first, bytes.as_slice());
        assert!(Arc::ptr_eq(&first, &second));
        assert!(Arc::strong_count(&first) >= 3);
        assert_eq!(
            sketch.worker_compiler_config(),
            SketchCompilerConfig::default()
        );
        assert_eq!(sketch.worker_policy(), policy);
    }
    #[test]
    fn parent_root_lease_never_prepares_guest_state() {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let bytes = super::super::threaded_root_observation_tests::threaded_yield_fixture();
        let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
            .expect("policy");
        let sketch = compiler.admit(&bytes, policy).expect("admission");
        assert_eq!(
            compiler.execution_limits_snapshot(),
            SketchExecutionSnapshot::default()
        );
        let lease = sketch
            .acquire_worker_parent_root_lease()
            .expect("parent lease");
        let during = compiler.execution_limits_snapshot();
        assert_eq!(during.active_root_executions(), 1);
        assert_eq!(during.reserved_shared_memory_bytes(), 0);
        assert_eq!(during.live_stores(), 0);
        assert_eq!(during.live_instances(), 0);
        assert_eq!(during.live_guest_threads(), 0);
        assert_eq!(during.active_epoch_registrations(), 0);
        drop(lease);
        assert_eq!(
            compiler.execution_limits_snapshot(),
            SketchExecutionSnapshot::default()
        );
    }
}
