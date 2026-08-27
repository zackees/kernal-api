//! Facade vocabulary and parent-only bookkeeping for phase-D worker supervision.
//!
//! Process launch, protocol I/O, and forced containment remain intentionally
//! absent here; this module only owns semantic configuration and lifecycle data.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "wasm-sketch-worker-test-support")]
mod test_support;

use super::worker_protocol::{
    self, ExecuteMetadata, FinalCounters, Message, RootOutcome, TerminalKind,
};
use super::{
    AdmittedSketch, RootExecutionPermit, SketchCompilerConfig, SketchExecutionError,
    SketchModulePolicy, ThreadSpawnRejectionSummary, ThreadedRootOutcome,
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
    ChildFailure,
    WorkerReportedFailure,
    WorkerForcedContainment,
}
impl SketchWorkerFailure {
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidConfiguration => "worker-invalid-configuration",
            Self::Launch => "worker-launch",
            Self::Protocol => "worker-protocol",
            Self::UnexpectedExit => "worker-unexpected-exit",
            Self::ContainmentCleanup => "worker-containment-cleanup",
            Self::ChildFailure => "worker-child-failure",
            Self::WorkerReportedFailure => "worker-reported-failure",
            Self::WorkerForcedContainment => "worker-forced-containment",
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
    pub spawned: u64,
    pub cancel_sent: u64,
    pub grace_expired: u64,
    pub forced: u64,
    pub reaped: u64,
    pub protocol_failures: u64,
    pub live_workers: u64,
    pub live_protocol_tasks: u64,
    pub pending_root_leases: u64,
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
    fn live_worker(self: &Arc<Self>) -> WorkerGauge {
        self.live_workers.fetch_add(1, Ordering::Relaxed);
        WorkerGauge {
            ledger: Arc::clone(self),
            kind: GaugeKind::Worker,
            #[cfg(test)]
            drop_notification: None,
        }
    }
    fn live_protocol(self: &Arc<Self>) -> WorkerGauge {
        self.live_protocol_tasks.fetch_add(1, Ordering::Relaxed);
        WorkerGauge {
            ledger: Arc::clone(self),
            kind: GaugeKind::Protocol,
            #[cfg(test)]
            drop_notification: None,
        }
    }
    fn live_lease(self: &Arc<Self>) -> WorkerGauge {
        self.pending_root_leases.fetch_add(1, Ordering::Relaxed);
        WorkerGauge {
            ledger: Arc::clone(self),
            kind: GaugeKind::Lease,
            #[cfg(test)]
            drop_notification: None,
        }
    }
}
enum GaugeKind {
    Worker,
    Protocol,
    Lease,
}
struct WorkerGauge {
    ledger: Arc<WorkerExecutionLedger>,
    kind: GaugeKind,
    #[cfg(test)]
    drop_notification: Option<std::sync::mpsc::Sender<()>>,
}
impl Drop for WorkerGauge {
    fn drop(&mut self) {
        match self.kind {
            GaugeKind::Worker => {
                self.ledger.live_workers.fetch_sub(1, Ordering::Relaxed);
            }
            GaugeKind::Protocol => {
                self.ledger
                    .live_protocol_tasks
                    .fetch_sub(1, Ordering::Relaxed);
            }
            GaugeKind::Lease => {
                self.ledger
                    .pending_root_leases
                    .fetch_sub(1, Ordering::Relaxed);
            }
        }
        #[cfg(test)]
        if let Some(notification) = self.drop_notification.take() {
            let _ = notification.send(());
        }
    }
}

