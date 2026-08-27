//! Versioned admission of untrusted Rust sketch modules.
//!
//! Wasmtime and all of its types remain implementation details. Admission is
//! intentionally stricter than successful Wasm validation: a module must use
//! the selected versioned kernel namespace and the two owned compatibility
//! imports needed by Rust's threaded WASI target. In particular, linking broad
//! WASI is never an admission shortcut.

use std::fmt;
use std::sync::Arc;

use wasmtime::{Config, Engine, ExternType, Module, Strategy, ValType};

const PAGE_BYTES: u64 = 64 * 1024;
const KERNEL_ABI_V1_NAMESPACE: &str = "kernal-api:v1";
const KERNEL_YIELD: &str = "kernel-yield";
const THREAD_NAMESPACE: &str = "wasi";
const THREAD_SPAWN: &str = "thread-spawn";
const MEMORY_NAMESPACE: &str = "env";
const MEMORY_NAME: &str = "memory";
const ENTRYPOINT_NAME: &str = "kernal-api-run";

/// Process-shareable compilation configuration for the selected sketch engine.
///
/// The configuration controls only compilation. Runtime resource accounting is
/// deliberately separate: Wasmtime documents that shared memories are not
/// integrated with ordinary store resource limiters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchEngineConfig {
    max_wasm_stack_bytes: usize,
}

impl SketchEngineConfig {
    /// Construct a compilation configuration with an explicit Wasm stack cap.
    pub fn new(max_wasm_stack_bytes: usize) -> Result<Self, SketchEngineError> {
        if max_wasm_stack_bytes == 0 {
            return Err(SketchEngineError::InvalidStackLimit);
        }
        Ok(Self {
            max_wasm_stack_bytes,
        })
    }

    /// Maximum stack consumed by Wasm frames on one guest thread.
    pub fn max_wasm_stack_bytes(self) -> usize {
        self.max_wasm_stack_bytes
    }
}

impl Default for SketchEngineConfig {
    fn default() -> Self {
        // This is a Wasm-frame limit, not a guarantee about the native thread
        // stack. Later thread creation chooses a larger native stack as needed.
        Self {
            max_wasm_stack_bytes: 2 * 1024 * 1024,
        }
    }
}

/// A private Wasmtime/Cranelift engine wrapped in facade-owned semantics.
#[derive(Clone)]
pub struct SketchEngine {
    inner: Arc<Engine>,
}

impl SketchEngine {
    /// Construct the sole core-Wasm/Cranelift engine profile for sketches.
    ///
    /// This intentionally excludes WASI linking, the Component Model, the
    /// pooling allocator, Winch, and any engine-managed task runtime.
    pub fn new(config: SketchEngineConfig) -> Result<Self, SketchEngineError> {
        let mut wasmtime = Config::new();
        wasmtime
            .strategy(Strategy::Cranelift)
            .map_err(|error| SketchEngineError::Initialization(error.to_string()))?;
        wasmtime.wasm_threads(true);
        // Shared memories are off by default in Wasmtime even when threads are
        // enabled, so make this policy choice explicit.
        wasmtime.shared_memory(true);
        wasmtime.wasm_memory64(false);
        wasmtime.wasm_multi_memory(false);
        wasmtime.wasm_shared_everything_threads(false);
        wasmtime.max_wasm_stack(config.max_wasm_stack_bytes);

        let engine = Engine::new(&wasmtime)
            .map_err(|error| SketchEngineError::Initialization(error.to_string()))?;
        Ok(Self {
            inner: Arc::new(engine),
        })
    }

    /// Compile and admit one module exactly once.
    ///
    /// The returned value retains the compiled module privately for a later
    /// instantiation slice. No store or instance is created here.
    pub fn admit(
        &self,
        bytes: &[u8],
        policy: SketchAdmissionPolicy,
    ) -> Result<SketchModule, SketchAdmissionError> {
        if bytes.len() > policy.max_module_bytes {
            return Err(SketchAdmissionError::ModuleTooLarge {
                actual_bytes: bytes.len(),
                maximum_bytes: policy.max_module_bytes,
            });
        }

        let module = Module::new(&self.inner, bytes)
            .map_err(|error| SketchAdmissionError::InvalidModule(error.to_string()))?;
        let shared_memory = validate_imports(&module, policy)?;
        validate_entrypoint(&module)?;

        Ok(SketchModule {
            module,
            module_bytes: bytes.len(),
            shared_memory,
        })
    }
}

/// Limits applied before a sketch module may be instantiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchAdmissionPolicy {
    max_module_bytes: usize,
    max_shared_memory_pages: u32,
}

