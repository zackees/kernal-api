//! Bounded admission of versioned core-Wasm sketches.
//!
//! Admission performs one private Cranelift compilation after the complete
//! binary contract has succeeded. The threaded-root profile can then start in
//! a fresh private store; no Wasmtime handle appears in the public API.

use std::cell::UnsafeCell;
use std::fmt;
use std::mem::align_of;
use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;

use wasmparser::{CompositeInnerType, ExternalKind, Parser, Payload, TypeRef, ValType};
use wasmtime::{
    Caller, Config, Engine, InstancePre, Linker, MemoryType, Module, SharedMemory, Store, Strategy,
};

const PAGE_BYTES: u64 = 64 * 1024;
const ABI_MODULE: &str = "kernal-api:v1";
const ABI_YIELD: &str = "kernel-yield";
const THREAD_MODULE: &str = "wasi";
const THREAD_SPAWN: &str = "thread-spawn";
const MEMORY_MODULE: &str = "env";
const MEMORY_NAME: &str = "memory";
const ENTRY: &str = "kernal-api-run";
const ABI_METADATA: &str = "kernal-api.abi";
const ABI_METADATA_VALUE: &[u8] = b"v1";
const PROFILE_METADATA: &str = "kernal-api.profile";
const PROFILE_METADATA_VALUE: &[u8] = b"threaded-core-wasm-v1";
const VALIDATION_PROFILE_METADATA_VALUE: &[u8] = b"threaded-core-wasm-validation-v1";
const VALIDATION_REPORT: &str = "kernal-api-threaded-validation-report-v1";
const MAX_METADATA_BYTES: usize = 128;
const THREADED_RUST_INITIAL_PAGES: u32 = 17;
const THREADED_RUST_MAX_PAGES: u32 = 16_384;
const THREADED_RUST_RESERVATION_BYTES: u64 = (THREADED_RUST_MAX_PAGES as u64) * PAGE_BYTES;
const ERRNO_SUCCESS: i32 = 0;
const ERRNO_FAULT: i32 = 21;
const THREAD_SPAWN_REJECTED: i32 = -1;
const MAX_P1_IOVECS: usize = 1024;
/// Absolute v1 ceiling. This bounds native JoinHandles and the facade-owned
/// child-outcome vector independently of a caller's requested quota.
const MAX_GUEST_THREADS_V1: usize = 16;
const DEFAULT_MAX_GUEST_THREADS: usize = MAX_GUEST_THREADS_V1;

/// Semantic admission contracts. The default remains the narrow synthetic
/// profile; Rust's standard threaded output is opt-in and versioned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchAdmissionProfile {
    SyntheticV1,
    ThreadedRustV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchCompilerConfig {
    max_wasm_stack_bytes: usize,
    execution_limits: SketchExecutionLimits,
}
impl SketchCompilerConfig {
    pub fn new(max_wasm_stack_bytes: usize) -> Result<Self, SketchCompilerError> {
        if max_wasm_stack_bytes == 0 {
            return Err(SketchCompilerError::InvalidStackLimit);
        }
        Ok(Self {
            max_wasm_stack_bytes,
            execution_limits: SketchExecutionLimits::default(),
        })
    }
    pub fn max_wasm_stack_bytes(self) -> usize {
        self.max_wasm_stack_bytes
    }
    /// Sets facade-owned aggregate limits for logical sketch sessions.
    pub fn with_execution_limits(
        mut self,
        limits: SketchExecutionLimits,
    ) -> Result<Self, SketchCompilerError> {
        if !limits.is_valid() {
            return Err(SketchCompilerError::InvalidExecutionLimits);
        }
        self.execution_limits = limits;
        Ok(self)
    }
    pub fn execution_limits(self) -> SketchExecutionLimits {
        self.execution_limits
    }
}
impl Default for SketchCompilerConfig {
    fn default() -> Self {
        Self {
            max_wasm_stack_bytes: 2 * 1024 * 1024,
            execution_limits: SketchExecutionLimits::default(),
        }
    }
}

/// Facade-owned aggregate bounds for one compiler's logical sketch sessions.
/// These are reservations, not Wasmtime Store or pooling limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchExecutionLimits {
    maximum_reserved_shared_memory_bytes: u64,
    maximum_active_root_executions: usize,
}
impl SketchExecutionLimits {
    pub fn new(
        maximum_reserved_shared_memory_bytes: u64,
        maximum_active_root_executions: usize,
    ) -> Result<Self, SketchCompilerError> {
        let limits = Self {
            maximum_reserved_shared_memory_bytes,
            maximum_active_root_executions,
        };
        limits
            .is_valid()
            .then_some(limits)
            .ok_or(SketchCompilerError::InvalidExecutionLimits)
    }
    pub fn maximum_reserved_shared_memory_bytes(self) -> u64 {
        self.maximum_reserved_shared_memory_bytes
    }
    pub fn maximum_active_root_executions(self) -> usize {
        self.maximum_active_root_executions
    }
    fn is_valid(self) -> bool {
        self.maximum_reserved_shared_memory_bytes >= THREADED_RUST_RESERVATION_BYTES
            && self.maximum_active_root_executions != 0
    }
}
impl Default for SketchExecutionLimits {
    fn default() -> Self {
        Self {
            // One exact Rust 1.95 threaded profile. Callers that want more
            // concurrent logical sketches must explicitly reserve more.
            maximum_reserved_shared_memory_bytes: THREADED_RUST_RESERVATION_BYTES,
            maximum_active_root_executions: 1,
        }
    }
}

/// Bounded semantic observations for compiler-owned logical sketch sessions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SketchExecutionSnapshot {
    reserved_shared_memory_bytes: u64,
    active_root_executions: usize,
    live_guest_threads: usize,
    live_stores: usize,
    live_instances: usize,
}
impl SketchExecutionSnapshot {
    pub fn reserved_shared_memory_bytes(self) -> u64 {
        self.reserved_shared_memory_bytes
    }
    pub fn active_root_executions(self) -> usize {
        self.active_root_executions
    }
    pub fn live_guest_threads(self) -> usize {
        self.live_guest_threads
    }
    pub fn live_stores(self) -> usize {
        self.live_stores
    }
    pub fn live_instances(self) -> usize {
        self.live_instances
    }
}