/// A logical parent admission permit only; it never prepares Wasmtime state.
#[allow(dead_code)] // Phase-D acquires this parent-only logical root permit.
pub(crate) struct WorkerParentRootLease {
    _permit: RootExecutionPermit,
}
#[allow(dead_code)] // Retained source/config/policy are consumed by the phase-D worker supervisor.
impl AdmittedSketch {
    pub fn worker_execution_snapshot(&self) -> SketchWorkerExecutionSnapshot {
        self.worker_ledger.snapshot()
    }
    pub async fn execute_threaded_root_contained(
        self: &Arc<Self>,
        runtime: crate::async_engine::RuntimeHandle,
        config: &SketchWorkerConfig,
    ) -> SketchWorkerTerminal {
        let source = crate::async_engine::CancellationSource::new();
        self.execute_threaded_root_contained_cancellable(runtime, config, source.token())
            .await
    }
    pub async fn execute_threaded_root_contained_cancellable(
        self: &Arc<Self>,
        runtime: crate::async_engine::RuntimeHandle,
        config: &SketchWorkerConfig,
        cancellation: crate::async_engine::CancellationToken,
    ) -> SketchWorkerTerminal {
        let sketch = Arc::clone(self);
        let config = config.clone();
        match runtime
            .launch_blocking(move || supervise(&sketch, config, cancellation))
            .await
        {
            Ok(value) => value,
            Err(_) => SketchWorkerTerminal::Failure(SketchWorkerFailure::UnexpectedExit),
        }
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

enum WriterCommand {
    Hello {
        request_id: u64,
    },
    Upload {
        request_id: u64,
        source: Arc<[u8]>,
        metadata: ExecuteMetadata,
    },
    Cancel {
        request_id: u64,
    },
    Close,
}
enum WriterEvent {
    Hello(Result<(), ()>),
    Upload(Result<(), ()>),
    Cancel(Result<(), ()>),
}

/// All parent-side resources whose lifetime must extend through deferred
/// containment.  Keeping these together prevents a failed force attempt from
/// synchronously dropping the native control or releasing admission early.
struct ExecutionOwnership {
    control: crate::platform::process::WorkerControl,
    _lease: WorkerParentRootLease,
    _lease_gauge: WorkerGauge,
    _worker_gauge: WorkerGauge,
}

struct ActiveOwnership {
    ownership: Option<ExecutionOwnership>,
    cleanup: CleanupDispatcher,
}

impl std::ops::Deref for ActiveOwnership {
    type Target = crate::platform::process::WorkerControl;

    fn deref(&self) -> &Self::Target {
        &self
            .ownership
            .as_ref()
            .expect("active worker ownership")
            .control
    }
}

impl std::ops::DerefMut for ActiveOwnership {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self
            .ownership
            .as_mut()
            .expect("active worker ownership")
            .control
    }
}

impl ActiveOwnership {
    fn take(&mut self) -> ExecutionOwnership {
        self.ownership.take().expect("active worker ownership")
    }
}

struct CleanupJob {
    ownership: ExecutionOwnership,
    writer_tx: Option<std::sync::mpsc::Sender<WriterCommand>>,
    writer: Option<std::thread::JoinHandle<()>>,
    reader: Option<std::thread::JoinHandle<()>>,
}

struct CleanupDispatcher {
    sender: std::sync::mpsc::Sender<CleanupJob>,
}

impl CleanupDispatcher {
    fn start(ledger: Arc<WorkerExecutionLedger>) -> Result<Self, ()> {
        let (sender, receiver) = std::sync::mpsc::channel::<CleanupJob>();
        std::thread::Builder::new()
            .name("kernal-worker-cleanup".into())
            .spawn(move || {
                while let Ok(mut job) = receiver.recv() {
                    // A failed force leaves the contained process and both
                    // pipe owners alive. Retry until containment completes;
                    // only then can joining the helpers be bounded.
                    while job
                        .ownership
                        .control
                        .force_and_reap(Duration::from_secs(5))
                        .is_err()
                    {
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    ledger.record_forced();
                    ledger.record_reaped();
                    if let Some(writer_tx) = job.writer_tx.take() {
                        let _ = writer_tx.send(WriterCommand::Close);
                    }
                    if let Some(writer) = job.writer.take() {
                        let _ = writer.join();
                    }
                    if let Some(reader) = job.reader.take() {
                        let _ = reader.join();
                    }
                    // `job` drops here, releasing the root lease and gauges.
                }
            })
            .map_err(|_| ())?;
        Ok(Self { sender })
    }

    fn hand_off(&self, job: CleanupJob) {
        if let Err(error) = self.sender.send(job) {
            // The dispatcher is intentionally durable. A dead receiver is an
            // impossible terminal condition; leaking is safer than allowing
            // `WorkerControl::drop` to synchronously invoke shutdown.
            std::mem::forget(error.0);
        }
    }

    fn hand_off_pre_protocol(&self, ownership: ExecutionOwnership) {
        self.hand_off(CleanupJob {
            ownership,
            writer_tx: None,
            writer: None,
            reader: None,
        });
    }
}

fn supervise(
    sketch: &AdmittedSketch,
    config: SketchWorkerConfig,
    cancellation: crate::async_engine::CancellationToken,
) -> SketchWorkerTerminal {
    // This authority begins before admission, spawning, and the handshake.
    let deadline = std::time::Instant::now()
        + sketch
            .worker_compiler_config()
            .execution_limits()
            .epoch_limits()
            .wall_clock_deadline();
    if cancellation.is_cancelled() {
        return SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled);
    }
    if validate_worker_module_len(sketch.module_bytes()).is_err() {
        return SketchWorkerTerminal::Failure(SketchWorkerFailure::InvalidConfiguration);
    }
    let _lease = match sketch.acquire_worker_parent_root_lease() {
        Ok(lease) => lease,
        Err(error) => return SketchWorkerTerminal::Execution(error),
    };
    let _lease_gauge = sketch.worker_ledger.live_lease();
    if let Some(trigger) = selected_stop(&cancellation, deadline) {
        return SketchWorkerTerminal::Stopped(trigger);
    }
    let cleanup = match CleanupDispatcher::start(Arc::clone(&sketch.worker_ledger)) {
        Ok(dispatcher) => dispatcher,
        Err(()) => return SketchWorkerTerminal::Failure(SketchWorkerFailure::Launch),
    };
    let mut command = std::process::Command::new(config.executable());
    let child = match spawn_worker(&mut command) {
        Ok(child) => child,
        Err(_) => return SketchWorkerTerminal::Failure(SketchWorkerFailure::Launch),
    };
    sketch.worker_ledger.record_spawned();
    let worker_gauge = sketch.worker_ledger.live_worker();
    let (native_control, stdin, stdout) = child.into_parts();
    let mut control = ActiveOwnership {
        ownership: Some(ExecutionOwnership {
            control: native_control,
            _lease,
            _lease_gauge,
            _worker_gauge: worker_gauge,
        }),
        cleanup,
    };
    #[cfg(feature = "wasm-sketch-worker-test-support")]
    if let Err(()) = test_support::publish_worker_identity(control.id()) {
        // The marker is an all-or-nothing test observation point.  Do not
        // start protocol work when its observer cannot identify this exact
        // worker; the existing pre-protocol path contains and reaps it.
        return force_pre_protocol(sketch, &mut control, SketchWorkerFailure::UnexpectedExit);
    }
    let (Some(stdin), Some(stdout)) = (stdin, stdout) else {
        return force_pre_protocol(sketch, &mut control, SketchWorkerFailure::Protocol);
    };
    let Some(id) = next_request_id() else {
        drop(stdin);
        drop(stdout);
        return force_pre_protocol(sketch, &mut control, SketchWorkerFailure::Protocol);
    };
    let (write_tx, write_rx) = std::sync::mpsc::channel();
    let (write_done_tx, write_done_rx) = std::sync::mpsc::channel();
    let writer_ledger = Arc::clone(&sketch.worker_ledger);
    let writer = std::thread::spawn(move || {
        let _gauge = writer_ledger.live_protocol();
        let mut input = stdin;
        while let Ok(command) = write_rx.recv() {
            let event = match command {
                WriterCommand::Hello { request_id } => WriterEvent::Hello(
                    worker_protocol::write_message(&mut input, &Message::Hello { request_id })
                        .map_err(|_| ()),
                ),
                WriterCommand::Upload {
                    request_id,
                    source,
                    metadata,
                } => {
                    let result = write_upload(&mut input, request_id, source, metadata);
                    WriterEvent::Upload(result)
                }
                WriterCommand::Cancel { request_id } => WriterEvent::Cancel(
                    worker_protocol::write_message(&mut input, &Message::Cancel { request_id })
                        .map_err(|_| ()),
                ),
                WriterCommand::Close => break,
            };
            if write_done_tx.send(event).is_err() {
                break;
            }
        }
    });
    let (read_tx, read_rx) = std::sync::mpsc::channel();
    let reader_ledger = Arc::clone(&sketch.worker_ledger);
    let reader = std::thread::spawn(move || {
        let _gauge = reader_ledger.live_protocol();
        let mut output = stdout;
        loop {
            let message = worker_protocol::read_message(&mut output).map_err(|_| ());
            let finished = message.is_err() || matches!(message, Ok(Message::Terminal { .. }));
            if read_tx.send(message).is_err() || finished {
                break;
            }
        }
    });
    if write_tx
        .send(WriterCommand::Hello { request_id: id })
        .is_err()
    {
        return force_join_result(
            sketch,
            &mut control,
            write_tx,
            writer,
            reader,
            SketchWorkerFailure::Protocol,
        );
    }
    let mut upload_sent = false;
    let mut upload_complete = false;
    let mut cancel_written = false;
    let mut cancel_queued = false;
    let mut hello_acked = false;
    // A malicious worker may write Terminal immediately after HelloAck. Keep
    // exactly one bounded pending frame, but never accept it until the parent
    // observed a successful complete upload.
    let mut execute_acked = false;
    let mut selected = None;
    let mut grace_deadline = None;
    loop {
        if selected.is_none() {
            selected = selected_stop(&cancellation, deadline);
        }
        if let Some(trigger) = selected {
            if !upload_complete {
                return force_join_terminal(
                    sketch,
                    &mut control,
                    write_tx,
                    writer,
                    reader,
                    trigger,
                );
            }
        }
        if let Ok(event) = write_done_rx.try_recv() {
            match event {
                WriterEvent::Hello(Ok(())) => {}
                WriterEvent::Upload(Ok(())) => upload_complete = true,
                WriterEvent::Cancel(Ok(())) => {
                    cancel_written = true;
                    sketch.worker_ledger.record_cancel_sent();
                    grace_deadline =
                        Some(std::time::Instant::now() + config.cooperative_cancel_grace());
                }
                WriterEvent::Hello(Err(()))
                | WriterEvent::Upload(Err(()))
                | WriterEvent::Cancel(Err(())) => {
                    return force_join_result(
                        sketch,
                        &mut control,
                        write_tx,
                        writer,
                        reader,
                        SketchWorkerFailure::Protocol,
                    )
                }
            }
        }
        if let Ok(result) = read_rx.try_recv() {
            let message = match result {
                Ok(message) => message,
                Err(()) => match control.try_wait() {
                    Ok(Some(_)) => return exited_join_result(sketch, write_tx, writer, reader),
                    Ok(None) | Err(_) => {
                        return force_join_result(
                            sketch,
                            &mut control,
                            write_tx,
                            writer,
                            reader,
                            SketchWorkerFailure::Protocol,
                        )
                    }
                },
            };
            match message {
                Message::HelloAck { request_id } if request_id == id && !hello_acked => {
                    hello_acked = true;
                    if write_tx
                        .send(WriterCommand::Upload {
                            request_id: id,
                            source: sketch.worker_source(),
                            metadata: metadata(sketch, deadline),
                        })
                        .is_err()
                    {
                        return force_join_result(
                            sketch,
                            &mut control,
                            write_tx,
                            writer,
                            reader,
                            SketchWorkerFailure::Protocol,
                        );
                    }
                    upload_sent = true;
                }
                Message::ExecuteAck { request_id } if hello_acked && upload_sent && upload_complete && !execute_acked && request_id == id => {
                    execute_acked = true;
                }
                Message::Terminal { .. } if hello_acked && execute_acked => {
                    let mapped = map_terminal(message, id);
                    let mapped = match mapped {
                        Ok(value) => value,
                        Err(error) => {
                            return force_join_result(
                                sketch,
                                &mut control,
                                write_tx,
                                writer,
                                reader,
                                error,
                            )
                        }
                    };
                    if matches!(
                        mapped,
                        SketchWorkerTerminal::Failure(SketchWorkerFailure::Protocol)
                    ) {
                        sketch.worker_ledger.record_protocol_failure();
                    }
                    match control.reap_clean(Duration::from_secs(5)) {
                        Ok(crate::platform::process::WorkerNormalReap::Clean) => {
                            let _ = write_tx.send(WriterCommand::Close);
                            let _ = writer.join();
                            let _ = reader.join();
                            sketch.worker_ledger.record_reaped();
                            return selected.map_or(mapped, SketchWorkerTerminal::Stopped);
                        }
                        Ok(crate::platform::process::WorkerNormalReap::Nonzero) => {
                            let _ = write_tx.send(WriterCommand::Close);
                            let _ = writer.join();
                            let _ = reader.join();
                            sketch.worker_ledger.record_reaped();
                            return SketchWorkerTerminal::Failure(
                                SketchWorkerFailure::UnexpectedExit,
                            );
                        }
                        Err(_) => {
                            return force_join_result(
                                sketch,
                                &mut control,
                                write_tx,
                                writer,
                                reader,
                                SketchWorkerFailure::ContainmentCleanup,
                            )
                        }
                    }
                }
                _ => {
                    return force_join_result(
                        sketch,
                        &mut control,
                        write_tx,
                        writer,
                        reader,
                        SketchWorkerFailure::Protocol,
                    )
                }
            }
        }
        match control.try_wait() {
            Ok(Some(_)) => return exited_join_result(sketch, write_tx, writer, reader),
            Ok(None) => {}
            Err(_) => {
                return force_join_result(
                    sketch,
                    &mut control,
                    write_tx,
                    writer,
                    reader,
                    SketchWorkerFailure::UnexpectedExit,
                )
            }
        }
        if let Some(trigger) = selected {
            if !upload_complete {
                return force_join_terminal(
                    sketch,
                    &mut control,
                    write_tx,
                    writer,
                    reader,
                    trigger,
                );
            }
            if !cancel_written && !cancel_queued && upload_sent {
                if write_tx
                    .send(WriterCommand::Cancel { request_id: id })
                    .is_err()
                {
                    return force_join_result(
                        sketch,
                        &mut control,
                        write_tx,
                        writer,
                        reader,
                        SketchWorkerFailure::Protocol,
                    );
                }
                cancel_queued = true;
                grace_deadline =
                    Some(std::time::Instant::now() + config.cooperative_cancel_grace());
            }
            if let Some(grace_deadline) = grace_deadline {
                if std::time::Instant::now() >= grace_deadline {
                    sketch.worker_ledger.record_grace_expired();
                    return force_join_terminal(
                        sketch,
                        &mut control,
                        write_tx,
                        writer,
                        reader,
                        trigger,
                    );
                }
            }
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// Validate the private process-transport limit without constraining direct
/// in-process admission/execution policy.
fn validate_worker_module_len(module_bytes: usize) -> Result<(), SketchWorkerFailure> {
    (u64::try_from(module_bytes)
        .ok()
        .filter(|bytes| *bytes <= worker_protocol::WORKER_PROTOCOL_MAX_MODULE_BYTES)
        .is_some())
    .then_some(())
    .ok_or(SketchWorkerFailure::InvalidConfiguration)
}

/// Bounded acceptance rule exercised by the supervisor ordering regression:
/// one terminal may wait for upload success, never for upload failure.
#[cfg(test)]
#[derive(Default)]
struct UploadTerminalGate {
    upload_complete: bool,
    deferred: bool,
}
#[cfg(test)]
impl UploadTerminalGate {
    fn terminal(&mut self) -> Result<bool, SketchWorkerFailure> {
        if self.upload_complete {
            return Ok(true);
        }
        if self.deferred {
            return Err(SketchWorkerFailure::Protocol);
        }
        self.deferred = true;
        Ok(false)
    }
    fn upload(&mut self, success: bool) -> Result<bool, SketchWorkerFailure> {
        if !success {
            return Err(SketchWorkerFailure::Protocol);
        }
        self.upload_complete = true;
        Ok(std::mem::take(&mut self.deferred))
    }
}

fn selected_stop(
    cancellation: &crate::async_engine::CancellationToken,
    deadline: std::time::Instant,
) -> Option<SketchWorkerStopReason> {
    select_stop(
        cancellation.is_cancelled(),
        std::time::Instant::now() >= deadline,
    )
}
fn select_stop(cancelled: bool, deadline_elapsed: bool) -> Option<SketchWorkerStopReason> {
    if cancelled {
        Some(SketchWorkerStopReason::Cancelled)
    } else if deadline_elapsed {
        Some(SketchWorkerStopReason::DeadlineExceeded)
    } else {
        None
    }
}
fn write_upload(
    input: &mut std::process::ChildStdin,
    request_id: u64,
    source: Arc<[u8]>,
    metadata: ExecuteMetadata,
) -> Result<(), ()> {
    worker_protocol::write_message(
        input,
        &Message::ExecuteStart {
            request_id,
            module_len: source.len() as u64,
            metadata,
        },
    )
    .map_err(|_| ())?;
    for (sequence, bytes) in source
        .chunks(worker_protocol::MAX_FRAME_PAYLOAD - 12)
        .enumerate()
    {
        let sequence = u32::try_from(sequence).map_err(|_| ())?;
        worker_protocol::write_message(
            input,
            &Message::ModuleChunk {
                request_id,
                sequence,
                bytes: bytes.to_vec(),
            },
        )
        .map_err(|_| ())?;
    }
    worker_protocol::write_message(input, &Message::ExecuteEnd { request_id }).map_err(|_| ())
}
fn force_result(
    ledger: &WorkerExecutionLedger,
    control: &mut crate::platform::process::WorkerControl,
    failure: SketchWorkerFailure,
) -> (SketchWorkerTerminal, bool) {
    if failure == SketchWorkerFailure::Protocol {
        ledger.record_protocol_failure();
    }
    match control.force_and_reap(Duration::from_secs(5)) {
        Ok(()) => {
            ledger.record_forced();
            ledger.record_reaped();
            (SketchWorkerTerminal::Failure(failure), true)
        }
        Err(_) => (
            SketchWorkerTerminal::Failure(SketchWorkerFailure::ContainmentCleanup),
            false,
        ),
    }
}
fn force_pre_protocol(
    sketch: &AdmittedSketch,
    control: &mut ActiveOwnership,
    failure: SketchWorkerFailure,
) -> SketchWorkerTerminal {
    force_pre_protocol_with_ledger(&sketch.worker_ledger, control, failure)
}
fn force_pre_protocol_with_ledger(
    ledger: &WorkerExecutionLedger,
    control: &mut ActiveOwnership,
    failure: SketchWorkerFailure,
) -> SketchWorkerTerminal {
    let (result, forced) = force_result(ledger, &mut *control, failure);
    if !forced {
        let ownership = control.take();
        control.cleanup.hand_off_pre_protocol(ownership);
    }
    result
}
fn force_join_result(
    sketch: &AdmittedSketch,
    control: &mut ActiveOwnership,
    tx: std::sync::mpsc::Sender<WriterCommand>,
    writer: std::thread::JoinHandle<()>,
    reader: std::thread::JoinHandle<()>,
    failure: SketchWorkerFailure,
) -> SketchWorkerTerminal {
    force_join_result_with_ledger(&sketch.worker_ledger, control, tx, writer, reader, failure)
}
fn force_join_result_with_ledger(
    ledger: &WorkerExecutionLedger,
    control: &mut ActiveOwnership,
    tx: std::sync::mpsc::Sender<WriterCommand>,
    writer: std::thread::JoinHandle<()>,
    reader: std::thread::JoinHandle<()>,
    failure: SketchWorkerFailure,
) -> SketchWorkerTerminal {
    let (result, forced) = force_result(ledger, &mut *control, failure);
    if !forced {
        // The dispatcher owns the unreaped child and both helpers from this
        // point. `ContainmentCleanup` means cleanup remains pending; gauges
        // and the root lease intentionally stay live until its retry succeeds.
        let ownership = control.take();
        control.cleanup.hand_off(CleanupJob {
            ownership,
            writer_tx: Some(tx),
            writer: Some(writer),
            reader: Some(reader),
        });
        return result;
    }
    let _ = tx.send(WriterCommand::Close);
    let _ = writer.join();
    let _ = reader.join();
    result
}
fn exited_join_result(
    sketch: &AdmittedSketch,
    tx: std::sync::mpsc::Sender<WriterCommand>,
    writer: std::thread::JoinHandle<()>,
    reader: std::thread::JoinHandle<()>,
) -> SketchWorkerTerminal {
    let _ = tx.send(WriterCommand::Close);
    let _ = writer.join();
    let _ = reader.join();
    sketch.worker_ledger.record_reaped();
    SketchWorkerTerminal::Failure(SketchWorkerFailure::UnexpectedExit)
}
fn force_join_terminal(
    sketch: &AdmittedSketch,
    control: &mut ActiveOwnership,
    tx: std::sync::mpsc::Sender<WriterCommand>,
    writer: std::thread::JoinHandle<()>,
    reader: std::thread::JoinHandle<()>,
    trigger: SketchWorkerStopReason,
) -> SketchWorkerTerminal {
    let (result, forced) = force_result(
        &sketch.worker_ledger,
        &mut *control,
        SketchWorkerFailure::UnexpectedExit,
    );
    if !forced {
        let ownership = control.take();
        control.cleanup.hand_off(CleanupJob {
            ownership,
            writer_tx: Some(tx),
            writer: Some(writer),
            reader: Some(reader),
        });
        return result;
    }
    let _ = tx.send(WriterCommand::Close);
    let _ = writer.join();
    let _ = reader.join();
    match result {
        SketchWorkerTerminal::Failure(SketchWorkerFailure::ContainmentCleanup) => result,
        _ => SketchWorkerTerminal::ForcedContainment { trigger },
    }
}

fn spawn_worker(
    command: &mut std::process::Command,
) -> Result<crate::platform::process::WorkerChild, crate::platform::process::WorkerError> {
    crate::platform_imp::spawn_contained_worker(
        command,
        crate::platform::process::WorkerLimits {
            active_processes: Some(1),
            ..Default::default()
        },
    )
}
fn metadata(sketch: &AdmittedSketch, deadline: std::time::Instant) -> ExecuteMetadata {
    let config = sketch.worker_compiler_config();
    let limits = config.execution_limits();
    let fuel = limits.fuel_limits();
    let epoch = limits.epoch_limits();
    let remaining = deadline
        .saturating_duration_since(std::time::Instant::now())
        .max(Duration::from_millis(1));
    let policy = sketch.worker_policy();
    ExecuteMetadata {
        max_wasm_stack_bytes: config.max_wasm_stack_bytes() as u64,
        reserved_memory_bytes: limits.maximum_reserved_shared_memory_bytes(),
        maximum_active_roots: limits.maximum_active_root_executions() as u64,
        total_fuel: fuel.total(),
        root_fuel: fuel.root_slice(),
        child_fuel: fuel.child_slice(),
        epoch_deadline_millis: remaining.as_millis().try_into().unwrap_or(u64::MAX),
        epoch_tick_millis: epoch
            .tick_interval()
            .as_millis()
            .try_into()
            .unwrap_or(u64::MAX),
        maximum_epoch_registrations: epoch.maximum_active_registrations() as u64,
        max_module_bytes: policy.max_module_bytes() as u64,
        max_shared_memory_pages: policy.max_shared_memory_pages(),
        max_guest_threads: policy.max_guest_threads() as u64,
    }
}
fn map_terminal(message: Message, id: u64) -> Result<SketchWorkerTerminal, SketchWorkerFailure> {
    let Message::Terminal {
        request_id,
        kind,
        detail,
        counters,
        ..
    } = message
    else {
        return Err(SketchWorkerFailure::Protocol);
    };
    if request_id != id || !zero(counters) {
        return Err(SketchWorkerFailure::Protocol);
    }
    let rejected = ThreadSpawnRejectionSummary::from_worker_counts(
        detail.capacity_rejections,
        detail.closing_rejections,
        detail.fuel_rejections,
        detail.epoch_rejections,
    );
    let terminal = match kind {
        TerminalKind::Completed => match detail.root_outcome {
            RootOutcome::Started => SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started),
            RootOutcome::Exited => SketchWorkerTerminal::Completed(ThreadedRootOutcome::Exited),
            RootOutcome::StartedWithThreadRejections => SketchWorkerTerminal::Completed(
                ThreadedRootOutcome::StartedWithThreadRejections(rejected),
            ),
            RootOutcome::ExitedWithThreadRejections => SketchWorkerTerminal::Completed(
                ThreadedRootOutcome::ExitedWithThreadRejections(rejected),
            ),
            RootOutcome::None => return Err(SketchWorkerFailure::Protocol),
        },
        TerminalKind::Cancelled => SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled),
        TerminalKind::DeadlineExceeded => {
            SketchWorkerTerminal::Stopped(SketchWorkerStopReason::DeadlineExceeded)
        }
        TerminalKind::OutOfFuel => SketchWorkerTerminal::Execution(SketchExecutionError::OutOfFuel),
        TerminalKind::Trapped => SketchWorkerTerminal::Execution(SketchExecutionError::Trapped),
        TerminalKind::NonzeroExit => {
            SketchWorkerTerminal::Execution(SketchExecutionError::NonzeroExit {
                code: detail.status_code.ok_or(SketchWorkerFailure::Protocol)?,
            })
        }
        TerminalKind::ChildFailure => match detail.status_code {
            Some(code) => {
                SketchWorkerTerminal::Execution(SketchExecutionError::ChildNonzeroExit { code })
            }
            None => SketchWorkerTerminal::Failure(SketchWorkerFailure::ChildFailure),
        },
        TerminalKind::WorkerFailure => {
            SketchWorkerTerminal::Failure(SketchWorkerFailure::WorkerReportedFailure)
        }
        TerminalKind::ProtocolFailure => {
            SketchWorkerTerminal::Failure(SketchWorkerFailure::Protocol)
        }
        TerminalKind::ForcedContainment => {
            SketchWorkerTerminal::Failure(SketchWorkerFailure::WorkerForcedContainment)
        }
    };
    Ok(terminal)
}
fn zero(c: FinalCounters) -> bool {
    c.active_roots == 0
        && c.live_stores == 0
        && c.live_instances == 0
        && c.active_epoch_registrations == 0
        && c.live_threads == 0
}
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
fn next_request_id() -> Option<u64> {
    let value = REQUEST_ID
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |id| id.checked_add(1))
        .ok()?;
    (value != 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::super::{SketchCompiler, SketchExecutionSnapshot, THREADED_RUST_MAX_PAGES};
    use super::*;
    use crate::platform::process::{WorkerChild, WorkerChildControl, WorkerError, WorkerStage};
    use std::io;
    use std::sync::{mpsc, Condvar, Mutex};

    const TEST_WAIT: Duration = Duration::from_secs(2);

    struct DeferredFake {
        calls: Arc<AtomicU64>,
        shutdown_threads: Arc<Mutex<Vec<std::thread::ThreadId>>>,
        drop_threads: Arc<Mutex<Vec<std::thread::ThreadId>>>,
        first_failure: Mutex<Option<mpsc::Sender<()>>>,
        retry_gate: Arc<(Mutex<bool>, Condvar)>,
        successful_reap: Mutex<Option<mpsc::Sender<()>>>,
    }

    impl WorkerChildControl for DeferredFake {
        fn try_wait(&mut self) -> io::Result<Option<i32>> {
            Ok(None)
        }

        fn force_and_reap(&mut self, _timeout: Duration) -> Result<(), WorkerError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            if call == 0 {
                if let Some(first_failure) = self
                    .first_failure
                    .lock()
                    .expect("first failure lock")
                    .take()
                {
                    let _ = first_failure.send(());
                }
                return Err(WorkerError::new(
                    WorkerStage::Reap,
                    io::Error::new(io::ErrorKind::TimedOut, "controlled first failure"),
                ));
            }
            let (released, wake) = &*self.retry_gate;
            let mut released = released.lock().expect("retry gate lock");
            while !*released {
                released = wake.wait(released).expect("retry gate poisoned");
            }
            if let Some(successful_reap) = self
                .successful_reap
                .lock()
                .expect("successful reap lock")
                .take()
            {
                let _ = successful_reap.send(());
            }
            Ok(())
        }

        fn shutdown(&mut self) {
            self.shutdown_threads
                .lock()
                .expect("shutdown threads lock")
                .push(std::thread::current().id());
        }
    }