impl SketchAdmissionPolicy {
    /// Construct the v1 admission policy.
    pub fn new(
        max_module_bytes: usize,
        max_shared_memory_pages: u32,
    ) -> Result<Self, SketchAdmissionError> {
        if max_module_bytes == 0 {
            return Err(SketchAdmissionError::InvalidModuleLimit);
        }
        if max_shared_memory_pages == 0 {
            return Err(SketchAdmissionError::InvalidSharedMemoryLimit);
        }
        Ok(Self {
            max_module_bytes,
            max_shared_memory_pages,
        })
    }

    /// Maximum accepted input bytes before compilation.
    pub fn max_module_bytes(self) -> usize {
        self.max_module_bytes
    }

    /// Maximum declared shared-memory pages (64 KiB each).
    pub fn max_shared_memory_pages(self) -> u32 {
        self.max_shared_memory_pages
    }
}

/// Metadata for a module admitted by [`SketchEngine`].
///
/// The compiled backend object remains private. Later internal runtime code can
/// use it for instantiation without leaking Wasmtime into the facade.
pub struct SketchModule {
    module: Module,
    module_bytes: usize,
    shared_memory: SketchSharedMemory,
}

impl SketchModule {
    /// Input byte length that was compiled exactly once for this admission.
    pub fn module_bytes(&self) -> usize {
        self.module_bytes
    }

    /// Validated shared-memory declaration for the future logical sketch.
    pub fn shared_memory(&self) -> SketchSharedMemory {
        self.shared_memory
    }

    #[allow(dead_code)]
    pub(crate) fn compiled_module(&self) -> &Module {
        &self.module
    }
}

/// Shared-memory bounds validated at sketch admission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SketchSharedMemory {
    minimum_pages: u32,
    maximum_pages: u32,
}

impl SketchSharedMemory {
    /// Minimum initially available memory in 64 KiB Wasm pages.
    pub fn minimum_pages(self) -> u32 {
        self.minimum_pages
    }

    /// Maximum memory in 64 KiB Wasm pages, bounded by kernel policy.
    pub fn maximum_pages(self) -> u32 {
        self.maximum_pages
    }

    /// Maximum memory as bytes, checked from the page bound.
    pub fn maximum_bytes(self) -> u64 {
        u64::from(self.maximum_pages) * PAGE_BYTES
    }
}

/// Failure while configuring the private sketch compiler.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchEngineError {
    /// A zero Wasm stack cap cannot execute any module safely.
    InvalidStackLimit,
    /// The selected engine could not be initialized on this host.
    Initialization(String),
}

impl fmt::Display for SketchEngineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidStackLimit => formatter.write_str("the Wasm stack limit must be nonzero"),
            Self::Initialization(detail) => write!(formatter, "could not initialize sketch engine: {detail}"),
        }
    }
}

impl std::error::Error for SketchEngineError {}

/// Failure while validating a module before any instance is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SketchAdmissionError {
    /// The caller exceeded its bounded module input budget.
    ModuleTooLarge {
        actual_bytes: usize,
        maximum_bytes: usize,
    },
    /// The module could not be parsed or compiled by the selected engine.
    InvalidModule(String),
    /// A policy with no module budget is invalid.
    InvalidModuleLimit,
    /// A policy with no shared-memory budget is invalid.
    InvalidSharedMemoryLimit,
    /// An import did not belong to the explicit kernel/compatibility allowlist.
    ForbiddenImport { module: String, name: String },
    /// An explicitly named import had the wrong external kind or ABI shape.
    ImportTypeMismatch { module: String, name: String },
    /// Exactly one owned shared-memory import is required.
    MissingSharedMemory,
    /// More than one memory import was requested.
    MultipleMemoryImports,
    /// The memory compatibility import must be a shared memory.
    UnsharedMemory,
    /// memory64 is prohibited in the v1 sketch ABI.
    Memory64,
    /// A shared memory must have an explicit finite maximum.
    SharedMemoryWithoutMaximum,
    /// The module's shared-memory declaration exceeds the kernel policy.
    SharedMemoryExceedsPolicy {
        minimum_pages: u64,
        maximum_pages: u64,
        policy_pages: u32,
    },
    /// The required versioned guest entrypoint is absent or has the wrong ABI.
    EntrypointMismatch,
}