/// Facade-owned compiler for the selected core-Wasm profile.
#[derive(Clone)]
pub struct SketchCompiler {
    engine: Arc<Engine>,
    compilations: Arc<AtomicU64>,
    execution_ledger: Arc<ExecutionLedger>,
}
impl SketchCompiler {
    /// Creates a Cranelift, threads, and shared-memory profile. Pooling,
    /// Component Model, Winch, WASI linking, and an executor are absent.
    pub fn new(config: SketchCompilerConfig) -> Result<Self, SketchCompilerError> {
        let mut cfg = Config::new();
        cfg.strategy(Strategy::Cranelift);
        cfg.wasm_threads(true);
        cfg.shared_memory(true);
        cfg.wasm_memory64(false);
        cfg.wasm_multi_memory(false);
        cfg.wasm_shared_everything_threads(false);
        cfg.max_wasm_stack(config.max_wasm_stack_bytes);
        let engine = Engine::new(&cfg).map_err(|_| SketchCompilerError::Unavailable)?;
        Ok(Self {
            engine: Arc::new(engine),
            compilations: Arc::new(AtomicU64::new(0)),
            execution_ledger: Arc::new(ExecutionLedger::new(config.execution_limits)),
        })
    }
    /// Preflights and then compiles one module. Rejected input cannot compile.
    pub fn admit(
        &self,
        bytes: &[u8],
        policy: SketchModulePolicy,
    ) -> Result<AdmittedSketch, SketchModuleError> {
        if bytes.len() > policy.max_module_bytes {
            return Err(SketchModuleError::ModuleTooLarge {
                actual_bytes: bytes.len(),
                maximum_bytes: policy.max_module_bytes,
            });
        }
        let memory = match policy.profile {
            SketchAdmissionProfile::SyntheticV1 => preflight(bytes, policy)?,
            SketchAdmissionProfile::ThreadedRustV1 => {
                preflight_threaded_rust(bytes, policy, policy.validation)?
            }
        };
        let module =
            Module::new(&self.engine, bytes).map_err(|_| SketchModuleError::InvalidBinary)?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        Ok(AdmittedSketch {
            engine: Arc::clone(&self.engine),
            module,
            module_bytes: bytes.len(),
            shared_memory: memory,
            max_guest_threads: policy.max_guest_threads,
            profile: policy.profile,
            validation: policy.validation,
            execution_ledger: Arc::clone(&self.execution_ledger),
            prepared_root: std::sync::Mutex::new(None),
            #[cfg(test)]
            preparation_count: AtomicU64::new(0),
        })
    }
    /// Number of modules whose complete preflight reached private compilation.
    pub fn compiled_module_count(&self) -> u64 {
        self.compilations.load(Ordering::Relaxed)
    }
    /// Snapshot of aggregate logical resources owned by this compiler.
    pub fn execution_limits_snapshot(&self) -> SketchExecutionSnapshot {
        self.execution_ledger.snapshot()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchModulePolicy {
    max_module_bytes: usize,
    max_shared_memory_pages: u32,
    max_guest_threads: usize,
    profile: SketchAdmissionProfile,
    validation: bool,
}
impl SketchModulePolicy {
    pub fn new(
        max_module_bytes: usize,
        max_shared_memory_pages: u32,
    ) -> Result<Self, SketchModuleError> {
        if max_module_bytes == 0 {
            return Err(SketchModuleError::InvalidModuleLimit);
        }
        if max_shared_memory_pages == 0 {
            return Err(SketchModuleError::InvalidSharedMemoryLimit);
        }
        Ok(Self {
            max_module_bytes,
            max_shared_memory_pages,
            max_guest_threads: DEFAULT_MAX_GUEST_THREADS,
            profile: SketchAdmissionProfile::SyntheticV1,
            validation: false,
        })
    }
    /// Selects the exact Rust 1.95 `wasm32-wasip1-threads` link profile.
    /// `max_shared_memory_pages` is explicit so callers can decline its 1 GiB
    /// (16,384 page) link contract with the usual typed quota failure.
    pub fn threaded_rust_v1(
        max_module_bytes: usize,
        max_shared_memory_pages: u32,
    ) -> Result<Self, SketchModuleError> {
        let mut policy = Self::new(max_module_bytes, max_shared_memory_pages)?;
        policy.profile = SketchAdmissionProfile::ThreadedRustV1;
        Ok(policy)
    }
    /// Crate-only artifact characterization contract. Applications cannot opt
    /// into this wider validation export surface.
    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn threaded_rust_validation_v1_for_test(
        max_module_bytes: usize,
        max_shared_memory_pages: u32,
    ) -> Result<Self, SketchModuleError> {
        let mut policy = Self::threaded_rust_v1(max_module_bytes, max_shared_memory_pages)?;
        policy.validation = true;
        Ok(policy)
    }
    pub fn max_module_bytes(self) -> usize {
        self.max_module_bytes
    }
    pub fn max_shared_memory_pages(self) -> u32 {
        self.max_shared_memory_pages
    }
    /// Bounds live guest-owned native threads for one admitted sketch. The
    /// limit is a semantic controller quota, not a Wasmtime pooling setting.
    /// V1 accepts at most 16 threads so the native
    /// registrations and bounded semantic child report cannot grow unbounded.
    pub fn with_max_guest_threads(mut self, maximum: usize) -> Result<Self, SketchModuleError> {
        if maximum == 0 {
            return Err(SketchModuleError::InvalidThreadLimit);
        }
        if maximum > MAX_GUEST_THREADS_V1 {
            return Err(SketchModuleError::ThreadLimitExceedsV1Maximum {
                requested: maximum,
                maximum: MAX_GUEST_THREADS_V1,
            });
        }
        self.max_guest_threads = maximum;
        Ok(self)
    }
    pub fn max_guest_threads(self) -> usize {
        self.max_guest_threads
    }
    pub fn profile(self) -> SketchAdmissionProfile {
        self.profile
    }
}

/// An admitted module. The backend object is private to the crate.
pub struct AdmittedSketch {
    engine: Arc<Engine>,
    execution_ledger: Arc<ExecutionLedger>,
    module: Module,
    module_bytes: usize,
    shared_memory: SketchSharedMemory,
    max_guest_threads: usize,
    profile: SketchAdmissionProfile,
    validation: bool,
    prepared_root: std::sync::Mutex<Option<Arc<PreparedThreadedRoot>>>,
    // A unit-only observation belongs to the admitted sketch, not the cached
    // controller: it must survive any future controller replacement to prove
    // the cache did not prepare a second private prelink/shared memory.
    #[cfg(test)]
    preparation_count: AtomicU64,
}
impl AdmittedSketch {
    pub fn module_bytes(&self) -> usize {
        self.module_bytes
    }
    pub fn shared_memory(&self) -> SketchSharedMemory {
        self.shared_memory
    }
    /// Aggregate accounting owned by the compiler that admitted this sketch.
    pub fn execution_limits_snapshot(&self) -> SketchExecutionSnapshot {
        self.execution_ledger.snapshot()
    }
    /// Explicitly releases the cached logical session. Calling this while a
    /// root is executing is rejected; dropping an admitted sketch also
    /// releases its cached session once the last execution reference ends.
    pub fn close_threaded_root(&self) -> Result<(), SketchExecutionError> {
        let mut slot = self
            .prepared_root
            .lock()
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        if let Some(prepared) = slot.as_ref() {
            // This lock is shared with root-permit acquisition.  A close either
            // observes an active root and leaves the cached session untouched,
            // or marks the session closed before a racing execution can obtain
            // its permit.  Merely cloning the Arc is not an execution lease.
            let mut session = prepared
                .controller
                .session
                .lock()
                .map_err(|_| SketchExecutionError::PrelinkFailed)?;
            if session.active_roots != 0 {
                return Err(SketchExecutionError::SessionBusy);
            }
            session.closing = true;
        }
        drop(slot.take());
        Ok(())
    }
    /// Instantiates the admitted module in one fresh root store, then invokes
    /// the admitted command ABI's exported `_start` exactly once. The Wasm
    /// start section runs during instantiation; Rust command artifacts use it
    /// only for memory initialization, while `_start` performs constructors,
    /// user entry, and the P1 exit path.
    pub fn execute_threaded_root(
        &self,
        runtime: crate::async_engine::RuntimeHandle,
    ) -> Result<ThreadedRootOutcome, SketchExecutionError> {
        if self.profile != SketchAdmissionProfile::ThreadedRustV1 {
            return Err(SketchExecutionError::ThreadedProfileRequired);
        }
        let (prepared, _root) = self.prepare_threaded_root_with_permit()?;
        prepared
            .controller
            .runtime_identity
            .store(runtime.identity_for_wasm(), Ordering::Release);
        let _store_observation = CounterObservation::new(
            Arc::clone(&prepared.controller.execution_ledger),
            LedgerCounter::Stores,
        );
        let mut store = Store::new(
            &self.engine,
            ThreadStoreState {
                controller: Arc::clone(&prepared.controller),
                runtime: Some(runtime),
            },
        );
        // Do not return from this scope before finalization. A Wasm start
        // section can invoke thread-spawn while instantiation is still in
        // progress, and lookup/call failures after that must still close and
        // drain every accepted native child.
        let mut validation_getter = None;
        let mut _instance_observation = None;
        let outcome = (|| {
            let instance = match prepared.prelink.instantiate(&mut store) {
                Ok(instance) => instance,
                Err(error) => return map_root_error(&error),
            };
            _instance_observation = Some(CounterObservation::new(
                Arc::clone(&prepared.controller.execution_ledger),
                LedgerCounter::Instances,
            ));
            let start = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .map_err(|_| SketchExecutionError::Trapped)?;
            if self.validation {
                validation_getter = Some(
                    instance
                        .get_typed_func::<(), i32>(&mut store, VALIDATION_REPORT)
                        .map_err(|_| SketchExecutionError::ValidationReportInvalid)?,
                );
            }
            start
                .call(&mut store, ())
                .map(|_| ThreadedRootOutcome::Started)
                .or_else(|error| map_root_error(&error))
        })();
        // Joining happens after the root Store has returned from Wasm and the
        // workers mutex is not held. A child failure is part of the semantic
        // execution result rather than a detached native-thread panic.
        let children = prepared.controller.join_completed();
        let rejections = prepared.controller.take_thread_spawn_rejections();
        // Finalization always drains children, but the root call is the
        // primary operation: its typed failure must not be masked by a
        // concurrent child or report diagnostic.
        let report = if self.validation && outcome.is_ok() && children.is_ok() {
            let getter = validation_getter.ok_or(SketchExecutionError::ValidationReportInvalid)?;
            let offset = getter
                .call(&mut store, ())
                .map_err(|_| SketchExecutionError::ValidationReportInvalid)?;
            validate_report(&prepared.controller.memory, offset)
        } else {
            Ok(())
        };
        resolve_threaded_result(outcome, children, report, rejections)
    }
    fn prepare_threaded_root_with_permit(
        &self,
    ) -> Result<(Arc<PreparedThreadedRoot>, LogicalRootPermit), SketchExecutionError> {
        let mut prepared = self
            .prepared_root
            .lock()
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        if let Some(prepared) = prepared.as_ref() {
            let prepared = Arc::clone(prepared);
            let permit = prepared.controller.acquire_root()?;
            return Ok((prepared, permit));
        }
        // The Rust threaded link profile has a fixed maximum. Reserve its
        // complete address-space contract before asking Wasmtime to create the
        // engine-owned shared memory; committed pages are telemetry only.
        let reservation = self
            .execution_ledger
            .reserve_shared_memory(THREADED_RUST_RESERVATION_BYTES)?;
        let memory = SharedMemory::new(
            &self.engine,
            MemoryType::shared(
                self.shared_memory.minimum_pages,
                self.shared_memory.maximum_pages,
            ),
        )
        .map_err(|_| SketchExecutionError::SharedMemoryUnavailable)?;
        let controller = Arc::new(ThreadController {
            engine: Arc::clone(&self.engine),
            memory,
            prelink: OnceLock::new(),
            workers: Mutex::new(Workers {
                next_tid: 1,
                accepted: 0,
                live: 0,
                closing: false,
                maximum: self.max_guest_threads,
                capacity_rejections: 0,
                closing_rejections: 0,
                handles: Vec::new(),
                // At most `maximum` spawns are accepted before the controller
                // closes, so this facade report remains absolutely bounded.
                outcomes: Vec::with_capacity(self.max_guest_threads),
            }),
            kernel_yield_count: AtomicU64::new(0),
            runtime_handle_count: AtomicU64::new(0),
            runtime_identity: AtomicUsize::new(0),
            runtime_identity_mismatches: AtomicU64::new(0),
            execution_ledger: Arc::clone(&self.execution_ledger),
            session: Mutex::new(SessionState::default()),
        });
        let mut linker = Linker::new(&self.engine);
        define_closed_imports(&mut linker)?;

        // SharedMemory is engine-owned, so this bootstrap store is only used to
        // register the import and never owns an instance or crosses a thread.
        let bootstrap = Store::new(
            &self.engine,
            ThreadStoreState {
                controller: Arc::clone(&controller),
                runtime: None,
            },
        );
        linker
            .define(
                &bootstrap,
                MEMORY_MODULE,
                MEMORY_NAME,
                controller.memory.clone(),
            )
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        let prelink = Arc::new(
            linker
                .instantiate_pre(&self.module)
                .map_err(|_| SketchExecutionError::PrelinkFailed)?,
        );
        controller
            .prelink
            .set(Arc::clone(&prelink))
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        let prelink = Arc::clone(
            controller
                .prelink
                .get()
                .ok_or(SketchExecutionError::PrelinkFailed)?,
        );

        let prepared_root = Arc::new(PreparedThreadedRoot {
            controller,
            prelink,
            _reservation: reservation,
        });
        // Do not cache a newly prepared session until it also has a root
        // permit. A concurrent compiler-wide root cap rejection must roll back
        // this new reservation by ordinary RAII rather than parking 1 GiB in
        // an unusable cache entry.
        let permit = prepared_root.controller.acquire_root()?;
        #[cfg(test)]
        self.preparation_count.fetch_add(1, Ordering::Relaxed);
        *prepared = Some(Arc::clone(&prepared_root));
        Ok((prepared_root, permit))
    }
    #[cfg(test)]
    fn root_execution_observation_for_test(&self) -> Option<RootExecutionObservation> {
        let prepared = self.prepared_root.lock().ok()?.as_ref()?.clone();
        let controller = &prepared.controller;
        let workers = controller.workers.lock().ok()?;
        Some(RootExecutionObservation {
            preparations: self.preparation_count.load(Ordering::Relaxed),
            kernel_yields: controller.kernel_yield_count.load(Ordering::Relaxed),
            supplied_runtime_handles: controller.runtime_handle_count.load(Ordering::Relaxed),
            runtime_identity_mismatches: controller
                .runtime_identity_mismatches
                .load(Ordering::Relaxed),
            accepted_child_registrations: workers.accepted,
            live_threads: workers.live,
            queued_join_handles: workers.handles.len(),
        })
    }
    #[allow(dead_code)]
    pub(crate) fn compiled_module(&self) -> &Module {
        &self.module
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchSharedMemory {
    minimum_pages: u32,
    maximum_pages: u32,
}
impl SketchSharedMemory {
    pub fn minimum_pages(self) -> u32 {
        self.minimum_pages
    }
    pub fn maximum_pages(self) -> u32 {
        self.maximum_pages
    }
    pub fn maximum_bytes(self) -> u64 {
        u64::from(self.maximum_pages) * PAGE_BYTES
    }
}

/// Result of executing the module's root start function.
///
/// This is intentionally semantic: backend stores, instances, and shared
/// memories stay private to the sketch host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadedRootOutcome {
    /// The module start completed without requesting process exit.
    Started,
    /// The root completed, but bounded guest thread-spawn requests were
    /// rejected. The guest ABI receives `-1`; this carries the typed host
    /// accounting without exposing a backend error.
    StartedWithThreadRejections(ThreadSpawnRejectionSummary),
    /// The module requested the normal `proc_exit(0)` completion path.
    Exited,
    /// The root requested normal process exit after bounded thread rejections.
    ExitedWithThreadRejections(ThreadSpawnRejectionSummary),
}

/// Bounded, facade-owned accounting for guest `wasi::thread-spawn` rejections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ThreadSpawnRejectionSummary {
    capacity: u32,
    closing: u32,
}
impl ThreadSpawnRejectionSummary {
    pub fn capacity(self) -> u32 {
        self.capacity
    }
    pub fn closing(self) -> u32 {
        self.closing
    }
    pub fn is_empty(self) -> bool {
        self.capacity == 0 && self.closing == 0
    }
}

/// Bounded failures from private threaded-root setup and start execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchExecutionError {
    ThreadedProfileRequired,
    SharedMemoryLimitExceeded,
    RootExecutionLimitExceeded,
    SessionBusy,
    SharedMemoryUnavailable,
    PrelinkFailed,
    NonzeroExit { code: i32 },
    ChildNonzeroExit { code: i32 },
    ChildTrapped,
    ChildPanicked,
    ChildOutcomes { outcomes: Vec<ThreadedChildOutcome> },
    ValidationReportInvalid,
    Trapped,
}
impl SketchExecutionError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ThreadedProfileRequired => "threaded-profile-required",
            Self::SharedMemoryLimitExceeded => "shared-memory-limit-exceeded",
            Self::RootExecutionLimitExceeded => "root-execution-limit-exceeded",
            Self::SessionBusy => "session-busy",
            Self::SharedMemoryUnavailable => "shared-memory-unavailable",
            Self::PrelinkFailed => "prelink-failed",
            Self::NonzeroExit { .. } => "nonzero-exit",
            Self::ChildNonzeroExit { .. } => "child-nonzero-exit",
            Self::ChildTrapped => "child-trapped",
            Self::ChildPanicked => "child-panicked",
            Self::ChildOutcomes { .. } => "child-outcomes",
            Self::ValidationReportInvalid => "validation-report-invalid",
            Self::Trapped => "trapped",
        }
    }
}
impl fmt::Display for SketchExecutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "threaded sketch execution failed ({})", self.code())
    }
}
impl std::error::Error for SketchExecutionError {}

/// Private compiler-owned accounting. It deliberately has no relationship to
/// a Store limiter: shared memory belongs to the Engine and is visible to all
/// per-thread stores in a logical sketch session.
struct ExecutionLedger {
    limits: SketchExecutionLimits,
    state: Mutex<ExecutionLedgerState>,
}

#[derive(Default)]
struct ExecutionLedgerState {
    reserved_shared_memory_bytes: u64,
    active_root_executions: usize,
    live_guest_threads: usize,
    live_stores: usize,
    live_instances: usize,
}
impl ExecutionLedger {
    fn new(limits: SketchExecutionLimits) -> Self {
        Self {
            limits,
            state: Mutex::new(ExecutionLedgerState::default()),
        }
    }
    fn snapshot(&self) -> SketchExecutionSnapshot {
        let state = self.state.lock().expect("execution ledger mutex poisoned");
        SketchExecutionSnapshot {
            reserved_shared_memory_bytes: state.reserved_shared_memory_bytes,
            active_root_executions: state.active_root_executions,
            live_guest_threads: state.live_guest_threads,
            live_stores: state.live_stores,
            live_instances: state.live_instances,
        }
    }
    fn reserve_shared_memory(
        self: &Arc<Self>,
        bytes: u64,
    ) -> Result<SharedMemoryReservation, SketchExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        let Some(next) = state.reserved_shared_memory_bytes.checked_add(bytes) else {
            return Err(SketchExecutionError::SharedMemoryLimitExceeded);
        };
        if next > self.limits.maximum_reserved_shared_memory_bytes {
            return Err(SketchExecutionError::SharedMemoryLimitExceeded);
        }
        state.reserved_shared_memory_bytes = next;
        Ok(SharedMemoryReservation {
            ledger: Arc::clone(self),
            bytes,
        })
    }
    fn acquire_root(self: &Arc<Self>) -> Result<RootExecutionPermit, SketchExecutionError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        if state.active_root_executions >= self.limits.maximum_active_root_executions {
            return Err(SketchExecutionError::RootExecutionLimitExceeded);
        }
        state.active_root_executions += 1;
        Ok(RootExecutionPermit {
            ledger: Arc::clone(self),
        })
    }
    fn increment(&self, counter: LedgerCounter) {
        let mut state = self.state.lock().expect("execution ledger mutex poisoned");
        let value = counter.get_mut(&mut state);
        *value = value.saturating_add(1);
    }
    fn decrement(&self, counter: LedgerCounter) {
        let mut state = self.state.lock().expect("execution ledger mutex poisoned");
        let value = counter.get_mut(&mut state);
        debug_assert_ne!(*value, 0, "execution ledger counter underflow");
        *value = value.saturating_sub(1);
    }
}

