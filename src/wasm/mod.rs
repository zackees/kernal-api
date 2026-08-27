//! Bounded admission of versioned core-Wasm sketches.
//!
//! Admission performs one private Cranelift compilation after the complete
//! binary contract has succeeded. The threaded-root profile can then start in
//! a fresh private store; no Wasmtime handle appears in the public API.

use std::fmt;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, OnceLock};

use wasmparser::{CompositeInnerType, ExternalKind, Parser, Payload, TypeRef, ValType};
use wasmtime::{
    Caller, Config, Engine, InstancePre, Linker, MemoryType, Module, SharedMemory, Store,
    Strategy,
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
const MAX_METADATA_BYTES: usize = 128;
const THREADED_RUST_INITIAL_PAGES: u32 = 17;
const THREADED_RUST_MAX_PAGES: u32 = 16_384;
const ERRNO_SUCCESS: i32 = 0;
const ERRNO_FAULT: i32 = 21;
const ERRNO_THREAD_SPAWN_UNAVAILABLE: i32 = 11;

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
}
impl SketchCompilerConfig {
    pub fn new(max_wasm_stack_bytes: usize) -> Result<Self, SketchCompilerError> {
        if max_wasm_stack_bytes == 0 {
            return Err(SketchCompilerError::InvalidStackLimit);
        }
        Ok(Self {
            max_wasm_stack_bytes,
        })
    }
    pub fn max_wasm_stack_bytes(self) -> usize {
        self.max_wasm_stack_bytes
    }
}
impl Default for SketchCompilerConfig {
    fn default() -> Self {
        Self {
            max_wasm_stack_bytes: 2 * 1024 * 1024,
        }
    }
}