    impl Drop for DeferredFake {
        fn drop(&mut self) {
            self.drop_threads
                .lock()
                .expect("drop threads lock")
                .push(std::thread::current().id());
        }
    }

    fn deferred_ownership(
        ledger: &Arc<WorkerExecutionLedger>,
        fake: DeferredFake,
        released: Option<mpsc::Sender<()>>,
    ) -> ExecutionOwnership {
        let execution_ledger = Arc::new(super::super::ExecutionLedger::new(
            super::super::SketchExecutionLimits::default(),
        ));
        let lease = WorkerParentRootLease {
            _permit: execution_ledger.acquire_root().expect("test root lease"),
        };
        let (control, _, _) = WorkerChild::new(None, None, 77, Box::new(fake)).into_parts();
        let mut worker_gauge = ledger.live_worker();
        worker_gauge.drop_notification = released;
        ExecutionOwnership {
            control,
            _lease: lease,
            _lease_gauge: ledger.live_lease(),
            _worker_gauge: worker_gauge,
        }
    }

    fn release_retry(retry_gate: &Arc<(Mutex<bool>, Condvar)>) {
        let (released, wake) = &**retry_gate;
        *released.lock().expect("retry gate lock") = true;
        wake.notify_all();
    }

    #[test]
    fn failed_force_handoff_returns_without_caller_shutdown() {
        let ledger = Arc::new(WorkerExecutionLedger::default());
        let (first_failure_tx, first_failure_rx) = mpsc::channel();
        let retry_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let shutdown_threads = Arc::new(Mutex::new(Vec::new()));
        let drop_threads = Arc::new(Mutex::new(Vec::new()));
        let ownership = deferred_ownership(
            &ledger,
            DeferredFake {
                calls: Arc::new(AtomicU64::new(0)),
                shutdown_threads: Arc::clone(&shutdown_threads),
                drop_threads: Arc::clone(&drop_threads),
                first_failure: Mutex::new(Some(first_failure_tx)),
                retry_gate: Arc::clone(&retry_gate),
                successful_reap: Mutex::new(None),
            },
            None,
        );
        let cleanup = CleanupDispatcher::start(Arc::clone(&ledger)).expect("cleanup dispatcher");
        let mut active = ActiveOwnership {
            ownership: Some(ownership),
            cleanup,
        };
        let caller = std::thread::current().id();
        let (writer_tx, writer_rx) = mpsc::channel();
        let (writer_closed_tx, writer_closed_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            assert!(matches!(writer_rx.recv(), Ok(WriterCommand::Close)));
            let _ = writer_closed_tx.send(());
        });
        let reader = std::thread::spawn(|| {});