#[derive(Clone, Copy)]
enum LedgerCounter {
    GuestThreads,
    Stores,
    Instances,
}
impl LedgerCounter {
    fn get_mut(self, state: &mut ExecutionLedgerState) -> &mut usize {
        match self {
            Self::GuestThreads => &mut state.live_guest_threads,
            Self::Stores => &mut state.live_stores,
            Self::Instances => &mut state.live_instances,
        }
    }
}

struct SharedMemoryReservation {
    ledger: Arc<ExecutionLedger>,
    bytes: u64,
}
impl Drop for SharedMemoryReservation {
    fn drop(&mut self) {
        let mut state = self
            .ledger
            .state
            .lock()
            .expect("execution ledger mutex poisoned");
        debug_assert!(
            state.reserved_shared_memory_bytes >= self.bytes,
            "shared-memory reservation underflow"
        );
        state.reserved_shared_memory_bytes = state
            .reserved_shared_memory_bytes
            .saturating_sub(self.bytes);
    }
}

struct RootExecutionPermit {
    ledger: Arc<ExecutionLedger>,
}

struct CounterObservation {
    ledger: Arc<ExecutionLedger>,
    counter: LedgerCounter,
}
impl CounterObservation {
    fn new(ledger: Arc<ExecutionLedger>, counter: LedgerCounter) -> Self {
        ledger.increment(counter);
        Self { ledger, counter }
    }
}
impl Drop for CounterObservation {
    fn drop(&mut self) {
        self.ledger.decrement(self.counter);
    }
}
impl Drop for RootExecutionPermit {
    fn drop(&mut self) {
        let mut state = self
            .ledger
            .state
            .lock()
            .expect("execution ledger mutex poisoned");
        debug_assert_ne!(state.active_root_executions, 0, "root permit underflow");
        state.active_root_executions = state.active_root_executions.saturating_sub(1);
    }
}

#[cfg(test)]
mod execution_ledger_tests {
    use super::*;

    #[test]
    fn execution_limits_reject_an_unreservable_profile_and_zero_root_permits() {
        assert_eq!(
            SketchExecutionLimits::new(THREADED_RUST_RESERVATION_BYTES - 1, 1),
            Err(SketchCompilerError::InvalidExecutionLimits)
        );
        assert_eq!(
            SketchExecutionLimits::new(THREADED_RUST_RESERVATION_BYTES, 0),
            Err(SketchCompilerError::InvalidExecutionLimits)
        );
    }

    #[test]
    fn reservation_is_exact_bounded_and_released_on_drop() {
        let ledger = Arc::new(ExecutionLedger::new(
            SketchExecutionLimits::new(THREADED_RUST_RESERVATION_BYTES, 1).expect("limits"),
        ));
        let reservation = ledger
            .reserve_shared_memory(THREADED_RUST_RESERVATION_BYTES)
            .expect("exact reservation");
        assert_eq!(
            ledger.snapshot().reserved_shared_memory_bytes(),
            THREADED_RUST_RESERVATION_BYTES
        );
        assert!(matches!(
            ledger.reserve_shared_memory(1),
            Err(SketchExecutionError::SharedMemoryLimitExceeded)
        ));
        drop(reservation);
        assert_eq!(ledger.snapshot(), SketchExecutionSnapshot::default());
    }

    #[test]
    fn root_permit_and_all_counter_observations_balance_on_unwind() {
        let ledger = Arc::new(ExecutionLedger::new(SketchExecutionLimits::default()));
        let result = std::panic::catch_unwind({
            let ledger = Arc::clone(&ledger);
            move || {
                let _root = ledger.acquire_root().expect("root permit");
                let _thread =
                    CounterObservation::new(Arc::clone(&ledger), LedgerCounter::GuestThreads);
                let _store = CounterObservation::new(Arc::clone(&ledger), LedgerCounter::Stores);
                let _instance = CounterObservation::new(ledger, LedgerCounter::Instances);
                panic!("simulated root failure");
            }
        });
        assert!(result.is_err());
        assert_eq!(ledger.snapshot(), SketchExecutionSnapshot::default());
    }

    #[test]
    fn concurrent_reservations_admit_exactly_one_default_threaded_session() {
        let ledger = Arc::new(ExecutionLedger::new(SketchExecutionLimits::default()));
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let mut workers = Vec::new();
        for _ in 0..2 {
            let ledger = Arc::clone(&ledger);
            let barrier = Arc::clone(&barrier);
            workers.push(std::thread::spawn(move || {
                barrier.wait();
                ledger.reserve_shared_memory(THREADED_RUST_RESERVATION_BYTES)
            }));
        }
        barrier.wait();
        let mut reservations = Vec::new();
        let mut rejected = 0;
        for worker in workers {
            match worker.join().expect("worker") {
                Ok(reservation) => reservations.push(reservation),
                Err(SketchExecutionError::SharedMemoryLimitExceeded) => rejected += 1,
                Err(error) => panic!("unexpected reservation error: {}", error.code()),
            }
        }
        assert_eq!(reservations.len(), 1);
        assert_eq!(rejected, 1);
        drop(reservations);
        assert_eq!(ledger.snapshot(), SketchExecutionSnapshot::default());
    }
}

struct ThreadController {
    engine: Arc<Engine>,
    memory: SharedMemory,
    // Callbacks reach their controller through ThreadStoreState. Keeping the
    // prelink here breaks their otherwise cyclic construction without letting a
    // Store or Instance escape to another thread.
    prelink: OnceLock<Arc<InstancePre<ThreadStoreState>>>,
    kernel_yield_count: AtomicU64,
    runtime_handle_count: AtomicU64,
    runtime_identity: AtomicUsize,
    runtime_identity_mismatches: AtomicU64,
    execution_ledger: Arc<ExecutionLedger>,
    session: Mutex<SessionState>,
    workers: Mutex<Workers>,
}

#[derive(Default)]
struct SessionState {
    active_roots: usize,
    closing: bool,
}
struct Workers {
    next_tid: i32,
    // Total child registrations in the current root execution. Unlike `live`,
    // this never decreases until drain, so a fast-completing guest cannot grow
    // the bounded facade outcome vector by repeatedly spawning replacements.
    accepted: usize,
    live: usize,
    closing: bool,
    maximum: usize,
    capacity_rejections: u32,
    closing_rejections: u32,
    handles: Vec<JoinHandle<()>>,
    // Completion is inherently concurrent; retain the guest TID so reporting
    // is deterministic rather than depending on native scheduler order.
    outcomes: Vec<(i32, ChildOutcome)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ChildOutcome {
    Completed,
    Exited,
    NonzeroExit(i32),
    Trapped,
    Panicked,
}

/// One cooperatively joined guest child outcome. This facade type contains no
/// native thread, Store, trap, or backend diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ThreadedChildOutcome {
    pub tid: i32,
    pub kind: ThreadedChildOutcomeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThreadedChildOutcomeKind {
    Completed,
    Exited,
    NonzeroExit { code: i32 },
    Trapped,
    Panicked,
}
impl ThreadController {
    fn acquire_root(self: &Arc<Self>) -> Result<LogicalRootPermit, SketchExecutionError> {
        let mut session = self
            .session
            .lock()
            .map_err(|_| SketchExecutionError::PrelinkFailed)?;
        if session.closing {
            return Err(SketchExecutionError::SessionBusy);
        }
        let permit = self.execution_ledger.acquire_root()?;
        session.active_roots += 1;
        Ok(LogicalRootPermit {
            controller: Arc::clone(self),
            _permit: permit,
        })
    }
    fn join_completed(&self) -> Result<(), SketchExecutionError> {
        // Mark closing before taking the queue. This prevents a child that is
        // still running from registering another native worker after the final
        // drain begins; no mutex remains held across `join` or guest Wasm.
        loop {
            let handles = self
                .workers
                .lock()
                .map_err(|_| SketchExecutionError::ChildPanicked)
                .map(|mut w| {
                    w.closing = true;
                    std::mem::take(&mut w.handles)
                })?;
            if handles.is_empty() {
                break;
            }
            for handle in handles {
                if handle.join().is_err() {
                    let mut workers = self
                        .workers
                        .lock()
                        .map_err(|_| SketchExecutionError::ChildPanicked)?;
                    if workers.outcomes.len() < workers.maximum {
                        workers.outcomes.push((i32::MAX, ChildOutcome::Panicked));
                    }
                }
            }
        }
        let mut workers = self
            .workers
            .lock()
            .map_err(|_| SketchExecutionError::ChildPanicked)?;
        let mut outcomes: Vec<_> = workers.outcomes.drain(..).collect();
        workers.accepted = 0;
        outcomes.sort_by_key(|(tid, _)| *tid);
        let report: Vec<_> = outcomes
            .into_iter()
            .map(|(tid, outcome)| ThreadedChildOutcome {
                tid,
                kind: match outcome {
                    ChildOutcome::Completed => ThreadedChildOutcomeKind::Completed,
                    ChildOutcome::Exited => ThreadedChildOutcomeKind::Exited,
                    ChildOutcome::NonzeroExit(code) => {
                        ThreadedChildOutcomeKind::NonzeroExit { code }
                    }
                    ChildOutcome::Trapped => ThreadedChildOutcomeKind::Trapped,
                    ChildOutcome::Panicked => ThreadedChildOutcomeKind::Panicked,
                },
            })
            .collect();
        workers.closing = false;
        if report.iter().all(|outcome| {
            matches!(
                outcome.kind,
                ThreadedChildOutcomeKind::Completed | ThreadedChildOutcomeKind::Exited
            )
        }) {
            Ok(())
        } else {
            Err(SketchExecutionError::ChildOutcomes { outcomes: report })
        }
    }