impl fmt::Display for SketchAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleTooLarge {
                actual_bytes,
                maximum_bytes,
            } => write!(formatter, "sketch module is {actual_bytes} bytes; maximum is {maximum_bytes}"),
            Self::InvalidModule(detail) => write!(formatter, "invalid sketch module: {detail}"),
            Self::InvalidModuleLimit => formatter.write_str("the module input limit must be nonzero"),
            Self::InvalidSharedMemoryLimit => formatter.write_str("the shared-memory page limit must be nonzero"),
            Self::ForbiddenImport { module, name } => write!(formatter, "forbidden sketch import {module}::{name}"),
            Self::ImportTypeMismatch { module, name } => write!(formatter, "sketch import {module}::{name} has an unsupported ABI"),
            Self::MissingSharedMemory => formatter.write_str("sketch must import owned shared memory"),
            Self::MultipleMemoryImports => formatter.write_str("sketch may import exactly one shared memory"),
            Self::UnsharedMemory => formatter.write_str("sketch memory import must be shared"),
            Self::Memory64 => formatter.write_str("memory64 is not admitted for v1 sketches"),
            Self::SharedMemoryWithoutMaximum => formatter.write_str("sketch shared memory must declare a maximum"),
            Self::SharedMemoryExceedsPolicy {
                minimum_pages,
                maximum_pages,
                policy_pages,
            } => write!(formatter, "sketch shared memory {minimum_pages}..={maximum_pages} pages exceeds policy maximum {policy_pages}"),
            Self::EntrypointMismatch => formatter.write_str("sketch must export kernal-api-run with signature () -> ()"),
        }
    }
}

impl std::error::Error for SketchAdmissionError {}

fn validate_imports(
    module: &Module,
    policy: SketchAdmissionPolicy,
) -> Result<SketchSharedMemory, SketchAdmissionError> {
    let mut shared_memory = None;

    for import in module.imports() {
        let module_name = import.module();
        let name = import.name();
        let external_type = import.ty();

        if module_name == MEMORY_NAMESPACE && name == MEMORY_NAME {
            if shared_memory.is_some() {
                return Err(SketchAdmissionError::MultipleMemoryImports);
            }
            let memory = external_type.memory().ok_or_else(|| {
                SketchAdmissionError::ImportTypeMismatch {
                    module: module_name.to_owned(),
                    name: name.to_owned(),
                }
            })?;
            shared_memory = Some(validate_shared_memory(memory, policy)?);
            continue;
        }

        if module_name == THREAD_NAMESPACE && name == THREAD_SPAWN {
            if !matches_thread_spawn(&external_type) {
                return Err(SketchAdmissionError::ImportTypeMismatch {
                    module: module_name.to_owned(),
                    name: name.to_owned(),
                });
            }
            continue;
        }

        if module_name == KERNEL_ABI_V1_NAMESPACE && name == KERNEL_YIELD {
            // This is the one deliberately tiny v1 kernel import admitted by
            // the engine slice. #16 replaces this fixed contract with the
            // generated ABI manifest; an arbitrary name in this namespace is
            // no more trusted than an arbitrary WASI import.
            if !matches_empty_function(&external_type) {
                return Err(SketchAdmissionError::ImportTypeMismatch {
                    module: module_name.to_owned(),
                    name: name.to_owned(),
                });
            }
            continue;
        }

        return Err(SketchAdmissionError::ForbiddenImport {
            module: module_name.to_owned(),
            name: name.to_owned(),
        });
    }

    shared_memory.ok_or(SketchAdmissionError::MissingSharedMemory)
}

fn validate_shared_memory(
    memory: &wasmtime::MemoryType,
    policy: SketchAdmissionPolicy,
) -> Result<SketchSharedMemory, SketchAdmissionError> {
    if memory.is_64() {
        return Err(SketchAdmissionError::Memory64);
    }
    if !memory.is_shared() {
        return Err(SketchAdmissionError::UnsharedMemory);
    }
    let maximum_pages = memory
        .maximum()
        .ok_or(SketchAdmissionError::SharedMemoryWithoutMaximum)?;
    let minimum_pages = memory.minimum();
    if minimum_pages > u64::from(policy.max_shared_memory_pages)
        || maximum_pages > u64::from(policy.max_shared_memory_pages)
    {
        return Err(SketchAdmissionError::SharedMemoryExceedsPolicy {
            minimum_pages,
            maximum_pages,
            policy_pages: policy.max_shared_memory_pages,
        });
    }

    Ok(SketchSharedMemory {
        minimum_pages: minimum_pages as u32,
        maximum_pages: maximum_pages as u32,
    })
}

fn matches_thread_spawn(external_type: &ExternType) -> bool {
    let Some(function) = external_type.func() else {
        return false;
    };
    let params: Vec<_> = function.params().collect();
    let results: Vec<_> = function.results().collect();
    params.as_slice() == [ValType::I32] && results.as_slice() == [ValType::I32]
}

fn matches_empty_function(external_type: &ExternType) -> bool {
    let Some(function) = external_type.func() else {
        return false;
    };
    function.params().next().is_none() && function.results().next().is_none()
}

fn validate_entrypoint(module: &Module) -> Result<(), SketchAdmissionError> {
    let Some(export) = module.get_export(ENTRYPOINT_NAME) else {
        return Err(SketchAdmissionError::EntrypointMismatch);
    };
    let Some(function) = export.func() else {
        return Err(SketchAdmissionError::EntrypointMismatch);
    };
    if function.params().next().is_some() || function.results().next().is_some() {
        return Err(SketchAdmissionError::EntrypointMismatch);
    }
    Ok(())
}