/// Facade-owned compiler for the selected core-Wasm profile.
#[derive(Clone)]
pub struct SketchCompiler {
    engine: Arc<Engine>,
    compilations: Arc<AtomicU64>,
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
            SketchAdmissionProfile::ThreadedRustV1 => preflight_threaded_rust(bytes, policy)?,
        };
        let module =
            Module::new(&self.engine, bytes).map_err(|_| SketchModuleError::InvalidBinary)?;
        self.compilations.fetch_add(1, Ordering::Relaxed);
        Ok(AdmittedSketch {
            engine: Arc::clone(&self.engine),
            module,
            module_bytes: bytes.len(),
            shared_memory: memory,
        })
    }
    /// Number of modules whose complete preflight reached private compilation.
    pub fn compiled_module_count(&self) -> u64 {
        self.compilations.load(Ordering::Relaxed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchModulePolicy {
    max_module_bytes: usize,
    max_shared_memory_pages: u32,
    profile: SketchAdmissionProfile,
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
            profile: SketchAdmissionProfile::SyntheticV1,
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
    pub fn max_module_bytes(self) -> usize {
        self.max_module_bytes
    }
    pub fn max_shared_memory_pages(self) -> u32 {
        self.max_shared_memory_pages
    }
    pub fn profile(self) -> SketchAdmissionProfile {
        self.profile
    }
}

/// An admitted module. The backend object is private to the crate.
pub struct AdmittedSketch {
    engine: Arc<Engine>,
    module: Module,
    module_bytes: usize,
    shared_memory: SketchSharedMemory,
}
impl AdmittedSketch {
    pub fn module_bytes(&self) -> usize {
        self.module_bytes
    }
    pub fn shared_memory(&self) -> SketchSharedMemory {
        self.shared_memory
    }
    /// Instantiates the admitted module in one fresh root store. Instantiation
    /// is the sole entry operation: Wasmtime invokes the module start itself.
    /// Child guest threads are deliberately unavailable until the next slice.
    pub fn execute_threaded_root(
        &self,
        runtime: crate::async_engine::RuntimeHandle,
    ) -> Result<ThreadedRootOutcome, SketchExecutionError> {
        let memory = SharedMemory::new(
            &self.engine,
            MemoryType::shared(
                self.shared_memory.minimum_pages,
                self.shared_memory.maximum_pages,
            ),
        )
        .map_err(|_| SketchExecutionError::SharedMemoryUnavailable)?;
        let controller = Arc::new(ThreadController {
            memory,
            prelink: OnceLock::new(),
        });
        let mut linker = Linker::new(&self.engine);
        define_closed_imports(&mut linker)?;

        // SharedMemory is engine-owned, so this bootstrap store is only used to
        // register the import and never owns an instance or crosses a thread.
        let bootstrap = Store::new(
            &self.engine,
            ThreadStoreState {
                controller: Arc::clone(&controller),
                runtime: runtime.clone(),
            },
        );
        linker
            .define(&bootstrap, MEMORY_MODULE, MEMORY_NAME, controller.memory.clone())
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

        let mut store = Store::new(
            &self.engine,
            ThreadStoreState {
                controller,
                runtime: runtime.clone(),
            },
        );
        match prelink.instantiate(&mut store) {
            Ok(_) => Ok(ThreadedRootOutcome::Started),
            Err(error) => map_root_error(&error),
        }
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
    /// The module requested the normal `proc_exit(0)` completion path.
    Exited,
}

/// Bounded failures from private threaded-root setup and start execution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchExecutionError {
    SharedMemoryUnavailable,
    PrelinkFailed,
    NonzeroExit { code: i32 },
    Trapped,
}
impl SketchExecutionError {
    pub fn code(self) -> &'static str {
        match self {
            Self::SharedMemoryUnavailable => "shared-memory-unavailable",
            Self::PrelinkFailed => "prelink-failed",
            Self::NonzeroExit { .. } => "nonzero-exit",
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

struct ThreadController {
    memory: SharedMemory,
    // Callbacks reach their controller through ThreadStoreState. Keeping the
    // prelink here breaks their otherwise cyclic construction without letting a
    // Store or Instance escape to another thread.
    prelink: OnceLock<Arc<InstancePre<ThreadStoreState>>>,
}

struct ThreadStoreState {
    controller: Arc<ThreadController>,
    runtime: crate::async_engine::RuntimeHandle,
}

#[derive(Debug, thiserror::Error)]
#[error("private kernal-api proc_exit sentinel ({0})")]
struct ProcExitSentinel(i32);

fn define_closed_imports(
    linker: &mut Linker<ThreadStoreState>,
) -> Result<(), SketchExecutionError> {
    linker
        .func_wrap(ABI_MODULE, ABI_YIELD, |caller: Caller<'_, ThreadStoreState>| {
            // The facade handle is deliberately supplied by the caller. This
            // slice has no scheduler yet, but must never construct a runtime.
            let _ = caller.data().runtime.clone();
            let _ = caller.data().controller.prelink.get();
        })
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;
    linker
        .func_wrap(
            THREAD_MODULE,
            THREAD_SPAWN,
            |_caller: Caller<'_, ThreadStoreState>, _arg: i32| -> i32 {
                // #35 replaces this pure deterministic rejection with the owned
                // child Store/Instance/native-thread bootstrap.
                ERRNO_THREAD_SPAWN_UNAVAILABLE
            },
        )
        .map_err(|_| SketchExecutionError::PrelinkFailed)?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |caller: Caller<'_, ThreadStoreState>,
             _id: i32,
             _precision: i64,
             output: i32|
             -> i32 {
                write_shared(&caller.data().controller.memory, output, &0_u64.to_le_bytes())
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
                let count =
                    write_shared(&caller.data().controller.memory, count, &0_u32.to_le_bytes());
                if count != ERRNO_SUCCESS {
                    return count;
                }
                write_shared(&caller.data().controller.memory, bytes, &0_u32.to_le_bytes())
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
                // Output is intentionally discarded. We only write a checked
                // zero-length result, never inspect untrusted iovecs.
                write_shared(&caller.data().controller.memory, written, &0_u32.to_le_bytes())
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

fn write_shared(memory: &SharedMemory, offset: i32, bytes: &[u8]) -> i32 {
    let Ok(offset) = usize::try_from(offset) else {
        return ERRNO_FAULT;
    };
    let Some(end) = offset.checked_add(bytes.len()) else {
        return ERRNO_FAULT;
    };
    let data = memory.data();
    let Some(cells) = data.get(offset..end) else {
        return ERRNO_FAULT;
    };
    for (cell, byte) in cells.iter().zip(bytes) {
        // Shared Wasm bytes may be concurrently accessed only atomically.
        unsafe { AtomicU8::from_ptr(cell.get()) }.store(*byte, Ordering::Relaxed);
    }
    ERRNO_SUCCESS
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SketchCompilerError {
    InvalidStackLimit,
    Unavailable,
}
impl fmt::Display for SketchCompilerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidStackLimit => "the Wasm stack limit must be nonzero",
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
) -> Result<SketchSharedMemory, SketchModuleError> {
    let mut types = Vec::<TypeEntry>::new();
    let mut functions = Vec::<u32>::new();
    let mut imported_function_types = Vec::<u32>::new();
    let mut memory = None;
    let mut memory_export = None;
    let mut exports = Vec::<(String, ExternalKind, u32)>::new();
    let mut start = None;
    let mut target_features = None;
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
    let allowed = [
        ("memory", ExternalKind::Memory),
        ("_start", ExternalKind::Func),
        ("__main_void", ExternalKind::Func),
        ("wasi_thread_start", ExternalKind::Func),
        (ENTRY, ExternalKind::Func),
    ];
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
    for (name, signature) in [
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
    ] {
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