    fn take_thread_spawn_rejections(&self) -> ThreadSpawnRejectionSummary {
        let Ok(mut workers) = self.workers.lock() else {
            return ThreadSpawnRejectionSummary::default();
        };
        let summary = ThreadSpawnRejectionSummary {
            capacity: workers.capacity_rejections,
            closing: workers.closing_rejections,
        };
        workers.capacity_rejections = 0;
        workers.closing_rejections = 0;
        summary
    }
}

struct PreparedThreadedRoot {
    controller: Arc<ThreadController>,
    prelink: Arc<InstancePre<ThreadStoreState>>,
    _reservation: SharedMemoryReservation,
}

struct LogicalRootPermit {
    controller: Arc<ThreadController>,
    _permit: RootExecutionPermit,
}
impl Drop for LogicalRootPermit {
    fn drop(&mut self) {
        let mut session = self
            .controller
            .session
            .lock()
            .expect("thread controller session mutex poisoned");
        debug_assert_ne!(session.active_roots, 0, "logical root permit underflow");
        session.active_roots = session.active_roots.saturating_sub(1);
    }
}

struct ThreadStoreState {
    controller: Arc<ThreadController>,
    runtime: Option<crate::async_engine::RuntimeHandle>,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RootExecutionObservation {
    preparations: u64,
    kernel_yields: u64,
    supplied_runtime_handles: u64,
    runtime_identity_mismatches: u64,
    accepted_child_registrations: usize,
    live_threads: usize,
    queued_join_handles: usize,
}

#[derive(Debug, thiserror::Error)]
#[error("private kernal-api proc_exit sentinel ({0})")]
struct ProcExitSentinel(i32);

fn define_closed_imports(
    linker: &mut Linker<ThreadStoreState>,
) -> Result<(), SketchExecutionError> {
    linker
        .func_wrap(
            ABI_MODULE,
            ABI_YIELD,
            |caller: Caller<'_, ThreadStoreState>| {
                // The facade handle is deliberately supplied by the caller. This
                // slice has no scheduler yet, but must never construct a runtime.
                let controller = &caller.data().controller;
                controller
                    .kernel_yield_count
                    .fetch_add(1, Ordering::Relaxed);
                if caller.data().runtime.is_some() {
                    controller
                        .runtime_handle_count
                        .fetch_add(1, Ordering::Relaxed);
                    let actual = caller
                        .data()
                        .runtime
                        .as_ref()
                        .expect("checked runtime")
                        .identity_for_wasm();
                    if controller.runtime_identity.load(Ordering::Acquire) != actual {
                        controller
                            .runtime_identity_mismatches
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                let _ = controller.prelink.get();
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            THREAD_MODULE,
            THREAD_SPAWN,
            |caller: Caller<'_, ThreadStoreState>, arg: i32| -> i32 {
                let controller = Arc::clone(&caller.data().controller);
                let runtime = caller.data().runtime.clone();
                let tid = match controller.workers.lock() {
                    Ok(mut w) => {
                        if w.closing {
                            w.closing_rejections = w.closing_rejections.saturating_add(1);
                            return THREAD_SPAWN_REJECTED;
                        }
                        if w.live >= w.maximum
                            || w.accepted >= w.maximum
                            || w.next_tid > 0x1fff_ffff
                        {
                            w.capacity_rejections = w.capacity_rejections.saturating_add(1);
                            return THREAD_SPAWN_REJECTED;
                        }
                        let tid = w.next_tid;
                        w.next_tid += 1;
                        w.accepted += 1;
                        w.live += 1;
                        tid
                    }
                    _ => return THREAD_SPAWN_REJECTED,
                };
                let child = Arc::clone(&controller);
                let spawned = std::thread::Builder::new().spawn(move || {
                    let _thread_observation = CounterObservation::new(
                        Arc::clone(&child.execution_ledger),
                        LedgerCounter::GuestThreads,
                    );
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        let _store_observation = CounterObservation::new(
                            Arc::clone(&child.execution_ledger),
                            LedgerCounter::Stores,
                        );
                        let mut store = Store::new(
                            &child.engine,
                            ThreadStoreState {
                                controller: Arc::clone(&child),
                                runtime,
                            },
                        );
                        let Some(prelink) = child.prelink.get() else {
                            return ChildOutcome::Trapped;
                        };
                        let instance = match prelink.instantiate(&mut store) {
                            Ok(instance) => instance,
                            Err(error) => return map_child_error(&error),
                        };
                        let _instance_observation = CounterObservation::new(
                            Arc::clone(&child.execution_ledger),
                            LedgerCounter::Instances,
                        );
                        let entry = instance
                            .get_typed_func::<(i32, i32), ()>(&mut store, "wasi_thread_start")
                            .map_err(|_| ())
                            .ok();
                        let Some(entry) = entry else {
                            return ChildOutcome::Trapped;
                        };
                        match entry.call(&mut store, (tid, arg)) {
                            Ok(()) => ChildOutcome::Completed,
                            Err(error) => map_child_error(&error),
                        }
                    }));
                    let outcome = match result {
                        Ok(outcome) => outcome,
                        Err(_) => ChildOutcome::Panicked,
                    };
                    if let Ok(mut w) = child.workers.lock() {
                        w.live = w.live.saturating_sub(1);
                        if w.outcomes.len() < w.maximum {
                            w.outcomes.push((tid, outcome));
                        }
                    }
                });
                match spawned {
                    Ok(handle) => {
                        if let Ok(mut w) = controller.workers.lock() {
                            w.handles.push(handle);
                            tid
                        } else {
                            // Do not detach a worker if bookkeeping became
                            // poisoned after reservation. We cannot safely
                            // expose its positive TID, so wait before failing.
                            let _ = handle.join();
                            THREAD_SPAWN_REJECTED
                        }
                    }
                    Err(_) => {
                        if let Ok(mut w) = controller.workers.lock() {
                            w.live = w.live.saturating_sub(1);
                            w.accepted = w.accepted.saturating_sub(1);
                        }
                        THREAD_SPAWN_REJECTED
                    }
                }
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |caller: Caller<'_, ThreadStoreState>, _id: i32, _precision: i64, output: i32| -> i32 {
                write_shared(
                    &caller.data().controller.memory,
                    output,
                    &0_u64.to_le_bytes(),
                )
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_get",
            |caller: Caller<'_, ThreadStoreState>, _entries: i32, _buffer: i32| -> i32 {
                // Empty environment: no guest memory is dereferenced.
                let _ = &caller.data().controller.memory;
                ERRNO_SUCCESS
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |caller: Caller<'_, ThreadStoreState>, count: i32, bytes: i32| -> i32 {
                let memory = &caller.data().controller.memory;
                // Validate both output ranges before either write so a bad
                // second pointer never causes a partial guest-memory update.
                if shared_range(memory, count, 4).is_none()
                    || shared_range(memory, bytes, 4).is_none()
                {
                    return ERRNO_FAULT;
                }
                write_shared(memory, count, &0_u32.to_le_bytes());
                write_shared(memory, bytes, &0_u32.to_le_bytes())
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |caller: Caller<'_, ThreadStoreState>,
             _fd: i32,
             _iovecs: i32,
             _iovecs_len: i32,
             written: i32|
             -> i32 {
                // Output is intentionally discarded, but P1 still requires a
                // complete validation of the iovec table and payload ranges.
                let memory = &caller.data().controller.memory;
                let result = validate_iovecs(memory, _iovecs, _iovecs_len);
                if result != ERRNO_SUCCESS {
                    return result;
                }
                write_shared(memory, written, &0_u32.to_le_bytes())
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "proc_exit",
            |_caller: Caller<'_, ThreadStoreState>, _code: i32| -> wasmtime::Result<()> {
                Err(wasmtime::Error::new(ProcExitSentinel(_code)))
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "sched_yield",
            |_caller: Caller<'_, ThreadStoreState>| -> i32 { ERRNO_SUCCESS },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    Ok(())
}

#[cfg(test)]
mod threaded_root_observation_tests {
    use super::*;

    #[test]
    fn compiler_owned_ledger_reserves_the_exact_threaded_contract_and_releases_once() {
        let limits = SketchExecutionLimits::new(THREADED_RUST_RESERVATION_BYTES, 1)
            .expect("exact one-session limit");
        let compiler = SketchCompiler::new(
            SketchCompilerConfig::default()
                .with_execution_limits(limits)
                .expect("limits"),
        )
        .expect("compiler");
        let bytes = threaded_yield_fixture();
        let sketch = compiler
            .admit(
                &bytes,
                SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
                    .expect("policy"),
            )
            .expect("admission");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.run(async {
            assert_eq!(
                sketch.execute_threaded_root(runtime.handle()),
                Ok(ThreadedRootOutcome::Started)
            );
        });
        assert_eq!(
            compiler
                .execution_limits_snapshot()
                .reserved_shared_memory_bytes(),
            THREADED_RUST_RESERVATION_BYTES,
        );
        sketch.close_threaded_root().expect("explicit close");
        assert_eq!(
            compiler.execution_limits_snapshot(),
            SketchExecutionSnapshot::default()
        );
    }

    #[test]
    fn compiler_ledger_rejects_a_second_preparation_before_shared_memory_allocation() {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let bytes = threaded_yield_fixture();
        let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
            .expect("policy");
        let first = compiler.admit(&bytes, policy).expect("first admission");
        let second = compiler.admit(&bytes, policy).expect("second admission");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        runtime.run(async {
            first
                .execute_threaded_root(runtime.handle())
                .expect("first preparation");
            assert_eq!(
                second.execute_threaded_root(runtime.handle()),
                Err(SketchExecutionError::SharedMemoryLimitExceeded),
            );
        });
        first.close_threaded_root().expect("close first");
        runtime.run(async {
            second
                .execute_threaded_root(runtime.handle())
                .expect("released reservation admits second");
        });
    }

    #[test]
    fn close_and_root_permit_are_linearized_by_the_cached_session_lock() {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let bytes = threaded_yield_fixture();
        let sketch = compiler
            .admit(
                &bytes,
                SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
                    .expect("policy"),
            )
            .expect("admission");

        // The permit is acquired while `prepared_root` remains locked. A
        // concurrent close can therefore neither detach this session before
        // admission nor release its reservation underneath the root.
        let (prepared, permit) = sketch
            .prepare_threaded_root_with_permit()
            .expect("prepared root permit");
        assert_eq!(
            sketch.close_threaded_root(),
            Err(SketchExecutionError::SessionBusy)
        );
        assert_eq!(
            compiler
                .execution_limits_snapshot()
                .active_root_executions(),
            1
        );
        drop(permit);
        sketch
            .close_threaded_root()
            .expect("close after root drain");
        drop(prepared);
        assert_eq!(
            compiler.execution_limits_snapshot(),
            SketchExecutionSnapshot::default()
        );
    }

    #[test]
    fn barrier_race_between_close_and_execute_keeps_exactly_one_live_session() {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let bytes = threaded_yield_fixture();
        let sketch = compiler
            .admit(
                &bytes,
                SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
                    .expect("policy"),
            )
            .expect("admission");
        let (prepared, permit) = sketch
            .prepare_threaded_root_with_permit()
            .expect("initial preparation");
        drop(permit);
        drop(prepared);

        // The barrier makes both contenders start from the same cached
        // session. Either close wins and execution prepares one replacement,
        // or execution wins and close reports SessionBusy; neither path may
        // drop the reservation of an admitted root.
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let close_result = Mutex::new(None);
        std::thread::scope(|scope| {
            let close_barrier = Arc::clone(&barrier);
            let close_result = &close_result;
            let sketch = &sketch;
            scope.spawn(move || {
                close_barrier.wait();
                *close_result.lock().expect("result mutex") = Some(sketch.close_threaded_root());
            });
            barrier.wait();
            let (prepared, permit) = sketch
                .prepare_threaded_root_with_permit()
                .expect("racing root permit");
            drop(permit);
            drop(prepared);
        });
        assert!(matches!(
            close_result.lock().expect("result mutex").take(),
            Some(Ok(())) | Some(Err(SketchExecutionError::SessionBusy))
        ));
        assert_eq!(
            compiler
                .execution_limits_snapshot()
                .reserved_shared_memory_bytes(),
            THREADED_RUST_RESERVATION_BYTES
        );
        sketch.close_threaded_root().expect("final close");
        assert_eq!(
            compiler.execution_limits_snapshot(),
            SketchExecutionSnapshot::default()
        );
    }

    #[test]
    fn thread_policy_rejects_requests_above_the_v1_absolute_cap() {
        let policy =
            SketchModulePolicy::threaded_rust_v1(1, THREADED_RUST_MAX_PAGES).expect("policy");
        assert_eq!(
            policy.with_max_guest_threads(MAX_GUEST_THREADS_V1 + 1),
            Err(SketchModuleError::ThreadLimitExceedsV1Maximum {
                requested: MAX_GUEST_THREADS_V1 + 1,
                maximum: MAX_GUEST_THREADS_V1,
            })
        );
    }

    #[test]
    fn repeated_roots_reuse_one_preparation_and_receive_the_supplied_runtime() {
        let bytes = threaded_yield_fixture();
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let sketch = compiler
            .admit(
                &bytes,
                SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy"),
            )
            .expect("admission");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let handle = runtime.handle();
        runtime.run(async {
            assert_eq!(
                sketch.execute_threaded_root(handle.clone()),
                Ok(ThreadedRootOutcome::Started)
            );
            assert_eq!(
                sketch.execute_threaded_root(handle),
                Ok(ThreadedRootOutcome::Started)
            );
        });
        assert_eq!(compiler.compiled_module_count(), 1);
        assert_eq!(
            sketch.root_execution_observation_for_test(),
            Some(RootExecutionObservation {
                preparations: 1,
                kernel_yields: 2,
                supplied_runtime_handles: 2,
                runtime_identity_mismatches: 0,
                accepted_child_registrations: 0,
                live_threads: 0,
                queued_join_handles: 0,
            })
        );
    }

    #[test]
    fn cap_rejection_drains_private_registrations_and_is_reusable() {
        let bytes = threaded_cap_fixture();
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, THREADED_RUST_MAX_PAGES)
            .expect("policy")
            .with_max_guest_threads(1)
            .expect("cap");
        let sketch = compiler.admit(&bytes, policy).expect("admission");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        for expected_yields in [0, 0] {
            let outcome = runtime.run(async { sketch.execute_threaded_root(runtime.handle()) });
            assert!(matches!(
                outcome,
                Ok(ThreadedRootOutcome::StartedWithThreadRejections(summary))
                    if summary.capacity() == 1 && summary.closing() == 0
            ));
            let observation = sketch
                .root_execution_observation_for_test()
                .expect("prepared observation");
            assert_eq!(observation.kernel_yields, expected_yields);
            assert_eq!(observation.accepted_child_registrations, 0);
            assert_eq!(observation.live_threads, 0);
            assert_eq!(observation.queued_join_handles, 0);
        }
    }

    #[test]
    fn validation_profile_requires_its_metadata_and_rejects_precompile() {
        let bytes = threaded_yield_fixture();
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let policy = SketchModulePolicy::threaded_rust_validation_v1_for_test(
            bytes.len() + 1,
            THREADED_RUST_MAX_PAGES,
        )
        .expect("policy");
        let error = match compiler.admit(&bytes, policy) {
            Ok(_) => panic!("metadata is required"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            SketchModuleError::MissingMetadata {
                name: PROFILE_METADATA
            }
        );
        assert_eq!(compiler.compiled_module_count(), 0);
    }

    #[test]
    fn supplied_validation_artifact_executes_the_private_validation_lane() {
        let Ok(path) = std::env::var("KERNAL_API_THREADED_VALIDATION_WASM") else {
            // The real Rust guest is assembled only by the explicit diagnostic
            // workflow; normal unit runs remain source-only.
            return;
        };
        let bytes = std::fs::read(path).expect("read validation artifact");
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let sketch = compiler
            .admit(
                &bytes,
                SketchModulePolicy::threaded_rust_validation_v1_for_test(
                    bytes.len() + 1,
                    THREADED_RUST_MAX_PAGES,
                )
                .expect("policy"),
            )
            .expect("validation artifact admission");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let handle = runtime.handle();
        assert_eq!(
            runtime.run(async { sketch.execute_threaded_root(handle) }),
            Ok(ThreadedRootOutcome::Started)
        );
        assert_eq!(compiler.compiled_module_count(), 1);
        assert_eq!(
            sketch.root_execution_observation_for_test(),
            Some(RootExecutionObservation {
                preparations: 1,
                // One root `_start` call and the two Rust std-thread entries
                // each invoke the closed kernel-yield import.
                kernel_yields: 3,
                supplied_runtime_handles: 3,
                runtime_identity_mismatches: 0,
                accepted_child_registrations: 0,
                live_threads: 0,
                queued_join_handles: 0,
            })
        );
    }

    #[test]
    fn validation_profile_mutations_reject_before_compilation() {
        let policy = |bytes: usize| {
            SketchModulePolicy::threaded_rust_validation_v1_for_test(
                bytes + 1,
                THREADED_RUST_MAX_PAGES,
            )
            .expect("policy")
        };
        for (metadata, expected) in [
            (
                vec![
                    VALIDATION_PROFILE_METADATA_VALUE,
                    VALIDATION_PROFILE_METADATA_VALUE,
                ],
                SketchModuleError::DuplicateMetadata {
                    name: PROFILE_METADATA,
                },
            ),
            (
                vec![b"wrong-profile"],
                SketchModuleError::MetadataMismatch {
                    name: PROFILE_METADATA,
                },
            ),
        ] {
            let mut bytes = threaded_yield_fixture();
            for value in metadata {
                custom(PROFILE_METADATA, value, &mut bytes);
            }
            let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
            let error = match compiler.admit(&bytes, policy(bytes.len())) {
                Ok(_) => panic!("mutation must reject"),
                Err(error) => error,
            };
            assert_eq!(error, expected);
            assert_eq!(compiler.compiled_module_count(), 0);
        }

        let mut bytes = threaded_yield_fixture();
        custom(
            PROFILE_METADATA,
            VALIDATION_PROFILE_METADATA_VALUE,
            &mut bytes,
        );
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let error = match compiler.admit(&bytes, policy(bytes.len())) {
            Ok(_) => panic!("report export is required"),
            Err(error) => error,
        };
        assert!(matches!(error, SketchModuleError::ExportNotAllowed { .. }));
        assert_eq!(compiler.compiled_module_count(), 0);
    }

    fn validation_export_mutation(name: &str, kind: u8, index: u32) -> Vec<u8> {
        let bytes = threaded_yield_fixture();
        let mut section = 8;
        loop {
            if bytes[section] == 7 {
                break;
            }
            let mut at = section + 1;
            let mut length = 0_usize;
            let mut shift = 0;
            loop {
                let byte = bytes[at];
                at += 1;
                length |= usize::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            section = at + length;
        }
        let mut at = section + 1;
        let mut length = 0_usize;
        let mut shift = 0;
        loop {
            let byte = bytes[at];
            at += 1;
            length |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let body = &bytes[at..at + length];
        let mut replacement = vec![6];
        replacement.extend_from_slice(&body[1..]);
        text(name, &mut replacement);
        replacement.push(kind);
        leb(index, &mut replacement);
        let mut output = bytes[..section].to_vec();
        output.push(7);
        leb(replacement.len() as u32, &mut output);
        output.extend(replacement);
        output.extend_from_slice(&bytes[at + length..]);
        custom(
            PROFILE_METADATA,
            VALIDATION_PROFILE_METADATA_VALUE,
            &mut output,
        );
        output
    }

    #[test]
    fn validation_report_export_mutations_reject_precompile() {
        let cases = [
            ("wrong-name", 0, 9),
            (VALIDATION_REPORT, 2, 0),
            (VALIDATION_REPORT, 0, 8),
            ("extra", 0, 9),
        ];
        for (name, kind, index) in cases {
            let bytes = validation_export_mutation(name, kind, index);
            let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
            let policy = SketchModulePolicy::threaded_rust_validation_v1_for_test(
                bytes.len() + 1,
                THREADED_RUST_MAX_PAGES,
            )
            .expect("policy");
            assert!(compiler.admit(&bytes, policy).is_err());
            assert_eq!(compiler.compiled_module_count(), 0);
        }
    }

    // Keep import-section mutation local to the raw fixture. It proves the
    // validation profile remains closed before Wasmtime compilation rather
    // than relying on linker failure after an artifact is admitted.
    fn validation_fixture_with_extra_import() -> Vec<u8> {
        let bytes = threaded_yield_fixture();
        let mut section = 8;
        while bytes[section] != 2 {
            let mut at = section + 1;
            while bytes[at] & 0x80 != 0 {
                at += 1;
            }
            let length = usize::from(bytes[at] & 0x7f);
            section = at + 1 + length;
        }
        let mut body_at = section + 1;
        let mut length = 0_usize;
        let mut shift = 0;
        loop {
            let byte = bytes[body_at];
            body_at += 1;
            length |= usize::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let end = body_at + length;
        let mut body = bytes[body_at..end].to_vec();
        body[0] = 10; // the raw fixture has nine imports; add one.
        let mut extra = Vec::new();
        text("unexpected", &mut extra);
        text("import", &mut extra);
        extra.extend([0, 0]);
        body.extend(extra);
        let mut bytes = bytes[..section].to_vec();
        bytes.push(2);
        leb(body.len() as u32, &mut bytes);
        bytes.extend(body);
        bytes.extend_from_slice(&threaded_yield_fixture()[end..]);
        custom(
            PROFILE_METADATA,
            VALIDATION_PROFILE_METADATA_VALUE,
            &mut bytes,
        );
        bytes
    }

    #[test]
    fn validation_extra_import_rejects_before_compilation() {
        let bytes = validation_fixture_with_extra_import();
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
        let policy = SketchModulePolicy::threaded_rust_validation_v1_for_test(
            bytes.len() + 1,
            THREADED_RUST_MAX_PAGES,
        )
        .expect("policy");
        let error = match compiler.admit(&bytes, policy) {
            Ok(_) => panic!("extra import admitted"),
            Err(error) => error,
        };
        assert!(
            matches!(error, SketchModuleError::ForbiddenImport { .. }),
            "unexpected rejection: {error:?}"
        );
        assert_eq!(compiler.compiled_module_count(), 0);
    }

    fn leb(mut value: u32, output: &mut Vec<u8>) {
        loop {
            let mut byte = (value & 0x7f) as u8;
            value >>= 7;
            if value != 0 {
                byte |= 0x80;
            }
            output.push(byte);
            if value == 0 {
                return;
            }
        }
    }
    fn text(value: &str, output: &mut Vec<u8>) {
        leb(value.len() as u32, output);
        output.extend(value.as_bytes());
    }
    fn section(id: u8, body: Vec<u8>, output: &mut Vec<u8>) {
        output.push(id);
        leb(body.len() as u32, output);
        output.extend(body);
    }
    fn custom(name: &str, value: &[u8], output: &mut Vec<u8>) {
        let mut body = Vec::new();
        text(name, &mut body);
        body.extend(value);
        section(0, body, output);
    }

    // Minimal exact ThreadedRustV1 profile whose start calls kernel-yield.
    // Keeping it in this private unit module prevents test instrumentation
    // from becoming a public sketch-host API.
    fn threaded_yield_fixture() -> Vec<u8> {
        let mut wasm = b"\0asm\x01\0\0\0".to_vec();
        let mut types = Vec::new();
        leb(9, &mut types);
        types.extend([
            0x60, 0, 0, 0x60, 1, 0x7f, 1, 0x7f, 0x60, 3, 0x7f, 0x7e, 0x7f, 1, 0x7f, 0x60, 2, 0x7f,
            0x7f, 1, 0x7f, 0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f, 0x60, 1, 0x7f, 0, 0x60, 0, 1,
            0x7f, 0x60, 0, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 0,
        ]);
        section(1, types, &mut wasm);
        let mut imports = Vec::new();
        leb(9, &mut imports);
        text("env", &mut imports);
        text("memory", &mut imports);
        imports.extend([2, 3]);
        leb(17, &mut imports);
        leb(16_384, &mut imports);
        text(ABI_MODULE, &mut imports);
        text(ABI_YIELD, &mut imports);
        imports.extend([0, 0]);
        text(THREAD_MODULE, &mut imports);
        text(THREAD_SPAWN, &mut imports);
        imports.extend([0, 1]);
        for (name, ty) in [
            ("clock_time_get", 2),
            ("environ_get", 3),
            ("environ_sizes_get", 3),
            ("fd_write", 4),
            ("proc_exit", 5),
            ("sched_yield", 6),
        ] {
            text("wasi_snapshot_preview1", &mut imports);
            text(name, &mut imports);
            imports.push(0);
            leb(ty, &mut imports);
        }
        section(2, imports, &mut wasm);
        let mut functions = Vec::new();
        leb(5, &mut functions);
        for ty in [0, 7, 8, 7, 0] {
            leb(ty, &mut functions);
        }
        section(3, functions, &mut wasm);
        let mut exports = Vec::new();
        leb(5, &mut exports);
        text("memory", &mut exports);
        exports.push(2);
        leb(0, &mut exports);
        for (name, index) in [
            ("_start", 8),
            ("__main_void", 9),
            ("wasi_thread_start", 10),
            (ENTRY, 11),
        ] {
            text(name, &mut exports);
            exports.push(0);
            leb(index, &mut exports);
        }
        section(7, exports, &mut wasm);
        section(8, vec![12], &mut wasm);
        let mut code = Vec::new();
        leb(5, &mut code);
        code.extend([
            2, 0, 0x0b, 4, 0, 0x41, 0, 0x0b, 2, 0, 0x0b, 4, 0, 0x41, 0, 0x0b, 4, 0, 0x10, 0, 0x0b,
        ]);
        section(10, code, &mut wasm);
        let mut features = Vec::new();
        let names = [
            "atomics",
            "bulk-memory",
            "bulk-memory-opt",
            "call-indirect-overlong",
            "extended-const",
            "multivalue",
            "mutable-globals",
            "nontrapping-fptoint",
            "reference-types",
            "sign-ext",
        ];
        leb(names.len() as u32, &mut features);
        for name in names {
            features.push(b'+');
            text(name, &mut features);
        }
        custom("target_features", &features, &mut wasm);
        wasm
    }

    fn threaded_cap_fixture() -> Vec<u8> {
        let bytes = threaded_yield_fixture();
        let mut section_offset = 8;
        loop {
            let id = bytes[section_offset];
            let mut body_at = section_offset + 1;
            let mut length = 0_usize;
            let mut shift = 0;
            loop {
                let byte = bytes[body_at];
                body_at += 1;
                length |= usize::from(byte & 0x7f) << shift;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
            }
            let end = body_at + length;
            if id == 10 {
                let mut code = Vec::new();
                leb(5, &mut code);
                // `_start`: first spawn must be positive, second exactly -1;
                // the child exits normally after the host has accepted it.
                code.extend([
                    32, 0, 0x41, 0, 0x10, 1, 0x41, 0, 0x4a, 0x45, 0x04, 0x40, 0x41, 6, 0x10, 6,
                    0x0b, 0x41, 1, 0x10, 1, 0x41, 0x7f, 0x46, 0x45, 0x04, 0x40, 0x41, 7, 0x10, 6,
                    0x0b, 0x0b, // root
                    4, 0, 0x41, 0, 0x0b, // __main_void
                    6, 0, 0x20, 1, 0x10, 6, 0x0b, // wasi_thread_start
                    4, 0, 0x41, 0, 0x0b, // kernal-api-run
                    2, 0, 0x0b, // unexported module start
                ]);
                let mut output = bytes[..section_offset].to_vec();
                section(10, code, &mut output);
                output.extend_from_slice(&bytes[end..]);
                return output;
            }
            section_offset = end;
        }
    }
}

fn write_shared(memory: &SharedMemory, offset: i32, bytes: &[u8]) -> i32 {
    let Some(cells) = shared_range(memory, offset, bytes.len()) else {
        return ERRNO_FAULT;
    };
    for (cell, byte) in cells.iter().zip(bytes) {
        // SAFETY: UnsafeCell<u8> has alignment 1, matching AtomicU8. The
        // SharedMemory owns and pins this backing allocation for its lifetime,
        // and every host access to concurrently reachable guest bytes in this
        // module uses AtomicU8 (never a plain dereference).
        unsafe { AtomicU8::from_ptr(cell.get()) }.store(*byte, Ordering::Relaxed);
    }
    ERRNO_SUCCESS
}

fn validate_iovecs(memory: &SharedMemory, iovecs: i32, iovecs_len: i32) -> i32 {
    let Ok(iovecs_len) = usize::try_from(iovecs_len) else {
        return ERRNO_FAULT;
    };
    if iovecs_len > MAX_P1_IOVECS {
        return ERRNO_FAULT;
    }
    let Some(descriptor_bytes) = iovecs_len.checked_mul(8) else {
        return ERRNO_FAULT;
    };
    if shared_range(memory, iovecs, descriptor_bytes).is_none() {
        return ERRNO_FAULT;
    }
    let base = match usize::try_from(iovecs) {
        Ok(base) => base,
        Err(_) => return ERRNO_FAULT,
    };
    for index in 0..iovecs_len {
        let Some(offset) = index
            .checked_mul(8)
            .and_then(|delta| base.checked_add(delta))
        else {
            return ERRNO_FAULT;
        };
        let Ok(offset) = i32::try_from(offset) else {
            return ERRNO_FAULT;
        };
        let Some(payload) = read_shared_u32(memory, offset) else {
            return ERRNO_FAULT;
        };
        let Some(length) = read_shared_u32(memory, offset.saturating_add(4)) else {
            return ERRNO_FAULT;
        };
        if shared_range(memory, payload as i32, length as usize).is_none() {
            return ERRNO_FAULT;
        }
    }
    ERRNO_SUCCESS
}

fn read_shared_u32(memory: &SharedMemory, offset: i32) -> Option<u32> {
    let cells = shared_range(memory, offset, 4)?;
    let mut bytes = [0_u8; 4];
    for (destination, cell) in bytes.iter_mut().zip(cells) {
        // SAFETY: UnsafeCell<u8> has AtomicU8-compatible alignment and belongs
        // to the live SharedMemory. This module's host-side shared-byte access
        // invariant is atomic-only; AtomicU8 avoids a plain concurrent load.
        *destination = unsafe { AtomicU8::from_ptr(cell.get()) }.load(Ordering::Relaxed);
    }
    Some(u32::from_le_bytes(bytes))
}

fn load_shared_atomic_u32(cells: &[UnsafeCell<u8>], ordering: Ordering) -> Option<u32> {
    if cells.len() != 4 {
        return None;
    }
    let pointer = cells.as_ptr().cast::<AtomicU32>();
    if pointer.addr() % align_of::<AtomicU32>() != 0 {
        return None;
    }
    // SAFETY: callers provide exactly four live SharedMemory cells and the
    // computed backing address is checked for AtomicU32 alignment. The shared
    // report protocol permits only atomic accesses; this reference is used for
    // one load and never escapes the function.
    Some(unsafe { (&*pointer).load(ordering) })
}

// Validation reports are intentionally private: the guest returns only a
// bounded offset and the facade consumes the fixed 64-byte schema itself.
fn validate_report(memory: &SharedMemory, offset: i32) -> Result<(), SketchExecutionError> {
    if offset < 0 || offset % 4 != 0 || shared_range(memory, offset, 64).is_none() {
        return Err(SketchExecutionError::ValidationReportInvalid);
    }
    let load = |word: i32, ordering| -> Option<u32> {
        let cells = shared_range(memory, offset.checked_add(word.checked_mul(4)?)?, 4)?;
        load_shared_atomic_u32(cells, ordering)
    };
    // The guest stores ready with Release after all record fields; acquire it
    // before copying the remaining words.
    if load(3, Ordering::Acquire) != Some(1) {
        return Err(SketchExecutionError::ValidationReportInvalid);
    }
    let words: Option<Vec<u32>> = (0..16).map(|word| load(word, Ordering::Relaxed)).collect();
    let Some(words) = words else {
        return Err(SketchExecutionError::ValidationReportInvalid);
    };
    if words[0] != 0x4b_52_56_31
        || words[1] != 1
        || words[2] != 64
        || words[4] != 2
        || words[5] != 2
        || words[6] != 2
        || words[7] != 2
        || words[8] != 2
        || words[9] != 2
        || words[10] != 2
        || words[11] != 2
        || words[12] != 0
        || words[13] != 0
        || words[14] != 0
        || words[15] != 0
    {
        return Err(SketchExecutionError::ValidationReportInvalid);
    }
    Ok(())
}

#[cfg(test)]
mod validation_report_tests {
    use super::*;

    fn memory() -> SharedMemory {
        let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("engine");
        SharedMemory::new(&compiler.engine, MemoryType::shared(17, 16_384)).expect("memory")
    }

    fn write_report(memory: &SharedMemory, offset: i32, words: [u32; 16]) {
        for (index, value) in words.into_iter().enumerate() {
            let cells = shared_range(memory, offset + (index as i32 * 4), 4).expect("range");
            // SAFETY: test offsets are aligned/in-range and this helper only
            // writes the atomic record representation used by validate_report.
            unsafe { (&*cells.as_ptr().cast::<AtomicU32>()).store(value, Ordering::Release) };
        }
    }

    fn valid() -> [u32; 16] {
        [0x4b_52_56_31, 1, 64, 1, 2, 2, 2, 2, 2, 2, 2, 2, 0, 0, 0, 0]
    }

    #[test]
    fn validation_report_rejects_bad_pointer_and_schema() {
        let memory = memory();
        assert_eq!(
            validate_report(&memory, -1),
            Err(SketchExecutionError::ValidationReportInvalid)
        );
        assert_eq!(
            validate_report(&memory, 2),
            Err(SketchExecutionError::ValidationReportInvalid)
        );
        assert_eq!(
            validate_report(&memory, i32::MAX - 2),
            Err(SketchExecutionError::ValidationReportInvalid)
        );
        write_report(&memory, 0, valid());
        assert_eq!(validate_report(&memory, 0), Ok(()));
        for index in [0, 1, 2, 3, 12, 13, 14, 15] {
            let mut words = valid();
            words[index] = if index == 3 { 0 } else { 9 };
            write_report(&memory, 0, words);
            assert_eq!(
                validate_report(&memory, 0),
                Err(SketchExecutionError::ValidationReportInvalid)
            );
        }
    }

    #[test]
    fn atomic_report_load_rejects_a_misaligned_computed_address() {
        let bytes: [UnsafeCell<u8>; 8] = std::array::from_fn(|_| UnsafeCell::new(0));
        let base = bytes.as_ptr().addr();
        let misaligned = (0..4)
            .find(|offset| !(base + offset).is_multiple_of(align_of::<AtomicU32>()))
            .expect("one of four adjacent byte addresses is misaligned");
        assert_eq!(
            load_shared_atomic_u32(&bytes[misaligned..misaligned + 4], Ordering::Relaxed),
            None
        );
    }
}

fn shared_range(
    memory: &SharedMemory,
    offset: i32,
    length: usize,
) -> Option<&[std::cell::UnsafeCell<u8>]> {
    let offset = usize::try_from(offset).ok()?;
    let end = offset.checked_add(length)?;
    memory.data().get(offset..end)
}

fn map_root_error(error: &wasmtime::Error) -> Result<ThreadedRootOutcome, SketchExecutionError> {
    if let Some(exit) = error.downcast_ref::<ProcExitSentinel>() {
        return if exit.0 == 0 {
            Ok(ThreadedRootOutcome::Exited)
        } else {
            Err(SketchExecutionError::NonzeroExit { code: exit.0 })
        };
    }
    Err(SketchExecutionError::Trapped)
}

fn map_child_error(error: &wasmtime::Error) -> ChildOutcome {
    if let Some(exit) = error.downcast_ref::<ProcExitSentinel>() {
        return if exit.0 == 0 {
            ChildOutcome::Exited
        } else {
            ChildOutcome::NonzeroExit(exit.0)
        };
    }
    ChildOutcome::Trapped
}

fn resolve_threaded_result(
    root: Result<ThreadedRootOutcome, SketchExecutionError>,
    children: Result<(), SketchExecutionError>,
    report: Result<(), SketchExecutionError>,
    rejections: ThreadSpawnRejectionSummary,
) -> Result<ThreadedRootOutcome, SketchExecutionError> {
    // All callers already drained children before this pure precedence step.
    // Root remains primary; ordered child aggregation is next; report schema
    // failures are only meaningful after successful guest execution.
    let outcome = root?;
    children?;
    report?;
    Ok(match outcome {
        ThreadedRootOutcome::Started if !rejections.is_empty() => {
            ThreadedRootOutcome::StartedWithThreadRejections(rejections)
        }
        ThreadedRootOutcome::Exited if !rejections.is_empty() => {
            ThreadedRootOutcome::ExitedWithThreadRejections(rejections)
        }
        outcome => outcome,
    })
}

#[cfg(test)]
mod result_precedence_tests {
    use super::*;

    fn child_error() -> SketchExecutionError {
        SketchExecutionError::ChildOutcomes {
            outcomes: vec![
                ThreadedChildOutcome {
                    tid: 1,
                    kind: ThreadedChildOutcomeKind::Trapped,
                },
                ThreadedChildOutcome {
                    tid: 2,
                    kind: ThreadedChildOutcomeKind::NonzeroExit { code: 9 },
                },
            ],
        }
    }

    #[test]
    fn root_child_and_report_precedence_is_semantic() {
        assert_eq!(
            resolve_threaded_result(
                Err(SketchExecutionError::NonzeroExit { code: 7 }),
                Err(child_error()),
                Err(SketchExecutionError::ValidationReportInvalid),
                ThreadSpawnRejectionSummary::default(),
            ),
            Err(SketchExecutionError::NonzeroExit { code: 7 }),
        );
        assert_eq!(
            resolve_threaded_result(
                Ok(ThreadedRootOutcome::Started),
                Err(child_error()),
                Err(SketchExecutionError::ValidationReportInvalid),
                ThreadSpawnRejectionSummary::default(),
            ),
            Err(child_error()),
        );
        assert_eq!(
            resolve_threaded_result(
                Ok(ThreadedRootOutcome::Started),
                Ok(()),
                Err(SketchExecutionError::ValidationReportInvalid),
                ThreadSpawnRejectionSummary::default(),
            ),
            Err(SketchExecutionError::ValidationReportInvalid),
        );
    }

    #[test]
    fn thread_rejection_summary_surfaces_only_after_primary_success() {
        let summary = ThreadSpawnRejectionSummary {
            capacity: 1,
            closing: 0,
        };
        assert_eq!(
            resolve_threaded_result(Ok(ThreadedRootOutcome::Started), Ok(()), Ok(()), summary,),
            Ok(ThreadedRootOutcome::StartedWithThreadRejections(summary)),
        );
        assert_eq!(
            resolve_threaded_result(
                Err(SketchExecutionError::NonzeroExit { code: 7 }),
                Ok(()),
                Ok(()),
                summary,
            ),
            Err(SketchExecutionError::NonzeroExit { code: 7 }),
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchCompilerError {
    InvalidStackLimit,
    InvalidExecutionLimits,
    Unavailable,
}
impl fmt::Display for SketchCompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidStackLimit => "the Wasm stack limit must be nonzero",
            Self::InvalidExecutionLimits => {
                "execution limits must admit one 1 GiB threaded Rust session and one root"
            }
            Self::Unavailable => "the sketch compiler is unavailable",
        })
    }
}
impl std::error::Error for SketchCompilerError {}

/// Stable bounded diagnostics; no parser or engine message crosses this facade.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchModuleError {
    ModuleTooLarge {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    InvalidBinary,
    InvalidModuleLimit,
    InvalidSharedMemoryLimit,
    InvalidThreadLimit,
    ThreadLimitExceedsV1Maximum {
        requested: usize,
        maximum: usize,
    },
    ForbiddenImport {
        module: String,
        name: String,
    },
    ImportTypeMismatch {
        module: String,
        name: String,
    },
    MissingRequiredImport {
        module: &'static str,
        name: &'static str,
    },
    MissingSharedMemory,
    MultipleMemoryImports,
    DefinedMemoryForbidden,
    UnsharedMemory,
    Memory64,
    UnsupportedMemoryPageSize,
    SharedMemoryWithoutMaximum,
    MemoryInitialExceedsMaximum {
        minimum_pages: u64,
        maximum_pages: u64,
    },
    SharedMemoryExceedsPolicy {
        minimum_pages: u64,
        maximum_pages: u64,
        policy_pages: u32,
    },
    ThreadedMemoryMismatch {
        minimum_pages: u32,
        maximum_pages: u32,
    },
    MissingMetadata {
        name: &'static str,
    },
    DuplicateMetadata {
        name: &'static str,
    },
    MetadataMismatch {
        name: &'static str,
    },
    MetadataTooLarge {
        name: &'static str,
    },
    StartFunctionForbidden,
    ExportNotAllowed {
        name: String,
    },
    EntrypointMismatch,
    ForbiddenCustomSection {
        name: String,
    },
    MissingTargetFeatures,
    TargetFeaturesMismatch,
    StartMismatch,
    MemoryExportMismatch,
}
impl SketchModuleError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::ModuleTooLarge { .. } => "module-too-large",
            Self::InvalidBinary => "invalid-binary",
            Self::InvalidModuleLimit => "invalid-module-limit",
            Self::InvalidSharedMemoryLimit => "invalid-shared-memory-limit",
            Self::InvalidThreadLimit => "invalid-thread-limit",
            Self::ThreadLimitExceedsV1Maximum { .. } => "thread-limit-exceeds-v1-maximum",
            Self::ForbiddenImport { .. } => "forbidden-import",
            Self::ImportTypeMismatch { .. } => "import-type-mismatch",
            Self::MissingRequiredImport { .. } => "missing-required-import",
            Self::MissingSharedMemory => "missing-shared-memory",
            Self::MultipleMemoryImports => "multiple-memory-imports",
            Self::DefinedMemoryForbidden => "defined-memory-forbidden",
            Self::UnsharedMemory => "unshared-memory",
            Self::Memory64 => "memory64",
            Self::UnsupportedMemoryPageSize => "unsupported-memory-page-size",
            Self::SharedMemoryWithoutMaximum => "shared-memory-without-maximum",
            Self::MemoryInitialExceedsMaximum { .. } => "memory-initial-exceeds-maximum",
            Self::SharedMemoryExceedsPolicy { .. } => "shared-memory-exceeds-policy",
            Self::ThreadedMemoryMismatch { .. } => "threaded-memory-mismatch",
            Self::MissingMetadata { .. } => "missing-metadata",
            Self::DuplicateMetadata { .. } => "duplicate-metadata",
            Self::MetadataMismatch { .. } => "metadata-mismatch",
            Self::MetadataTooLarge { .. } => "metadata-too-large",
            Self::StartFunctionForbidden => "start-function-forbidden",
            Self::ExportNotAllowed { .. } => "export-not-allowed",
            Self::EntrypointMismatch => "entrypoint-mismatch",
            Self::ForbiddenCustomSection { .. } => "forbidden-custom-section",
            Self::MissingTargetFeatures => "missing-target-features",
            Self::TargetFeaturesMismatch => "target-features-mismatch",
            Self::StartMismatch => "start-mismatch",
            Self::MemoryExportMismatch => "memory-export-mismatch",
        }
    }
}
impl fmt::Display for SketchModuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sketch admission failed ({})", self.code())
    }
}
impl std::error::Error for SketchModuleError {}

