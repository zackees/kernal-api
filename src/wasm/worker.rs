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
use super::worker_protocol::{self, ExecuteMetadata, FinalCounters, Message, ModuleAssembler, RootOutcome, TerminalKind};

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
    live_workers: AtomicU64,
    live_protocol_tasks: AtomicU64,
    pending_root_leases: AtomicU64,
}
#[allow(dead_code)] // Snapshot remains private until the phase-D supervisor owns it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SketchWorkerExecutionSnapshot {
    pub spawned: u64, pub cancel_sent: u64, pub grace_expired: u64, pub forced: u64, pub reaped: u64, pub protocol_failures: u64,
    pub live_workers: u64, pub live_protocol_tasks: u64, pub pending_root_leases: u64,
}
#[allow(dead_code)] // See WorkerExecutionLedger.
impl WorkerExecutionLedger {
    pub(crate) fn snapshot(&self) -> SketchWorkerExecutionSnapshot {
        SketchWorkerExecutionSnapshot {
            spawned: self.spawned.load(Ordering::Relaxed),
            cancel_sent: self.cancel_sent.load(Ordering::Relaxed),
            grace_expired: self.grace_expired.load(Ordering::Relaxed),
            forced: self.forced.load(Ordering::Relaxed),
            reaped: self.reaped.load(Ordering::Relaxed),
            protocol_failures: self.protocol_failures.load(Ordering::Relaxed),
            live_workers: self.live_workers.load(Ordering::Relaxed),
            live_protocol_tasks: self.live_protocol_tasks.load(Ordering::Relaxed),
            pending_root_leases: self.pending_root_leases.load(Ordering::Relaxed),
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
    fn live_worker(self: &Arc<Self>) -> WorkerGauge { self.live_workers.fetch_add(1, Ordering::Relaxed); WorkerGauge { ledger: Arc::clone(self), kind: GaugeKind::Worker } }
    fn live_protocol(self: &Arc<Self>) -> WorkerGauge { self.live_protocol_tasks.fetch_add(1, Ordering::Relaxed); WorkerGauge { ledger: Arc::clone(self), kind: GaugeKind::Protocol } }
    fn live_lease(self: &Arc<Self>) -> WorkerGauge { self.pending_root_leases.fetch_add(1, Ordering::Relaxed); WorkerGauge { ledger: Arc::clone(self), kind: GaugeKind::Lease } }
}
enum GaugeKind { Worker, Protocol, Lease }
struct WorkerGauge { ledger: Arc<WorkerExecutionLedger>, kind: GaugeKind }
impl Drop for WorkerGauge { fn drop(&mut self) { match self.kind { GaugeKind::Worker => { self.ledger.live_workers.fetch_sub(1, Ordering::Relaxed); }, GaugeKind::Protocol => { self.ledger.live_protocol_tasks.fetch_sub(1, Ordering::Relaxed); }, GaugeKind::Lease => { self.ledger.pending_root_leases.fetch_sub(1, Ordering::Relaxed); } } } }

/// A logical parent admission permit only; it never prepares Wasmtime state.
#[allow(dead_code)] // Phase-D acquires this parent-only logical root permit.
pub(crate) struct WorkerParentRootLease {
    _permit: RootExecutionPermit,
}
#[allow(dead_code)] // Retained source/config/policy are consumed by the phase-D worker supervisor.
impl AdmittedSketch {
    pub fn worker_execution_snapshot(&self) -> SketchWorkerExecutionSnapshot { self.worker_ledger.snapshot() }
    pub async fn execute_threaded_root_contained(self: &Arc<Self>, runtime: crate::async_engine::RuntimeHandle, config: &SketchWorkerConfig) -> SketchWorkerTerminal {
        let source = crate::async_engine::CancellationSource::new();
        self.execute_threaded_root_contained_cancellable(runtime, config, source.token()).await
    }
    pub async fn execute_threaded_root_contained_cancellable(self: &Arc<Self>, runtime: crate::async_engine::RuntimeHandle, config: &SketchWorkerConfig, cancellation: crate::async_engine::CancellationToken) -> SketchWorkerTerminal {
        let sketch = Arc::clone(self); let config = config.clone();
        match runtime.launch_blocking(move || supervise(&sketch, config, cancellation)).await { Ok(value) => value, Err(_) => SketchWorkerTerminal::Failure(SketchWorkerFailure::UnexpectedExit) }
    }
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

fn supervise(sketch: &AdmittedSketch, config: SketchWorkerConfig, cancellation: crate::async_engine::CancellationToken) -> SketchWorkerTerminal {
    let _lease = match sketch.acquire_worker_parent_root_lease() { Ok(value) => value, Err(error) => return SketchWorkerTerminal::Execution(error) };
    let _lease_gauge = sketch.worker_ledger.live_lease();
    if cancellation.is_cancelled() { return SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled); }
    let deadline = std::time::Instant::now() + sketch.worker_compiler_config().execution_limits().epoch_limits().wall_clock_deadline();
    let mut command = std::process::Command::new(config.executable());
    let child = spawn_worker(&mut command);
    let child = match child { Ok(value) => value, Err(_) => return SketchWorkerTerminal::Failure(SketchWorkerFailure::Launch) };
    sketch.worker_ledger.record_spawned(); let _worker_gauge = sketch.worker_ledger.live_worker();
    let (mut control, mut stdin, mut stdout) = child.into_parts();
    let _protocol_gauge = sketch.worker_ledger.live_protocol();
    let id = next_request_id().ok_or(SketchWorkerTerminal::Failure(SketchWorkerFailure::Protocol));
    let id = match id { Ok(value) => value, Err(value) => return value };
    let terminal = (|| -> Result<SketchWorkerTerminal, SketchWorkerFailure> {
        let input = stdin.as_mut().ok_or(SketchWorkerFailure::Protocol)?; let output = stdout.as_mut().ok_or(SketchWorkerFailure::Protocol)?;
        worker_protocol::write_message(input, &Message::Hello { request_id: id }).map_err(|_| SketchWorkerFailure::Protocol)?;
        match worker_protocol::read_message(output).map_err(|_| SketchWorkerFailure::Protocol)? { Message::HelloAck { request_id } if request_id == id => {}, _ => return Err(SketchWorkerFailure::Protocol) }
        let metadata = metadata(sketch, deadline); let source = sketch.worker_source();
        worker_protocol::write_message(input, &Message::ExecuteStart { request_id: id, module_len: source.len() as u64, metadata }).map_err(|_| SketchWorkerFailure::Protocol)?;
        for (sequence, bytes) in source.chunks(worker_protocol::MAX_FRAME_PAYLOAD - 12).enumerate() { if cancellation.is_cancelled() || std::time::Instant::now() >= deadline { return Ok(SketchWorkerTerminal::Stopped(winner(&cancellation, deadline))); } worker_protocol::write_message(input, &Message::ModuleChunk { request_id: id, sequence: u32::try_from(sequence).map_err(|_| SketchWorkerFailure::Protocol)?, bytes: bytes.to_vec() }).map_err(|_| SketchWorkerFailure::Protocol)?; }
        worker_protocol::write_message(input, &Message::ExecuteEnd { request_id: id }).map_err(|_| SketchWorkerFailure::Protocol)?;
        let reader = stdout.take().ok_or(SketchWorkerFailure::Protocol)?;
        let (terminal_tx, terminal_rx) = std::sync::mpsc::sync_channel(1);
        let reader_thread = std::thread::spawn(move || { let mut reader = reader; let _ = terminal_tx.send(worker_protocol::read_message(&mut reader)); });
        let mut selected = None;
        let mut grace_deadline = None;
        loop {
            if let Ok(message) = terminal_rx.try_recv() {
                let _ = reader_thread.join();
                let mapped = map_terminal(message.map_err(|_| SketchWorkerFailure::Protocol)?, id)?;
                return Ok(selected.map_or(mapped, SketchWorkerTerminal::Stopped));
            }
            if selected.is_none() && cancellation.is_cancelled() { selected = Some(SketchWorkerStopReason::Cancelled); }
            if selected.is_none() && std::time::Instant::now() >= deadline { selected = Some(SketchWorkerStopReason::DeadlineExceeded); }
            if let Some(trigger) = selected {
                if grace_deadline.is_none() {
                    sketch.worker_ledger.record_cancel_sent();
                    worker_protocol::write_message(input, &Message::Cancel { request_id: id }).map_err(|_| SketchWorkerFailure::Protocol)?;
                    grace_deadline = Some(std::time::Instant::now() + config.cooperative_cancel_grace());
                }
                if std::time::Instant::now() >= grace_deadline.expect("grace") {
                    sketch.worker_ledger.record_grace_expired();
                    return Ok(SketchWorkerTerminal::ForcedContainment { trigger });
                }
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    })();
    match terminal { Ok(value) => { if control.force_and_reap(Duration::from_secs(5)).is_ok() { sketch.worker_ledger.record_reaped(); value } else { SketchWorkerTerminal::Failure(SketchWorkerFailure::ContainmentCleanup) } }, Err(error) => { sketch.worker_ledger.record_protocol_failure(); let _ = control.force_and_reap(Duration::from_secs(5)); sketch.worker_ledger.record_forced(); SketchWorkerTerminal::Failure(error) } }
}

fn spawn_worker(command: &mut std::process::Command) -> Result<crate::platform::process::WorkerChild, crate::platform::process::WorkerError> { #[cfg(target_os = "windows")] { crate::platform_win::spawn_contained_worker(command, crate::platform::process::WorkerLimits { active_processes: Some(1), ..Default::default() }) } #[cfg(target_os = "linux")] { crate::platform_linux::spawn_contained_worker(command, crate::platform::process::WorkerLimits { active_processes: Some(1), ..Default::default() }) } #[cfg(target_os = "macos")] { crate::platform_macos::spawn_contained_worker(command, crate::platform::process::WorkerLimits { active_processes: Some(1), ..Default::default() }) } }
fn winner(c: &crate::async_engine::CancellationToken, deadline: std::time::Instant) -> SketchWorkerStopReason { if c.is_cancelled() { SketchWorkerStopReason::Cancelled } else { let _ = deadline; SketchWorkerStopReason::DeadlineExceeded } }
fn metadata(sketch: &AdmittedSketch, deadline: std::time::Instant) -> ExecuteMetadata { let config = sketch.worker_compiler_config(); let limits = config.execution_limits(); let fuel = limits.fuel_limits(); let epoch = limits.epoch_limits(); let remaining = deadline.saturating_duration_since(std::time::Instant::now()).max(Duration::from_millis(1)); let policy = sketch.worker_policy(); ExecuteMetadata { max_wasm_stack_bytes: config.max_wasm_stack_bytes() as u64, reserved_memory_bytes: limits.maximum_reserved_shared_memory_bytes(), maximum_active_roots: limits.maximum_active_root_executions() as u64, total_fuel: fuel.total(), root_fuel: fuel.root_slice(), child_fuel: fuel.child_slice(), epoch_deadline_millis: remaining.as_millis().try_into().unwrap_or(u64::MAX), epoch_tick_millis: epoch.tick_interval().as_millis().try_into().unwrap_or(u64::MAX), maximum_epoch_registrations: epoch.maximum_active_registrations() as u64, max_module_bytes: policy.max_module_bytes() as u64, max_shared_memory_pages: policy.max_shared_memory_pages(), max_guest_threads: policy.max_guest_threads() as u64 } }
fn map_terminal(message: Message, id: u64) -> Result<SketchWorkerTerminal, SketchWorkerFailure> { match message { Message::Terminal { request_id, kind: TerminalKind::Completed, detail, counters, .. } if request_id == id && zero(counters) => match detail.root_outcome { RootOutcome::Started => Ok(SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started)), RootOutcome::Exited => Ok(SketchWorkerTerminal::Completed(ThreadedRootOutcome::Exited)), _ => Err(SketchWorkerFailure::Protocol) }, Message::Terminal { request_id, kind: TerminalKind::Cancelled, counters, .. } if request_id == id && zero(counters) => Ok(SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled)), Message::Terminal { request_id, kind: TerminalKind::DeadlineExceeded, counters, .. } if request_id == id && zero(counters) => Ok(SketchWorkerTerminal::Stopped(SketchWorkerStopReason::DeadlineExceeded)), _ => Err(SketchWorkerFailure::Protocol) } }
fn zero(c: FinalCounters) -> bool { c.active_roots == 0 && c.live_stores == 0 && c.live_instances == 0 && c.active_epoch_registrations == 0 && c.live_threads == 0 }
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
fn next_request_id() -> Option<u64> { let value = REQUEST_ID.fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1)).ok()?; (value != 0).then_some(value) }

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
            SketchWorkerExecutionSnapshot {
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
