//! Facade vocabulary and parent-only bookkeeping for phase-D worker supervision.
//!
//! Process launch, protocol I/O, and forced containment remain intentionally
//! absent here; this module only owns semantic configuration and lifecycle data.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::{AdmittedSketch, RootExecutionPermit, SketchCompilerConfig, SketchExecutionError, SketchModulePolicy, ThreadedRootOutcome};

const MAX_COOPERATIVE_CANCEL_GRACE: Duration = Duration::from_secs(60);

/// Explicit worker executable and bounded grace before a future supervisor
/// escalates to process containment. No executable discovery occurs here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SketchWorkerConfig {
    executable: PathBuf,
    cooperative_cancel_grace: Duration,
}
impl SketchWorkerConfig {
    pub fn new(executable: PathBuf, cooperative_cancel_grace: Duration) -> Result<Self, SketchWorkerFailure> {
        if executable.as_os_str().is_empty() || !executable.is_absolute() || cooperative_cancel_grace.is_zero() || cooperative_cancel_grace > MAX_COOPERATIVE_CANCEL_GRACE {
            return Err(SketchWorkerFailure::Launch);
        }
        Ok(Self { executable, cooperative_cancel_grace })
    }
    pub fn executable(&self) -> &Path { &self.executable }
    pub fn cooperative_cancel_grace(&self) -> Duration { self.cooperative_cancel_grace }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchWorkerStopReason { Cancelled, DeadlineExceeded, ForcedContainment }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SketchWorkerFailure { Launch, Protocol, UnexpectedExit, ContainmentCleanup }
impl SketchWorkerFailure { pub fn code(self) -> &'static str { match self { Self::Launch => "worker-launch", Self::Protocol => "worker-protocol", Self::UnexpectedExit => "worker-unexpected-exit", Self::ContainmentCleanup => "worker-containment-cleanup" } } }
impl fmt::Display for SketchWorkerFailure { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.code()) } }
impl std::error::Error for SketchWorkerFailure {}

/// Semantic terminal vocabulary for a future worker supervisor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchWorkerTerminal {
    Completed(ThreadedRootOutcome),
    Stopped(SketchWorkerStopReason),
    Execution(SketchExecutionError),
    Failure(SketchWorkerFailure),
}
impl SketchWorkerTerminal {
    pub fn code(&self) -> &'static str { match self { Self::Completed(_) => "worker-completed", Self::Stopped(SketchWorkerStopReason::Cancelled) => "cancelled", Self::Stopped(SketchWorkerStopReason::DeadlineExceeded) => "deadline-exceeded", Self::Stopped(SketchWorkerStopReason::ForcedContainment) => "forced-containment", Self::Execution(error) => error.code(), Self::Failure(error) => error.code() } }
}
impl fmt::Display for SketchWorkerTerminal { fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str(self.code()) } }

/// Parent-only counters. They never merge into the compiler/guest ledger.
#[derive(Default)]
pub(crate) struct WorkerExecutionLedger {
    spawned: AtomicU64, cancel_sent: AtomicU64, grace_expired: AtomicU64,
    forced: AtomicU64, reaped: AtomicU64, protocol_failures: AtomicU64,
}
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct WorkerExecutionSnapshot { pub(crate) spawned: u64, pub(crate) cancel_sent: u64, pub(crate) grace_expired: u64, pub(crate) forced: u64, pub(crate) reaped: u64, pub(crate) protocol_failures: u64 }
impl WorkerExecutionLedger {
    pub(crate) fn snapshot(&self) -> WorkerExecutionSnapshot { WorkerExecutionSnapshot { spawned: self.spawned.load(Ordering::Relaxed), cancel_sent: self.cancel_sent.load(Ordering::Relaxed), grace_expired: self.grace_expired.load(Ordering::Relaxed), forced: self.forced.load(Ordering::Relaxed), reaped: self.reaped.load(Ordering::Relaxed), protocol_failures: self.protocol_failures.load(Ordering::Relaxed) } }
    pub(crate) fn record_spawned(&self) { self.spawned.fetch_add(1, Ordering::Relaxed); }
    pub(crate) fn record_cancel_sent(&self) { self.cancel_sent.fetch_add(1, Ordering::Relaxed); }
    pub(crate) fn record_grace_expired(&self) { self.grace_expired.fetch_add(1, Ordering::Relaxed); }
    pub(crate) fn record_forced(&self) { self.forced.fetch_add(1, Ordering::Relaxed); }
    pub(crate) fn record_reaped(&self) { self.reaped.fetch_add(1, Ordering::Relaxed); }
    pub(crate) fn record_protocol_failure(&self) { self.protocol_failures.fetch_add(1, Ordering::Relaxed); }
}

/// A logical parent admission permit only; it never prepares Wasmtime state.
pub(crate) struct WorkerParentRootLease { _permit: RootExecutionPermit }
impl AdmittedSketch {
    pub(crate) fn worker_source(&self) -> Arc<[u8]> { Arc::clone(&self.worker_source) }
    pub(crate) fn worker_compiler_config(&self) -> SketchCompilerConfig { self.worker_compiler_config }
    pub(crate) fn worker_policy(&self) -> SketchModulePolicy { self.worker_policy }
    pub(crate) fn acquire_worker_parent_root_lease(&self) -> Result<WorkerParentRootLease, SketchExecutionError> { Ok(WorkerParentRootLease { _permit: self.execution_ledger.acquire_root()? }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn worker_config_requires_absolute_path_and_bounded_nonzero_grace() {
        assert_eq!(SketchWorkerConfig::new(PathBuf::from("worker"), Duration::from_secs(1)), Err(SketchWorkerFailure::Launch));
        let absolute = std::env::current_dir().expect("current directory").join("worker");
        assert_eq!(SketchWorkerConfig::new(absolute.clone(), Duration::ZERO), Err(SketchWorkerFailure::Launch));
        assert!(SketchWorkerConfig::new(absolute, Duration::from_millis(1)).is_ok());
    }
    #[test]
    fn terminal_codes_preserve_semantic_categories() {
        assert_eq!(SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled).code(), "cancelled");
        assert_eq!(SketchWorkerTerminal::Stopped(SketchWorkerStopReason::ForcedContainment).code(), "forced-containment");
        assert_eq!(SketchWorkerTerminal::Failure(SketchWorkerFailure::Protocol).code(), "worker-protocol");
    }
    #[test]
    fn parent_lifecycle_ledger_is_separate_and_bounded() {
        let ledger = WorkerExecutionLedger::default();
        ledger.record_spawned(); ledger.record_cancel_sent(); ledger.record_grace_expired(); ledger.record_forced(); ledger.record_reaped(); ledger.record_protocol_failure();
        assert_eq!(ledger.snapshot(), WorkerExecutionSnapshot { spawned: 1, cancel_sent: 1, grace_expired: 1, forced: 1, reaped: 1, protocol_failures: 1 });
    }
}