/// Deterministically describes the semantic sections of an untrusted Wasm
/// artifact for the threaded-smoke RED test. This is intentionally an
/// inspection seam, not a loader or an execution API.
#[doc(hidden)]
pub fn threaded_artifact_manifest_for_test(bytes: &[u8]) -> Result<String, SketchModuleError> {
    let mut facts = Vec::new();
    let mut types = Vec::<TypeEntry>::new();
    let mut imported_function_types = Vec::<u32>::new();
    let mut functions = Vec::<u32>::new();
    let mut exports = Vec::<(String, ExternalKind, u32)>::new();
    let mut start = None;
    let mut type_index = 0_u32;
    for item in Parser::new(0).parse_all(bytes) {
        match item.map_err(|_| SketchModuleError::InvalidBinary)? {
            Payload::TypeSection(reader) => {
                for group in reader {
                    let group = group.map_err(|_| SketchModuleError::InvalidBinary)?;
                    for ty in group.types() {
                        let fact =
                            if let CompositeInnerType::Func(function) = &ty.composite_type.inner {
                                types.push(TypeEntry::Function {
                                    params: function.params().to_vec(),
                                    results: function.results().to_vec(),
                                });
                                format!(
                                "type index={type_index} kind=function params={:?} results={:?}",
                                function.params(),
                                function.results(),
                            )
                            } else {
                                types.push(TypeEntry::NonFunction);
                                format!("type index={type_index} kind=non-function")
                            };
                        facts.push(fact);
                        type_index += 1;
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|_| SketchModuleError::InvalidBinary)?;
                    facts.push(format!(
                        "import module={} name={} kind={}",
                        bounded(import.module),
                        bounded(import.name),
                        import_kind(import.ty),
                    ));
                    if let TypeRef::Func(index) | TypeRef::FuncExact(index) = import.ty {
                        imported_function_types.push(index);
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for index in reader {
                    functions.push(index.map_err(|_| SketchModuleError::InvalidBinary)?);
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let memory = memory.map_err(|_| SketchModuleError::InvalidBinary)?;
                    facts.push(memory_fact(
                        "defined-memory",
                        memory.initial,
                        memory.maximum,
                        memory.shared,
                        memory.memory64,
                        memory.page_size_log2,
                    ));
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|_| SketchModuleError::InvalidBinary)?;
                    exports.push((bounded(export.name), export.kind, export.index));
                }
            }
            Payload::StartSection { func, .. } => start = Some(func),
            Payload::CustomSection(section) => facts.push(format!(
                "custom name={} bytes={}",
                bounded(section.name()),
                section.data().len(),
            )),
            _ => {}
        }
    }
    for (name, kind, index) in exports {
        let type_index = match kind {
            ExternalKind::Func | ExternalKind::FuncExact => {
                function_type(index, &imported_function_types, &functions)
            }
            _ => None,
        };
        facts.push(format!(
            "export name={name} kind={} index={index} type={type_index:?}",
            external_kind(kind),
        ));
    }
    if let Some(index) = start {
        let type_index = function_type(index, &imported_function_types, &functions);
        facts.push(format!("start function index={index} type={type_index:?}"));
    }
    facts.sort_unstable();
    Ok(format!(
        "threaded-artifact-manifest-v1\n{}\n",
        facts.join("\n")
    ))
}

fn import_kind(import: TypeRef) -> String {
    match import {
        TypeRef::Func(index) | TypeRef::FuncExact(index) => format!("function type={index}"),
        TypeRef::Table(_) => "table".to_owned(),
        TypeRef::Memory(memory) => memory_fact(
            "memory",
            memory.initial,
            memory.maximum,
            memory.shared,
            memory.memory64,
            memory.page_size_log2,
        ),
        TypeRef::Global(_) => "global".to_owned(),
        TypeRef::Tag(_) => "tag".to_owned(),
    }
}

fn memory_fact(
    prefix: &str,
    initial: u64,
    maximum: Option<u64>,
    shared: bool,
    memory64: bool,
    page_size_log2: Option<u32>,
) -> String {
    format!(
        "{prefix} initial={initial} maximum={maximum:?} shared={shared} memory64={memory64} page-size-log2={page_size_log2:?}"
    )
}

fn external_kind(kind: ExternalKind) -> &'static str {
    match kind {
        ExternalKind::Func => "function",
        ExternalKind::FuncExact => "function-exact",
        ExternalKind::Table => "table",
        ExternalKind::Memory => "memory",
        ExternalKind::Global => "global",
        ExternalKind::Tag => "tag",
    }
}