        let terminal = force_join_result_with_ledger(
            &ledger,
            &mut active,
            writer_tx,
            writer,
            reader,
            SketchWorkerFailure::Protocol,
        );
        assert_eq!(
            terminal,
            SketchWorkerTerminal::Failure(SketchWorkerFailure::ContainmentCleanup)
        );
        assert!(active.ownership.is_none());
        assert!(first_failure_rx.recv_timeout(TEST_WAIT).is_ok());
        assert!(shutdown_threads
            .lock()
            .expect("shutdown threads lock")
            .iter()
            .all(|id| *id != caller));
        assert!(drop_threads
            .lock()
            .expect("drop threads lock")
            .iter()
            .all(|id| *id != caller));
        release_retry(&retry_gate);
        assert!(writer_closed_rx.recv_timeout(TEST_WAIT).is_ok());
    }

    #[test]
    fn dispatcher_retry_releases_ownership_and_records_one_forced_reap() {
        let ledger = Arc::new(WorkerExecutionLedger::default());
        let (first_failure_tx, first_failure_rx) = mpsc::channel();
        let (successful_reap_tx, successful_reap_rx) = mpsc::channel();
        let (released_tx, released_rx) = mpsc::channel();
        let retry_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let calls = Arc::new(AtomicU64::new(0));
        let ownership = deferred_ownership(
            &ledger,
            DeferredFake {
                calls: Arc::clone(&calls),
                shutdown_threads: Arc::new(Mutex::new(Vec::new())),
                drop_threads: Arc::new(Mutex::new(Vec::new())),
                first_failure: Mutex::new(Some(first_failure_tx)),
                retry_gate: Arc::clone(&retry_gate),
                successful_reap: Mutex::new(Some(successful_reap_tx)),
            },
            Some(released_tx),
        );
        let cleanup = CleanupDispatcher::start(Arc::clone(&ledger)).expect("cleanup dispatcher");
        cleanup.hand_off_pre_protocol(ownership);
        assert!(first_failure_rx.recv_timeout(TEST_WAIT).is_ok());
        release_retry(&retry_gate);
        assert!(successful_reap_rx.recv_timeout(TEST_WAIT).is_ok());
        assert!(released_rx.recv_timeout(TEST_WAIT).is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(ledger.snapshot().forced, 1);
        assert_eq!(ledger.snapshot().reaped, 1);
        assert_eq!(ledger.snapshot().live_workers, 0);
        assert_eq!(ledger.snapshot().live_protocol_tasks, 0);
        assert_eq!(ledger.snapshot().pending_root_leases, 0);
    }

    #[test]
    fn closed_cleanup_receiver_leaks_whole_job_without_caller_shutdown() {
        let ledger = Arc::new(WorkerExecutionLedger::default());
        let shutdown_threads = Arc::new(Mutex::new(Vec::new()));
        let drop_threads = Arc::new(Mutex::new(Vec::new()));
        let retry_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let ownership = deferred_ownership(
            &ledger,
            DeferredFake {
                calls: Arc::new(AtomicU64::new(0)),
                shutdown_threads: Arc::clone(&shutdown_threads),
                drop_threads: Arc::clone(&drop_threads),
                first_failure: Mutex::new(None),
                retry_gate,
                successful_reap: Mutex::new(None),
            },
            None,
        );
        let (sender, receiver) = mpsc::channel();
        drop(receiver);
        let cleanup = CleanupDispatcher { sender };
        let caller = std::thread::current().id();
        cleanup.hand_off_pre_protocol(ownership);
        assert!(shutdown_threads
            .lock()
            .expect("shutdown threads lock")
            .iter()
            .all(|id| *id != caller));
        assert!(drop_threads
            .lock()
            .expect("drop threads lock")
            .iter()
            .all(|id| *id != caller));
        assert_eq!(ledger.snapshot().live_workers, 1);
        assert_eq!(ledger.snapshot().pending_root_leases, 1);
    }

    #[test]
    fn pre_protocol_failed_force_hands_off_without_pipe_helpers() {
        let ledger = Arc::new(WorkerExecutionLedger::default());
        let (first_failure_tx, first_failure_rx) = mpsc::channel();
        let retry_gate = Arc::new((Mutex::new(false), Condvar::new()));
        let ownership = deferred_ownership(
            &ledger,
            DeferredFake {
                calls: Arc::new(AtomicU64::new(0)),
                shutdown_threads: Arc::new(Mutex::new(Vec::new())),
                drop_threads: Arc::new(Mutex::new(Vec::new())),
                first_failure: Mutex::new(Some(first_failure_tx)),
                retry_gate: Arc::clone(&retry_gate),
                successful_reap: Mutex::new(None),
            },
            None,
        );
        let cleanup = CleanupDispatcher::start(Arc::clone(&ledger)).expect("cleanup dispatcher");
        let mut active = ActiveOwnership {
            ownership: Some(ownership),
            cleanup,
        };
        let terminal =
            force_pre_protocol_with_ledger(&ledger, &mut active, SketchWorkerFailure::Protocol);
        assert_eq!(
            terminal,
            SketchWorkerTerminal::Failure(SketchWorkerFailure::ContainmentCleanup)
        );
        assert!(active.ownership.is_none());
        assert!(first_failure_rx.recv_timeout(TEST_WAIT).is_ok());
        release_retry(&retry_gate);
    }
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
    fn worker_protocol_module_ceiling_is_checked_without_allocating_a_fixture() {
        let cap: usize = worker_protocol::WORKER_PROTOCOL_MAX_MODULE_BYTES
            .try_into()
            .expect("host usize");
        assert_eq!(validate_worker_module_len(cap), Ok(()));
        assert_eq!(
            validate_worker_module_len(cap + 1),
            Err(SketchWorkerFailure::InvalidConfiguration)
        );
    }
    #[test]
    fn early_terminal_gate_requires_upload_success_and_is_bounded() {
        let mut gate = UploadTerminalGate::default();
        assert_eq!(gate.terminal(), Ok(false));
        assert_eq!(gate.upload(false), Err(SketchWorkerFailure::Protocol));
        let mut gate = UploadTerminalGate::default();
        assert_eq!(gate.terminal(), Ok(false));
        assert_eq!(gate.upload(true), Ok(true));
        assert_eq!(gate.terminal(), Ok(true));
        let mut gate = UploadTerminalGate::default();
        assert_eq!(gate.terminal(), Ok(false));
        assert_eq!(gate.terminal(), Err(SketchWorkerFailure::Protocol));
    }
    #[test]
    fn cancellation_wins_a_same_tick_deadline() {
        assert_eq!(
            select_stop(true, true),
            Some(SketchWorkerStopReason::Cancelled)
        );
        assert_eq!(
            select_stop(false, true),
            Some(SketchWorkerStopReason::DeadlineExceeded)
        );
        assert_eq!(select_stop(false, false), None);
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
                protocol_failures: 1,
                live_workers: 0,
                live_protocol_tasks: 0,
                pending_root_leases: 0,
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