#[derive(Clone, Copy)]
struct Signature {
    params: &'static [ValType],
    results: &'static [ValType],
}
const EMPTY: &[ValType] = &[];
const I32: &[ValType] = &[ValType::I32];

/// The type section may contain GC composite types. They are not empty
/// function signatures: keeping that distinction makes ABI type references
/// fail at preflight instead of being mistaken for `() -> ()`.
enum TypeEntry {
    Function {
        params: Vec<ValType>,
        results: Vec<ValType>,
    },
    NonFunction,
}

fn preflight(
    bytes: &[u8],
    policy: SketchModulePolicy,
) -> Result<SketchSharedMemory, SketchModuleError> {
    let mut types = Vec::<TypeEntry>::new();
    let mut functions = Vec::<u32>::new();
    let mut imported_functions = 0_u32;
    let mut memory = None;
    let mut saw_thread = false;
    let mut saw_yield = false;
    let mut exports = Vec::<(String, ExternalKind, u32)>::new();
    let mut abi = 0;
    let mut profile = 0;
    for item in Parser::new(0).parse_all(bytes) {
        match item.map_err(|_| SketchModuleError::InvalidBinary)? {
            Payload::TypeSection(reader) => {
                for group in reader {
                    let group = group.map_err(|_| SketchModuleError::InvalidBinary)?;
                    for ty in group.types() {
                        if let CompositeInnerType::Func(function) = &ty.composite_type.inner {
                            types.push(TypeEntry::Function {
                                params: function.params().to_vec(),
                                results: function.results().to_vec(),
                            });
                        } else {
                            types.push(TypeEntry::NonFunction);
                        }
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|_| SketchModuleError::InvalidBinary)?;
                    match (import.module, import.name, import.ty) {
                        (MEMORY_MODULE, MEMORY_NAME, TypeRef::Memory(ty)) => {
                            if memory.is_some() {
                                return Err(SketchModuleError::MultipleMemoryImports);
                            }
                            memory = Some(check_memory(
                                ty.initial,
                                ty.maximum,
                                ty.shared,
                                ty.memory64,
                                ty.page_size_log2,
                                policy,
                            )?);
                        }
                        (THREAD_MODULE, THREAD_SPAWN, TypeRef::Func(i) | TypeRef::FuncExact(i)) => {
                            check_signature(
                                &types,
                                i,
                                Signature {
                                    params: I32,
                                    results: I32,
                                },
                                THREAD_MODULE,
                                THREAD_SPAWN,
                            )?;
                            saw_thread = true;
                            imported_functions += 1;
                        }
                        (ABI_MODULE, ABI_YIELD, TypeRef::Func(i) | TypeRef::FuncExact(i)) => {
                            check_signature(
                                &types,
                                i,
                                Signature {
                                    params: EMPTY,
                                    results: EMPTY,
                                },
                                ABI_MODULE,
                                ABI_YIELD,
                            )?;
                            saw_yield = true;
                            imported_functions += 1;
                        }
                        (module, name, _) => {
                            return Err(SketchModuleError::ForbiddenImport {
                                module: bounded(module),
                                name: bounded(name),
                            })
                        }
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for index in reader {
                    functions.push(index.map_err(|_| SketchModuleError::InvalidBinary)?);
                }
            }
            // This profile has exactly one shared memory, supplied by the host.
            // Rejecting a defined memory here also guarantees that no second
            // memory shape reaches Wasmtime before the semantic boundary.
            Payload::MemorySection(_) => return Err(SketchModuleError::DefinedMemoryForbidden),
            Payload::ExportSection(reader) => {
                for export in reader {
                    let e = export.map_err(|_| SketchModuleError::InvalidBinary)?;
                    exports.push((bounded(e.name), e.kind, e.index));
                }
            }
            Payload::StartSection { .. } => return Err(SketchModuleError::StartFunctionForbidden),
            Payload::CustomSection(section) => {
                let (count, expected, name) = if section.name() == ABI_METADATA {
                    (&mut abi, ABI_METADATA_VALUE, ABI_METADATA)
                } else if section.name() == PROFILE_METADATA {
                    (&mut profile, PROFILE_METADATA_VALUE, PROFILE_METADATA)
                } else {
                    continue;
                };
                *count += 1;
                if *count > 1 {
                    return Err(SketchModuleError::DuplicateMetadata { name });
                }
                if section.data().len() > MAX_METADATA_BYTES {
                    return Err(SketchModuleError::MetadataTooLarge { name });
                }
                if section.data() != expected {
                    return Err(SketchModuleError::MetadataMismatch { name });
                }
            }
            _ => {}
        }
    }
    let memory = memory.ok_or(SketchModuleError::MissingSharedMemory)?;
    if !saw_thread {
        return Err(SketchModuleError::MissingRequiredImport {
            module: THREAD_MODULE,
            name: THREAD_SPAWN,
        });
    }
    if !saw_yield {
        return Err(SketchModuleError::MissingRequiredImport {
            module: ABI_MODULE,
            name: ABI_YIELD,
        });
    }
    if abi == 0 {
        return Err(SketchModuleError::MissingMetadata { name: ABI_METADATA });
    }
    if profile == 0 {
        return Err(SketchModuleError::MissingMetadata {
            name: PROFILE_METADATA,
        });
    }
    if exports.len() != 1 || exports[0].0 != ENTRY || exports[0].1 != ExternalKind::Func {
        return Err(SketchModuleError::ExportNotAllowed {
            name: exports.first().map(|e| e.0.clone()).unwrap_or_default(),
        });
    }
    let index = exports[0]
        .2
        .checked_sub(imported_functions)
        .ok_or(SketchModuleError::EntrypointMismatch)? as usize;
    let ty = *functions
        .get(index)
        .ok_or(SketchModuleError::EntrypointMismatch)?;
    check_entrypoint_signature(&types, ty)?;
    Ok(memory)
}
fn check_memory(
    initial: u64,
    maximum: Option<u64>,
    shared: bool,
    memory64: bool,
    page_size_log2: Option<u32>,
    policy: SketchModulePolicy,
) -> Result<SketchSharedMemory, SketchModuleError> {
    if memory64 {
        return Err(SketchModuleError::Memory64);
    }
    if !shared {
        return Err(SketchModuleError::UnsharedMemory);
    }
    if page_size_log2.is_some() {
        return Err(SketchModuleError::UnsupportedMemoryPageSize);
    }
    let maximum = maximum.ok_or(SketchModuleError::SharedMemoryWithoutMaximum)?;
    if initial > maximum {
        return Err(SketchModuleError::MemoryInitialExceedsMaximum {
            minimum_pages: initial,
            maximum_pages: maximum,
        });
    }
    if initial > u64::from(policy.max_shared_memory_pages)
        || maximum > u64::from(policy.max_shared_memory_pages)
    {
        return Err(SketchModuleError::SharedMemoryExceedsPolicy {
            minimum_pages: initial,
            maximum_pages: maximum,
            policy_pages: policy.max_shared_memory_pages,
        });
    }
    Ok(SketchSharedMemory {
        minimum_pages: initial as u32,
        maximum_pages: maximum as u32,
    })
}

// Rust's wasi1 threads target has a deliberately closed compatibility import
// surface. These names are recorded here, but this admission slice links none
// of them and grants no filesystem, clock, environment, or process authority.
fn preflight_threaded_rust(
    bytes: &[u8],
    policy: SketchModulePolicy,
    validation: bool,
) -> Result<SketchSharedMemory, SketchModuleError> {
    let mut types = Vec::<TypeEntry>::new();
    let mut functions = Vec::<u32>::new();
    let mut imported_function_types = Vec::<u32>::new();
    let mut memory = None;
    let mut memory_export = None;
    let mut exports = Vec::<(String, ExternalKind, u32)>::new();
    let mut start = None;
    let mut target_features = None;
    let mut validation_metadata = 0_u8;
    let mut seen = std::collections::BTreeSet::new();
    for item in Parser::new(0).parse_all(bytes) {
        match item.map_err(|_| SketchModuleError::InvalidBinary)? {
            Payload::TypeSection(reader) => {
                for group in reader {
                    let group = group.map_err(|_| SketchModuleError::InvalidBinary)?;
                    for ty in group.types() {
                        if let CompositeInnerType::Func(function) = &ty.composite_type.inner {
                            types.push(TypeEntry::Function {
                                params: function.params().to_vec(),
                                results: function.results().to_vec(),
                            });
                        } else {
                            types.push(TypeEntry::NonFunction);
                        }
                    }
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import.map_err(|_| SketchModuleError::InvalidBinary)?;
                    match import.ty {
                        TypeRef::Memory(ty)
                            if import.module == MEMORY_MODULE && import.name == MEMORY_NAME =>
                        {
                            if memory.is_some() {
                                return Err(SketchModuleError::MultipleMemoryImports);
                            }
                            memory = Some(check_memory(
                                ty.initial,
                                ty.maximum,
                                ty.shared,
                                ty.memory64,
                                ty.page_size_log2,
                                policy,
                            )?);
                        }
                        TypeRef::Func(index) | TypeRef::FuncExact(index) => {
                            if !threaded_import_signature(
                                import.module,
                                import.name,
                                &types,
                                index,
                            )? {
                                return Err(SketchModuleError::ForbiddenImport {
                                    module: bounded(import.module),
                                    name: bounded(import.name),
                                });
                            }
                            if !seen.insert((import.module, import.name)) {
                                return Err(SketchModuleError::ForbiddenImport {
                                    module: bounded(import.module),
                                    name: bounded(import.name),
                                });
                            }
                            imported_function_types.push(index);
                        }
                        _ => {
                            return Err(SketchModuleError::ForbiddenImport {
                                module: bounded(import.module),
                                name: bounded(import.name),
                            })
                        }
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for index in reader {
                    functions.push(index.map_err(|_| SketchModuleError::InvalidBinary)?);
                }
            }
            Payload::MemorySection(_) => return Err(SketchModuleError::DefinedMemoryForbidden),
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export.map_err(|_| SketchModuleError::InvalidBinary)?;
                    if export.name == "memory" {
                        memory_export = Some((export.kind, export.index));
                    }
                    exports.push((bounded(export.name), export.kind, export.index));
                }
            }
            Payload::StartSection { func, .. } => start = Some(func),
            Payload::CustomSection(section) => match section.name() {
                PROFILE_METADATA if validation => {
                    validation_metadata += 1;
                    if validation_metadata > 1 {
                        return Err(SketchModuleError::DuplicateMetadata {
                            name: PROFILE_METADATA,
                        });
                    }
                    if section.data() != VALIDATION_PROFILE_METADATA_VALUE {
                        return Err(SketchModuleError::MetadataMismatch {
                            name: PROFILE_METADATA,
                        });
                    }
                }
                "target_features" => {
                    if target_features
                        .replace(parse_target_features(section.data())?)
                        .is_some()
                    {
                        return Err(SketchModuleError::TargetFeaturesMismatch);
                    }
                }
                "name" | "producers" | ".debug_abbrev" | ".debug_info" | ".debug_line"
                | ".debug_ranges" | ".debug_str" => {
                    if section.data().len() > MAX_METADATA_BYTES * 1024 {
                        return Err(SketchModuleError::MetadataTooLarge { name: "debug" });
                    }
                }
                name => {
                    return Err(SketchModuleError::ForbiddenCustomSection {
                        name: bounded(name),
                    })
                }
            },
            _ => {}
        }
    }
    let memory = memory.ok_or(SketchModuleError::MissingSharedMemory)?;
    if memory.minimum_pages != THREADED_RUST_INITIAL_PAGES
        || memory.maximum_pages != THREADED_RUST_MAX_PAGES
    {
        return Err(SketchModuleError::ThreadedMemoryMismatch {
            minimum_pages: memory.minimum_pages,
            maximum_pages: memory.maximum_pages,
        });
    }
    if policy.max_shared_memory_pages < THREADED_RUST_MAX_PAGES {
        return Err(SketchModuleError::SharedMemoryExceedsPolicy {
            minimum_pages: memory.minimum_pages.into(),
            maximum_pages: THREADED_RUST_MAX_PAGES.into(),
            policy_pages: policy.max_shared_memory_pages,
        });
    }
    if memory_export != Some((ExternalKind::Memory, 0)) {
        return Err(SketchModuleError::MemoryExportMismatch);
    }
    let required = [
        (ABI_MODULE, ABI_YIELD),
        (THREAD_MODULE, THREAD_SPAWN),
        ("wasi_snapshot_preview1", "clock_time_get"),
        ("wasi_snapshot_preview1", "environ_get"),
        ("wasi_snapshot_preview1", "environ_sizes_get"),
        ("wasi_snapshot_preview1", "fd_write"),
        ("wasi_snapshot_preview1", "proc_exit"),
        ("wasi_snapshot_preview1", "sched_yield"),
    ];
    if !required.iter().all(|pair| seen.contains(pair)) || seen.len() != required.len() {
        return Err(SketchModuleError::MissingRequiredImport {
            module: "threaded-rust-v1",
            name: "closed-import-set",
        });
    }
    let features = target_features.ok_or(SketchModuleError::MissingTargetFeatures)?;
    let expected_features = [
        "atomics",
        "bulk-memory",
        "bulk-memory-opt",
        "call-indirect-overlong",
        "extended-const",
        "multivalue",
        "mutable-globals",
        "nontrapping-fptoint",
        "reference-types",
        "sign-ext",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    if features != expected_features {
        return Err(SketchModuleError::TargetFeaturesMismatch);
    }
    let mut allowed = vec![
        ("memory", ExternalKind::Memory),
        ("_start", ExternalKind::Func),
        ("__main_void", ExternalKind::Func),
        ("wasi_thread_start", ExternalKind::Func),
        (ENTRY, ExternalKind::Func),
    ];
    if validation {
        allowed.push((VALIDATION_REPORT, ExternalKind::Func));
        if validation_metadata != 1 {
            return Err(SketchModuleError::MissingMetadata {
                name: PROFILE_METADATA,
            });
        }
    }
    if exports.len() != allowed.len() {
        return Err(SketchModuleError::ExportNotAllowed {
            name: exports.first().map(|e| e.0.clone()).unwrap_or_default(),
        });
    }
    if let Some((name, _, _)) = exports
        .iter()
        .find(|(name, kind, _)| !allowed.contains(&(name.as_str(), *kind)))
    {
        return Err(SketchModuleError::ExportNotAllowed { name: name.clone() });
    }
    let mut by_name = std::collections::BTreeMap::new();
    for (name, _, index) in exports {
        by_name.insert(name, index);
    }
    let mut signatures = vec![
        (
            "_start",
            Signature {
                params: EMPTY,
                results: EMPTY,
            },
        ),
        (
            "__main_void",
            Signature {
                params: EMPTY,
                results: I32,
            },
        ),
        (
            "wasi_thread_start",
            Signature {
                params: &[ValType::I32, ValType::I32],
                results: EMPTY,
            },
        ),
        (
            ENTRY,
            Signature {
                params: EMPTY,
                results: I32,
            },
        ),
    ];
    if validation {
        signatures.push((
            VALIDATION_REPORT,
            Signature {
                params: EMPTY,
                results: I32,
            },
        ));
    }
    for (name, signature) in signatures {
        let index = *by_name
            .get(name)
            .ok_or(SketchModuleError::EntrypointMismatch)?;
        let type_index = function_type(index, &imported_function_types, &functions)
            .ok_or(SketchModuleError::EntrypointMismatch)?;
        check_signature(&types, type_index, signature, ABI_MODULE, name)?;
    }
    // Rust emits a dedicated () -> () start function. It is intentionally not
    // required to equal exported `_start`: import admission, not that identity,
    // defines the authority boundary, and the deterministic Rust artifact uses
    // distinct functions.
    let start = start.ok_or(SketchModuleError::StartMismatch)?;
    let start_type = function_type(start, &imported_function_types, &functions)
        .ok_or(SketchModuleError::StartMismatch)?;
    let Some(TypeEntry::Function { params, results }) = types.get(start_type as usize) else {
        return Err(SketchModuleError::StartMismatch);
    };
    if params.as_slice() != EMPTY || results.as_slice() != EMPTY {
        return Err(SketchModuleError::StartMismatch);
    }
    Ok(memory)
}

fn function_type(index: u32, imported: &[u32], defined: &[u32]) -> Option<u32> {
    if (index as usize) < imported.len() {
        imported.get(index as usize).copied()
    } else {
        defined.get(index as usize - imported.len()).copied()
    }
}

fn threaded_import_signature(
    module: &str,
    name: &str,
    types: &[TypeEntry],
    index: u32,
) -> Result<bool, SketchModuleError> {
    let signature = match (module, name) {
        (ABI_MODULE, ABI_YIELD) => Signature {
            params: EMPTY,
            results: EMPTY,
        },
        (THREAD_MODULE, THREAD_SPAWN) => Signature {
            params: I32,
            results: I32,
        },
        ("wasi_snapshot_preview1", "clock_time_get") => Signature {
            params: &[ValType::I32, ValType::I64, ValType::I32],
            results: I32,
        },
        ("wasi_snapshot_preview1", "environ_get")
        | ("wasi_snapshot_preview1", "environ_sizes_get") => Signature {
            params: &[ValType::I32, ValType::I32],
            results: I32,
        },
        ("wasi_snapshot_preview1", "fd_write") => Signature {
            params: &[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            results: I32,
        },
        ("wasi_snapshot_preview1", "proc_exit") => Signature {
            params: I32,
            results: EMPTY,
        },
        ("wasi_snapshot_preview1", "sched_yield") => Signature {
            params: EMPTY,
            results: I32,
        },
        _ => return Ok(false),
    };
    check_signature(types, index, signature, module, name)?;
    Ok(true)
}

fn parse_target_features(
    data: &[u8],
) -> Result<std::collections::BTreeSet<String>, SketchModuleError> {
    let (count, mut offset) = read_leb(data, 0)?;
    let mut result = std::collections::BTreeSet::new();
    for _ in 0..count {
        if offset >= data.len() {
            return Err(SketchModuleError::TargetFeaturesMismatch);
        }
        let prefix = data[offset];
        offset += 1;
        if prefix != b'+' {
            return Err(SketchModuleError::TargetFeaturesMismatch);
        }
        let (length, next) = read_leb(data, offset)?;
        offset = next;
        let end = offset
            .checked_add(length as usize)
            .ok_or(SketchModuleError::TargetFeaturesMismatch)?;
        let name = std::str::from_utf8(
            data.get(offset..end)
                .ok_or(SketchModuleError::TargetFeaturesMismatch)?,
        )
        .map_err(|_| SketchModuleError::TargetFeaturesMismatch)?;
        if !result.insert(bounded(name)) {
            return Err(SketchModuleError::TargetFeaturesMismatch);
        }
        offset = end;
    }
    if offset != data.len() {
        return Err(SketchModuleError::TargetFeaturesMismatch);
    }
    Ok(result)
}

fn read_leb(data: &[u8], mut offset: usize) -> Result<(u32, usize), SketchModuleError> {
    let mut value = 0_u32;
    for shift in (0..35).step_by(7) {
        let byte = *data
            .get(offset)
            .ok_or(SketchModuleError::TargetFeaturesMismatch)?;
        offset += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok((value, offset));
        }
    }
    Err(SketchModuleError::TargetFeaturesMismatch)
}
fn check_signature(
    types: &[TypeEntry],
    index: u32,
    expected: Signature,
    module: &str,
    name: &str,
) -> Result<(), SketchModuleError> {
    let Some(TypeEntry::Function { params, results }) = types.get(index as usize) else {
        return Err(SketchModuleError::ImportTypeMismatch {
            module: bounded(module),
            name: bounded(name),
        });
    };
    if params.as_slice() != expected.params || results.as_slice() != expected.results {
        return Err(SketchModuleError::ImportTypeMismatch {
            module: bounded(module),
            name: bounded(name),
        });
    }
    Ok(())
}

fn check_entrypoint_signature(types: &[TypeEntry], index: u32) -> Result<(), SketchModuleError> {
    let Some(TypeEntry::Function { params, results }) = types.get(index as usize) else {
        return Err(SketchModuleError::EntrypointMismatch);
    };
    if params.as_slice() != EMPTY || results.as_slice() != EMPTY {
        // Preserve the existing stable error for a function-valued entrypoint
        // whose ABI is wrong; only a non-function type gets the distinct
        // entrypoint-type diagnostic above.
        return Err(SketchModuleError::ImportTypeMismatch {
            module: ABI_MODULE.to_owned(),
            name: ENTRY.to_owned(),
        });
    }
    Ok(())
}
fn bounded(value: &str) -> String {
    value.chars().take(96).collect()
}
