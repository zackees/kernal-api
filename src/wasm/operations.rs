//! Logical-sketch operation and resource ownership.
//!
//! This is deliberately independent of Wasmtime.  A callback can retain this
//! hub and wake a waiter, but it can never retain or re-enter a Store.

use crate::async_engine::Notify;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
#[cfg(feature = "wasm-sketch-host")]
use std::fs::{self, File};
#[cfg(feature = "wasm-sketch-host")]
use std::io::Write;
#[cfg(feature = "wasm-sketch-host")]
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static NEXT_OPAQUE_TOKEN: AtomicU64 = AtomicU64::new(1);
static NEXT_LOGICAL_SCOPE: AtomicU64 = AtomicU64::new(1);
const MAX_CLOSED_TOMBSTONES: usize = 128;
const MAX_WIRE_TOKEN: u64 = (1_u64 << 56) - 1;

pub(crate) const OP_SYNTHETIC_YIELD: u32 = 1;
pub(crate) const OP_SYNTHETIC_RESOURCE_CREATE: u32 = 2;
pub(crate) const OP_SYNTHETIC_RESOURCE_USE: u32 = 3;
pub(crate) const OP_SYNTHETIC_RESOURCE_CLOSE: u32 = 4;
pub(crate) const OP_BLOB_CREATE: u32 = 5;
pub(crate) const OP_BLOB_WRITE: u32 = 6;
pub(crate) const OP_BLOB_READ: u32 = 7;
pub(crate) const OP_BLOB_READ_COLLECT: u32 = 8;
pub(crate) const OP_BLOB_SEAL: u32 = 9;
pub(crate) const OP_OUTPUT_COMMIT: u32 = 10;
pub(crate) const OP_OUTPUT_GRANT: u32 = 11;
const SYNTHETIC_RESOURCE_KIND: u8 = 1;
pub(crate) const EXTERNAL_WEBVIEW_RESOURCE_KIND: u8 = 2;
const EXTERNAL_WEBVIEW_RIGHT_LOAD: u8 = 0b01;
const BLOB_RESOURCE_KIND: u8 = 3;
const BLOB_RIGHT_READ: u8 = 0b01;
const BLOB_RIGHT_WRITE: u8 = 0b10;
const OUTPUT_RESOURCE_KIND: u8 = 4;
const OUTPUT_RIGHT_COMMIT: u8 = 0b01;
const STATUS_PENDING: u8 = 0;
const STATUS_COMPLETED: u8 = 1;
const STATUS_CANCELLED: u8 = 2;
const STATUS_TIMED_OUT: u8 = 3;
const STATUS_TRAPPED: u8 = 4;
const STATUS_OWNER_EXITED: u8 = 5;
const STATUS_CLOSED: u8 = 6;
const STATUS_REJECTED: u8 = 7;
const STATUS_ERROR: u8 = 0x80;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct OpaqueToken(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Terminal {
    Completed,
    Cancelled,
    TimedOut,
    Trapped,
    OwnerExited,
    Closed,
    Rejected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TerminalResult {
    pub(super) terminal: Terminal,
    pub(super) resource: Option<OpaqueToken>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HubError {
    Quota,
    Invalid,
    Stale,
    WrongKind,
    WrongRights,
    Closed,
    Exhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HubSnapshot {
    pub(crate) scope: u64,
    pub(crate) pending_operations: usize,
    pub(crate) live_resources: usize,
    pub(crate) suspends: u64,
    pub(crate) resumes: u64,
    pub(crate) buffered_blob_bytes: usize,
    pub(crate) peak_buffered_blob_bytes: usize,
    pub(crate) pending_write_bytes: usize,
    pub(crate) completed_read_bytes: usize,
    /// Actual buffer capacities currently retained by the hub, including
    /// unused blob capacity, pending inputs, and uncollected read results.
    pub(crate) retained_transfer_capacity: usize,
}

/// Private limits for the opaque bulk-data boundary.  They deliberately live
/// beside operation/resource authority: a producer cannot bypass accounting
/// by choosing a different host queue.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct BlobLimits {
    pub(crate) maximum_chunk_bytes: usize,
    pub(crate) maximum_blob_bytes: usize,
    pub(crate) maximum_sketch_bytes: usize,
}

impl BlobLimits {
    pub(crate) const fn new(
        maximum_chunk_bytes: usize,
        maximum_blob_bytes: usize,
        maximum_sketch_bytes: usize,
    ) -> Result<Self, HubError> {
        if maximum_chunk_bytes == 0
            || maximum_blob_bytes < maximum_chunk_bytes
            || maximum_sketch_bytes < maximum_blob_bytes
        {
            return Err(HubError::Quota);
        }
        Ok(Self {
            maximum_chunk_bytes,
            maximum_blob_bytes,
            maximum_sketch_bytes,
        })
    }
}

const DEFAULT_BLOB_LIMITS: BlobLimits = BlobLimits {
    maximum_chunk_bytes: 64 * 1024,
    maximum_blob_bytes: 1024 * 1024,
    maximum_sketch_bytes: 4 * 1024 * 1024,
};

/// Private typed requests shared by the generated ABI and heavyweight native
/// backends.  It is intentionally closed: adding authority means adding a
/// request here, rather than a second registry or scheduler.
pub(crate) enum Request {
    BlobCreate,
    SyntheticYield,
    SyntheticCreate {
        kind: u8,
        rights: u8,
        shareable: bool,
    },
    SyntheticUse {
        resource: OpaqueToken,
        kind: u8,
        rights: u8,
    },
    Close {
        resource: OpaqueToken,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ResourceIdentity {
    scope: u64,
    slot: u32,
    generation: u32,
    kind: u8,
    rights: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Owner {
    // An owner is an authorized Store/thread instance in this logical sketch.
    // It is not a Wasmtime identity and never leaves the private dispatcher.
    store: u64,
}

struct ResourceSlot {
    identity: ResourceIdentity,
    owner: Owner,
    shareable: bool,
    value: ResourceValue,
    reserved: bool,
    committing: bool,
}

enum ResourceValue {
    Synthetic,
    ExternalWebview,
    Blob {
        buffer: VecDeque<u8>,
        sealed: bool,
    },
    #[cfg(feature = "wasm-sketch-host")]
    ExactOutput(PathBuf),
}

struct OperationSlot {
    owner: Owner,
    resource: Option<OpaqueToken>,
    terminal: Option<TerminalResult>,
    // A generated close must not complete before its guest future has parked.
    // The scheduler may run its detached task before the synchronous import
    // returns, so retain the authority-free completion until `suspend` has
    // registered the guest waiter.
    deferred_completion: Option<DeferredCompletion>,
    suspended: bool,
    notify: Arc<Notify>,
    created_resource: Option<OpaqueToken>,
    pending_blob_write: Option<Vec<u8>>,
    pending_blob_read: Option<usize>,
    is_blob_read: bool,
    blob_read_result: Option<Vec<u8>>,
}

#[derive(Clone, Copy)]
struct DeferredCompletion {
    result: TerminalResult,
    revoke: Option<OpaqueToken>,
}

struct State {
    resources: BTreeMap<OpaqueToken, ResourceSlot>,
    operations: BTreeMap<OpaqueToken, OperationSlot>,
    next_resource_slot: u32,
    resource_generations: BTreeMap<u32, u32>,
    free_resource_slots: Vec<u32>,
    closed_resources: BTreeSet<OpaqueToken>,
    closed_resource_order: VecDeque<OpaqueToken>,
    suspends: u64,
    resumes: u64,
    buffered_blob_bytes: usize,
    peak_buffered_blob_bytes: usize,
    closed: bool,
}

/// Private logical authority shared only by explicitly authorized instances.
pub(crate) struct OperationHub {
    scope: u64,
    maximum_operations: usize,
    maximum_resources: usize,
    blob_limits: BlobLimits,
    state: Mutex<State>,
}

impl OperationHub {
    pub(crate) fn new(
        maximum_operations: usize,
        maximum_resources: usize,
    ) -> Result<Arc<Self>, HubError> {
        Self::new_with_blob_limits(maximum_operations, maximum_resources, DEFAULT_BLOB_LIMITS)
    }

    fn new_with_blob_limits(
        maximum_operations: usize,
        maximum_resources: usize,
        blob_limits: BlobLimits,
    ) -> Result<Arc<Self>, HubError> {
        let scope = next(&NEXT_LOGICAL_SCOPE)?;
        Ok(Arc::new(Self {
            scope,
            maximum_operations,
            maximum_resources,
            blob_limits,
            state: Mutex::new(State {
                resources: BTreeMap::new(),
                operations: BTreeMap::new(),
                next_resource_slot: 0,
                resource_generations: BTreeMap::new(),
                free_resource_slots: Vec::new(),
                closed_resources: BTreeSet::new(),
                closed_resource_order: VecDeque::new(),
                suspends: 0,
                resumes: 0,
                buffered_blob_bytes: 0,
                peak_buffered_blob_bytes: 0,
                closed: false,
            }),
        }))
    }

    /// Tighten the default bulk limits for a logical sketch before it has any
    /// resources.  This makes test/embedding quotas explicit without adding a
    /// public storage abstraction or a second resource table.
    pub(crate) fn with_blob_limits(
        maximum_operations: usize,
        maximum_resources: usize,
        limits: BlobLimits,
    ) -> Result<Arc<Self>, HubError> {
        Self::new_with_blob_limits(maximum_operations, maximum_resources, limits)
    }

    /// The sole scheduling entry point.  Both generated guest imports and a
    /// native backend submit this closed request type through the caller's
    /// RuntimeHandle; neither path can create an ambient runtime or bypass
    /// quota/identity accounting.
    pub(crate) fn dispatch(
        self: &Arc<Self>,
        runtime: crate::async_engine::RuntimeHandle,
        store: u64,
        request: Request,
    ) -> Result<OpaqueToken, HubError> {
        let (resource, created, close_after, kind, rights, completion) = match request {
            Request::BlobCreate => {
                let resource = self.create_resource_value(
                    store,
                    BLOB_RESOURCE_KIND,
                    BLOB_RIGHT_READ | BLOB_RIGHT_WRITE,
                    false,
                    ResourceValue::Blob {
                        buffer: VecDeque::new(),
                        sealed: false,
                    },
                )?;
                (
                    None,
                    true,
                    None,
                    BLOB_RESOURCE_KIND,
                    BLOB_RIGHT_READ | BLOB_RIGHT_WRITE,
                    TerminalResult {
                        terminal: Terminal::Completed,
                        resource: Some(resource),
                    },
                )
            }
            Request::SyntheticYield => (
                None,
                false,
                None,
                0,
                0,
                TerminalResult {
                    terminal: Terminal::Completed,
                    resource: None,
                },
            ),
            Request::SyntheticCreate {
                kind,
                rights,
                shareable,
            } => {
                // Reserve the resource before scheduling.  If operation
                // reservation fails, close it again below.
                let resource = self.create_resource(store, kind, rights, shareable)?;
                (
                    None,
                    true,
                    None,
                    kind,
                    rights,
                    TerminalResult {
                        terminal: Terminal::Completed,
                        resource: Some(resource),
                    },
                )
            }
            Request::SyntheticUse {
                resource,
                kind,
                rights,
            } => (
                Some(resource),
                false,
                None,
                kind,
                rights,
                TerminalResult {
                    terminal: Terminal::Completed,
                    resource: None,
                },
            ),
            Request::Close { resource } => {
                self.validate_close(store, resource)?;
                (
                    None,
                    false,
                    Some(resource),
                    0,
                    0,
                    TerminalResult {
                        // Closing is a successful operation. The resource
                        // becomes closed as its side effect; reporting that
                        // resource state as this operation's terminal error
                        // would make the generated semantic close future
                        // unusable.
                        terminal: Terminal::Completed,
                        resource: None,
                    },
                )
            }
        };
        let operation = match self.submit(store, resource, kind, rights) {
            Ok((operation, _)) => operation,
            Err(error) => {
                if created {
                    let resource = completion
                        .resource
                        .expect("create completion owns reservation");
                    let _ = self.close_resource(resource);
                }
                return Err(error);
            }
        };
        if created {
            self.attach_created_resource(
                operation,
                completion
                    .resource
                    .expect("create completion owns resource"),
            )?;
        }
        let hub = Arc::clone(self);
        runtime
            .launch(async move {
                // This is the deliberately authority-free #37 synthetic
                // operation. Real native work supplies the same terminal
                // handoff after its callback, never a Store callback.
                crate::async_engine::yield_now().await;
                let _ = hub.complete_after_suspend(operation, completion, close_after);
            })
            .detach();
        Ok(operation)
    }

    /// Reserve a generation-safe external-webview resource and its open
    /// operation in the sole hub.  The caller must later publish either a
    /// terminal outcome or a successful resource activation from its native
    /// callback; it may never retain a Wasmtime Store.
    pub(crate) fn begin_external_webview_open(
        &self,
        store: u64,
    ) -> Result<(OpaqueToken, OpaqueToken), HubError> {
        let resource = self.create_resource_value(
            store,
            EXTERNAL_WEBVIEW_RESOURCE_KIND,
            EXTERNAL_WEBVIEW_RIGHT_LOAD,
            false,
            ResourceValue::ExternalWebview,
        )?;
        let operation = match self.submit(store, None, 0, 0) {
            Ok((operation, _)) => operation,
            Err(error) => {
                let _ = self.close_resource(resource);
                return Err(error);
            }
        };
        self.attach_created_resource(operation, resource)?;
        Ok((resource, operation))
    }

    /// Submit a load observation operation for an already activated webview.
    pub(crate) fn begin_external_webview_wait(
        &self,
        store: u64,
        resource: OpaqueToken,
    ) -> Result<OpaqueToken, HubError> {
        match self.submit(
            store,
            Some(resource),
            EXTERNAL_WEBVIEW_RESOURCE_KIND,
            EXTERNAL_WEBVIEW_RIGHT_LOAD,
        ) {
            Ok((operation, _)) => Ok(operation),
            // A revoked external handle has a bounded tombstone. Preserve
            // that semantic distinction for the facade instead of treating
            // an immediately repeated use as a malformed host request.
            Err(HubError::Invalid) if self.resource_was_closed(resource) => Err(HubError::Closed),
            Err(error) => Err(error),
        }
    }

    /// Validate and reserve a semantic close operation.  Physical teardown is
    /// performed by the private native backend; this hub remains the sole
    /// authority that revokes the resource generation.
    pub(crate) fn begin_external_webview_close(
        &self,
        store: u64,
        resource: OpaqueToken,
    ) -> Result<OpaqueToken, HubError> {
        self.validate_close(store, resource)?;
        self.submit(store, Some(resource), EXTERNAL_WEBVIEW_RESOURCE_KIND, 0)
            .map(|(operation, _)| operation)
    }

    pub(crate) fn finish_external_operation(&self, operation: OpaqueToken, terminal: Terminal) {
        let _ = self.terminal(
            operation,
            TerminalResult {
                terminal,
                resource: None,
            },
        );
    }

    pub(crate) fn finish_external_open(&self, operation: OpaqueToken, resource: OpaqueToken) {
        let _ = self.terminal(
            operation,
            TerminalResult {
                terminal: Terminal::Completed,
                resource: Some(resource),
            },
        );
    }

    pub(crate) fn observe_terminal(
        &self,
        store: u64,
        operation: OpaqueToken,
    ) -> Result<Option<TerminalResult>, HubError> {
        self.take_terminal(operation, store)
    }

    pub(crate) fn wait_external_operation(
        &self,
        store: u64,
        operation: OpaqueToken,
    ) -> Result<Arc<Notify>, HubError> {
        self.suspend(operation, store)
    }

    /// Generated scalar ABI adapter.  `kind` is a closed opcode; `arg0` and
    /// `arg1` never carry a pointer or a host object.  Resource create/use/
    /// close are operation variants, not additional imports.
    pub(crate) fn submit_wire(
        self: &Arc<Self>,
        runtime: crate::async_engine::RuntimeHandle,
        store: u64,
        kind: u32,
        arg0: u64,
        arg1: u64,
    ) -> Result<u64, HubError> {
        #[cfg(feature = "wasm-sketch-host")]
        if kind == OP_OUTPUT_COMMIT {
            return self
                .submit_output_commit(runtime, store, OpaqueToken(arg0), OpaqueToken(arg1))
                .map(|operation| operation.0);
        }
        if kind == OP_BLOB_SEAL {
            if arg1 != 0 {
                return Err(HubError::Invalid);
            }
            let blob = OpaqueToken(arg0);
            let (operation, _) =
                self.submit(store, Some(blob), BLOB_RESOURCE_KIND, BLOB_RIGHT_WRITE)?;
            let terminal = if self.seal_blob(store, blob).is_ok() {
                Terminal::Completed
            } else {
                Terminal::Rejected
            };
            self.terminal(
                operation,
                TerminalResult {
                    terminal,
                    resource: None,
                },
            )?;
            return Ok(operation.0);
        }
        if kind == OP_BLOB_READ {
            return self
                .submit_blob_read(
                    store,
                    OpaqueToken(arg0),
                    usize::try_from(arg1).map_err(|_| HubError::Quota)?,
                )
                .map(|token| token.0);
        }
        let request = match kind {
            OP_BLOB_CREATE if arg0 == 0 && arg1 == 0 => Request::BlobCreate,
            OP_SYNTHETIC_YIELD => Request::SyntheticYield,
            OP_SYNTHETIC_RESOURCE_CREATE => Request::SyntheticCreate {
                kind: SYNTHETIC_RESOURCE_KIND,
                rights: u8::try_from(arg1).map_err(|_| HubError::WrongRights)?,
                shareable: match arg0 {
                    0 => false,
                    1 => true,
                    _ => return Err(HubError::Invalid),
                },
            },
            OP_SYNTHETIC_RESOURCE_USE => Request::SyntheticUse {
                resource: OpaqueToken(arg0),
                kind: SYNTHETIC_RESOURCE_KIND,
                rights: u8::try_from(arg1).map_err(|_| HubError::WrongRights)?,
            },
            OP_SYNTHETIC_RESOURCE_CLOSE => Request::Close {
                resource: OpaqueToken(arg0),
            },
            _ => return Err(HubError::Invalid),
        };
        Ok(self.dispatch(runtime, store, request)?.0)
    }

    pub(crate) fn poll_wire(&self, store: u64, operation: u64) -> u64 {
        // Read completion must be collected with its bytes, never consumed
        // through the payload-free lifecycle poll.
        if self.state.lock().is_ok_and(|state| {
            state
                .operations
                .get(&OpaqueToken(operation))
                .is_some_and(|op| op.is_blob_read)
        }) {
            return pack(STATUS_ERROR, None);
        }
        match self.take_terminal(OpaqueToken(operation), store) {
            Ok(None) => pack(STATUS_PENDING, None),
            Ok(Some(result)) => pack(status(result.terminal), result.resource),
            Err(_) => pack(STATUS_ERROR, None),
        }
    }

    pub(crate) fn suspend_wire(&self, store: u64, operation: u64) -> Result<Arc<Notify>, HubError> {
        self.suspend(OpaqueToken(operation), store)
    }

    pub(crate) fn cancel_wire(&self, store: u64, operation: u64) -> Result<(), HubError> {
        self.take_owner(OpaqueToken(operation), store)?;
        self.terminal(
            OpaqueToken(operation),
            TerminalResult {
                terminal: Terminal::Cancelled,
                resource: None,
            },
        )
    }

    fn validate_close(&self, store: u64, resource: OpaqueToken) -> Result<(), HubError> {
        let state = self.state.lock().map_err(|_| HubError::Closed)?;
        let slot = state.resources.get(&resource).ok_or(HubError::Invalid)?;
        if slot.owner.store != store && !slot.shareable {
            return Err(HubError::WrongRights);
        }
        Ok(())
    }

    fn resource_was_closed(&self, resource: OpaqueToken) -> bool {
        self.state
            .lock()
            .is_ok_and(|state| state.closed_resources.contains(&resource))
    }

    fn take_owner(&self, operation: OpaqueToken, store: u64) -> Result<(), HubError> {
        let state = self.state.lock().map_err(|_| HubError::Closed)?;
        let slot = state.operations.get(&operation).ok_or(HubError::Invalid)?;
        if slot.owner.store != store {
            return Err(HubError::Stale);
        }
        Ok(())
    }

    fn attach_created_resource(
        &self,
        operation: OpaqueToken,
        resource: OpaqueToken,
    ) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        state
            .operations
            .get_mut(&operation)
            .ok_or(HubError::Invalid)?
            .created_resource = Some(resource);
        Ok(())
    }

    #[cfg(test)]
    fn activate_for_test(&self, resource: OpaqueToken) {
        self.state
            .lock()
            .unwrap()
            .resources
            .get_mut(&resource)
            .unwrap()
            .reserved = false;
    }

    pub(super) fn create_resource(
        &self,
        store: u64,
        kind: u8,
        rights: u8,
        shareable: bool,
    ) -> Result<OpaqueToken, HubError> {
        self.create_resource_value(store, kind, rights, shareable, ResourceValue::Synthetic)
    }

    /// Grant an opaque, bidirectional in-memory blob to one logical sketch.
    /// The buffer is host-owned; callers can only append or pull bounded
    /// chunks, so it cannot become a disguised whole-value ABI transport.
    pub(crate) fn create_blob(&self, store: u64) -> Result<OpaqueToken, HubError> {
        let token = self.create_resource_value(
            store,
            BLOB_RESOURCE_KIND,
            BLOB_RIGHT_READ | BLOB_RIGHT_WRITE,
            false,
            ResourceValue::Blob {
                buffer: VecDeque::new(),
                sealed: false,
            },
        )?;
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        state
            .resources
            .get_mut(&token)
            .ok_or(HubError::Invalid)?
            .reserved = false;
        Ok(token)
    }

    /// Append exactly one quota-accounted chunk. A full blob or sketch budget
    /// rejects before copying, which is the synchronous reservation half of
    /// the generated capacity-awaited write operation.
    pub(crate) fn blob_write(
        &self,
        store: u64,
        blob: OpaqueToken,
        bytes: &[u8],
    ) -> Result<usize, HubError> {
        if bytes.len() > self.blob_limits.maximum_chunk_bytes {
            return Err(HubError::Quota);
        }
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.buffered_blob_bytes.saturating_add(bytes.len())
            > self.blob_limits.maximum_sketch_bytes
        {
            return Err(HubError::Quota);
        }
        let resource = state.resources.get_mut(&blob).ok_or(HubError::Invalid)?;
        Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_WRITE)?;
        let ResourceValue::Blob { buffer, sealed } = &mut resource.value else {
            return Err(HubError::WrongKind);
        };
        if *sealed {
            return Err(HubError::Closed);
        }
        if buffer.len().saturating_add(bytes.len()) > self.blob_limits.maximum_blob_bytes {
            return Err(HubError::Quota);
        }
        Self::reserve_blob_capacity(
            &mut state,
            blob,
            bytes.len(),
            self.blob_limits.maximum_sketch_bytes,
        )?;
        if let ResourceValue::Blob { buffer, .. } = &mut state
            .resources
            .get_mut(&blob)
            .ok_or(HubError::Invalid)?
            .value
        {
            buffer.extend(bytes);
        }
        state.buffered_blob_bytes += bytes.len();
        state.peak_buffered_blob_bytes = state
            .peak_buffered_blob_bytes
            .max(state.buffered_blob_bytes);
        drop(state);
        self.drive_blob_reads()?;
        Ok(bytes.len())
    }

    /// Pull at most one configured chunk. Nothing is copied or produced until
    /// the logical guest explicitly asks, and capacity is released before the
    /// next producer attempt observes it.
    pub(crate) fn blob_read(
        &self,
        store: u64,
        blob: OpaqueToken,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, HubError> {
        self.read_blob_chunk(store, blob, maximum_bytes, false)
    }

    fn read_blob_chunk(
        &self,
        store: u64,
        blob: OpaqueToken,
        maximum_bytes: usize,
        committing: bool,
    ) -> Result<Vec<u8>, HubError> {
        if maximum_bytes > self.blob_limits.maximum_chunk_bytes {
            return Err(HubError::Quota);
        }
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let resource = state.resources.get_mut(&blob).ok_or(HubError::Invalid)?;
        if resource.committing != committing {
            return Err(HubError::WrongRights);
        }
        Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)?;
        let ResourceValue::Blob { buffer, .. } = &mut resource.value else {
            return Err(HubError::WrongKind);
        };
        let count = maximum_bytes.min(buffer.len());
        let result: Vec<_> = buffer.drain(..count).collect();
        if buffer.is_empty() {
            // An empty live resource must not retain a formerly full buffer
            // while its length-based byte ledger reports zero.
            *buffer = VecDeque::new();
        }
        state.buffered_blob_bytes = state.buffered_blob_bytes.saturating_sub(count);
        drop(state);
        self.drive_blob_writes()?;
        Ok(result)
    }

    /// Reserve a capacity-awaited write in the shared operation table.
    pub(crate) fn submit_blob_write(
        &self,
        store: u64,
        blob: OpaqueToken,
        bytes: &[u8],
    ) -> Result<OpaqueToken, HubError> {
        self.submit_blob_write_from(store, blob, bytes.len(), || bytes.to_vec())
    }

    pub(crate) fn submit_blob_write_wire(
        &self,
        store: u64,
        blob: u64,
        length: usize,
        copy: impl FnOnce() -> Vec<u8>,
    ) -> Result<u64, HubError> {
        self.submit_blob_write_from(store, OpaqueToken(blob), length, copy)
            .map(|token| token.0)
    }

    fn submit_blob_write_from(
        &self,
        store: u64,
        blob: OpaqueToken,
        length: usize,
        copy: impl FnOnce() -> Vec<u8>,
    ) -> Result<OpaqueToken, HubError> {
        if length > self.blob_limits.maximum_chunk_bytes {
            return Err(HubError::Quota);
        }
        let (operation, _) =
            self.submit(store, Some(blob), BLOB_RESOURCE_KIND, BLOB_RIGHT_WRITE)?;
        {
            let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
            let pending: usize = state
                .operations
                .values()
                .filter_map(|op| op.pending_blob_write.as_ref())
                .map(Vec::len)
                .sum();
            if pending.saturating_add(length) > self.blob_limits.maximum_sketch_bytes {
                state.operations.remove(&operation);
                return Err(HubError::Quota);
            }
            let slot = state
                .operations
                .get_mut(&operation)
                .ok_or(HubError::Invalid)?;
            if slot.terminal.is_none() {
                // Copy only after authority and capacity checks, without
                // retaining the producer or guest-memory view in the hub.
                slot.pending_blob_write = Some(copy());
            }
        }
        self.drive_blob_writes()?;
        Ok(operation)
    }

    fn reserve_blob_capacity(
        state: &mut State,
        blob: OpaqueToken,
        additional: usize,
        maximum: usize,
    ) -> Result<(), HubError> {
        let retained = state.resources.values().fold(0_usize, |total, resource| {
            total.saturating_add(match &resource.value {
                ResourceValue::Blob { buffer, .. } => buffer.capacity(),
                _ => 0,
            })
        });
        let ResourceValue::Blob { buffer, .. } = &mut state
            .resources
            .get_mut(&blob)
            .ok_or(HubError::Invalid)?
            .value
        else {
            return Err(HubError::WrongKind);
        };
        let required = buffer
            .len()
            .checked_add(additional)
            .ok_or(HubError::Quota)?;
        let growth = required.saturating_sub(buffer.capacity());
        if retained.saturating_add(growth) > maximum {
            return Err(HubError::Quota);
        }
        // Avoid VecDeque's geometric growth retaining uncharged spare bytes.
        buffer
            .try_reserve_exact(additional)
            .map_err(|_| HubError::Exhausted)
    }

    fn drive_blob_writes(&self) -> Result<(), HubError> {
        // Each successful pass terminalizes at least one operation. Revisit
        // writes after reads release capacity, without recursive pumping.
        while self.drive_blob_write_pass()? {}
        Ok(())
    }

    fn drive_blob_write_pass(&self) -> Result<bool, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let tokens: Vec<_> = state
            .operations
            .iter()
            .filter(|(_, op)| op.pending_blob_write.is_some() && op.terminal.is_none())
            .map(|(token, _)| *token)
            .collect();
        let mut wakes = Vec::new();
        for token in tokens {
            let operation = &state.operations[&token];
            let resource = operation.resource.ok_or(HubError::Invalid)?;
            let count = operation
                .pending_blob_write
                .as_ref()
                .ok_or(HubError::Invalid)?
                .len();
            let mut terminal = match state.resources.get(&resource).map(|slot| &slot.value) {
                Some(ResourceValue::Blob { sealed: true, .. }) | None => Terminal::Closed,
                Some(ResourceValue::Blob { buffer, .. }) => {
                    if buffer.len().saturating_add(count) > self.blob_limits.maximum_blob_bytes
                        || state.buffered_blob_bytes.saturating_add(count)
                            > self.blob_limits.maximum_sketch_bytes
                    {
                        continue;
                    }
                    Terminal::Completed
                }
                _ => Terminal::Rejected,
            };
            if terminal == Terminal::Completed {
                match Self::reserve_blob_capacity(
                    &mut state,
                    resource,
                    count,
                    self.blob_limits.maximum_sketch_bytes,
                ) {
                    Ok(()) => {}
                    Err(HubError::Quota) => continue,
                    Err(_) => terminal = Terminal::Rejected,
                }
            }
            if terminal == Terminal::Completed {
                let bytes = state
                    .operations
                    .get_mut(&token)
                    .unwrap()
                    .pending_blob_write
                    .take()
                    .unwrap();
                if let ResourceValue::Blob { buffer, .. } =
                    &mut state.resources.get_mut(&resource).unwrap().value
                {
                    buffer.extend(bytes);
                }
                state.buffered_blob_bytes += count;
                state.peak_buffered_blob_bytes = state
                    .peak_buffered_blob_bytes
                    .max(state.buffered_blob_bytes);
            }
            if let Some(wake) = Self::terminal_locked(
                &mut state,
                token,
                TerminalResult {
                    terminal,
                    resource: None,
                },
            )? {
                wakes.push(wake);
            }
        }
        drop(state);
        let progressed = !wakes.is_empty();
        for wake in wakes {
            wake.notify_one();
        }
        Ok(self.drive_blob_read_pass()? || progressed)
    }

    pub(crate) fn submit_blob_read(
        &self,
        store: u64,
        blob: OpaqueToken,
        maximum: usize,
    ) -> Result<OpaqueToken, HubError> {
        if maximum == 0 || maximum > self.blob_limits.maximum_chunk_bytes {
            return Err(HubError::Quota);
        }
        let (token, _) = self.submit(store, Some(blob), BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)?;
        {
            let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
            let operation = state.operations.get_mut(&token).ok_or(HubError::Invalid)?;
            if operation.terminal.is_none() {
                operation.pending_blob_read = Some(maximum);
            }
            operation.is_blob_read = true;
        }
        self.drive_blob_reads()?;
        self.drive_blob_writes()?;
        Ok(token)
    }

    fn drive_blob_reads(&self) -> Result<(), HubError> {
        self.drive_blob_read_pass().map(|_| ())
    }

    fn drive_blob_read_pass(&self) -> Result<bool, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let tokens: Vec<_> = state
            .operations
            .iter()
            .filter(|(_, op)| op.pending_blob_read.is_some() && op.terminal.is_none())
            .map(|(token, _)| *token)
            .collect();
        let mut wakes = Vec::new();
        for token in tokens {
            let operation = &state.operations[&token];
            let resource = operation.resource.ok_or(HubError::Invalid)?;
            let maximum = operation.pending_blob_read.ok_or(HubError::Invalid)?;
            let retained: usize = state
                .operations
                .values()
                .filter_map(|op| op.blob_read_result.as_ref())
                .map(Vec::len)
                .sum();
            let Some(ResourceSlot {
                value: ResourceValue::Blob { buffer, sealed },
                ..
            }) = state.resources.get_mut(&resource)
            else {
                continue;
            };
            if buffer.is_empty() && !*sealed {
                continue;
            }
            let count = maximum.min(buffer.len());
            if retained.saturating_add(count) > self.blob_limits.maximum_sketch_bytes {
                continue;
            }
            let bytes = buffer.drain(..count).collect();
            if buffer.is_empty() {
                *buffer = VecDeque::new();
            }
            state.buffered_blob_bytes -= count;
            state
                .operations
                .get_mut(&token)
                .ok_or(HubError::Invalid)?
                .blob_read_result = Some(bytes);
            if let Some(wake) = Self::terminal_locked(
                &mut state,
                token,
                TerminalResult {
                    terminal: Terminal::Completed,
                    resource: None,
                },
            )? {
                wakes.push(wake);
            }
        }
        drop(state);
        let progressed = !wakes.is_empty();
        for wake in wakes {
            wake.notify_one();
        }
        Ok(progressed)
    }

    /// Copy a terminal bounded result during the collecting import. Invalid
    /// destinations and wrong owners leave the operation available to retry.
    pub(crate) fn collect_blob_read_wire(
        &self,
        store: u64,
        token: u64,
        capacity: usize,
        copy: impl FnOnce(&[u8]),
    ) -> Result<u64, HubError> {
        let token = OpaqueToken(token);
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed {
            return Err(HubError::Closed);
        }
        let operation = state.operations.get(&token).ok_or(HubError::Invalid)?;
        if operation.owner.store != store {
            return Err(HubError::Stale);
        }
        if !operation.is_blob_read {
            return Err(HubError::WrongKind);
        }
        let Some(result) = operation.terminal else {
            return Ok(0);
        };
        let bytes = operation.blob_read_result.as_deref().unwrap_or_default();
        if capacity < bytes.len() {
            return Err(HubError::Quota);
        }
        let length = bytes.len();
        if result.terminal == Terminal::Completed {
            copy(bytes);
        }
        let packed = (length as u64) << 8 | u64::from(status(result.terminal));
        state.operations.remove(&token);
        state.resumes = state.resumes.saturating_add(1);
        drop(state);
        self.drive_blob_writes()?;
        self.drive_blob_reads()?;
        Ok(packed)
    }

    pub(crate) fn take_blob_read(
        &self,
        store: u64,
        token: OpaqueToken,
    ) -> Result<Option<(Terminal, Vec<u8>)>, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed {
            return Err(HubError::Closed);
        }
        let operation = state.operations.get(&token).ok_or(HubError::Invalid)?;
        if operation.owner.store != store {
            return Err(HubError::Stale);
        }
        if !operation.is_blob_read {
            return Err(HubError::WrongKind);
        }
        let Some(result) = operation.terminal else {
            return Ok(None);
        };
        let operation = state.operations.remove(&token).ok_or(HubError::Invalid)?;
        state.resumes = state.resumes.saturating_add(1);
        drop(state);
        self.drive_blob_writes()?;
        self.drive_blob_reads()?;
        Ok(Some((
            result.terminal,
            operation.blob_read_result.unwrap_or_default(),
        )))
    }

    /// Publish EOF explicitly. An empty, unsealed blob can still receive data.
    pub(crate) fn seal_blob(&self, store: u64, blob: OpaqueToken) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let resource = state.resources.get_mut(&blob).ok_or(HubError::Invalid)?;
        Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_WRITE)?;
        let ResourceValue::Blob { sealed, .. } = &mut resource.value else {
            return Err(HubError::WrongKind);
        };
        *sealed = true;
        drop(state);
        self.drive_blob_writes()?;
        self.drive_blob_reads()?;
        Ok(())
    }

    /// Canonicalize one host-authorized final destination and represent it
    /// only as an opaque resource. The generated guest ABI receives the
    /// returned token, never this path or a directory capability.
    #[cfg(feature = "wasm-sketch-host")]
    pub(crate) fn grant_exact_output_wire(
        &self,
        store: u64,
        destination: &Path,
    ) -> Result<u64, HubError> {
        self.grant_exact_output(store, destination)
            .map(|token| token.0)
    }

    #[cfg(feature = "wasm-sketch-host")]
    pub(crate) fn grant_exact_output(
        &self,
        store: u64,
        destination: &Path,
    ) -> Result<OpaqueToken, HubError> {
        let parent = destination.parent().ok_or(HubError::Invalid)?;
        let name = destination.file_name().ok_or(HubError::Invalid)?;
        let parent = fs::canonicalize(parent).map_err(|_| HubError::Invalid)?;
        let destination = parent.join(name);
        let token = self.create_resource_value(
            store,
            OUTPUT_RESOURCE_KIND,
            OUTPUT_RIGHT_COMMIT,
            false,
            ResourceValue::ExactOutput(destination),
        )?;
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        state
            .resources
            .get_mut(&token)
            .ok_or(HubError::Invalid)?
            .reserved = false;
        Ok(token)
    }

    /// Stream one opaque blob into an authorized sibling temporary file and
    /// atomically replace the exact final path only after flush and close.
    /// Bulk writes run without the hub lock. Final replacement is serialized
    /// with revocation; this synchronous helper belongs on a blocking lane.
    #[cfg(feature = "wasm-sketch-host")]
    pub(crate) fn commit_blob_to_output(
        &self,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
    ) -> std::io::Result<()> {
        self.commit_blob_operation(store, blob, output, None)
    }

    #[cfg(feature = "wasm-sketch-host")]
    pub(crate) fn submit_output_commit(
        self: &Arc<Self>,
        runtime: crate::async_engine::RuntimeHandle,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
    ) -> Result<OpaqueToken, HubError> {
        let (operation, _) = self.submit(
            store,
            Some(output),
            OUTPUT_RESOURCE_KIND,
            OUTPUT_RIGHT_COMMIT,
        )?;
        let hub = Arc::clone(self);
        runtime
            .launch_blocking(move || {
                if hub
                    .commit_blob_operation(store, blob, output, Some(operation))
                    .is_err()
                {
                    let _ = hub.terminal(
                        operation,
                        TerminalResult {
                            terminal: Terminal::Rejected,
                            resource: None,
                        },
                    );
                }
            })
            .detach();
        Ok(operation)
    }

    #[cfg(feature = "wasm-sketch-host")]
    fn commit_blob_operation(
        &self,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
        operation: Option<OpaqueToken>,
    ) -> std::io::Result<()> {
        let destination = self.exact_output_path(store, output)?;
        {
            let mut state = self
                .state
                .lock()
                .map_err(|_| hub_io_error(HubError::Closed))?;
            if state
                .operations
                .values()
                .any(|operation| operation.resource == Some(blob) && operation.terminal.is_none())
            {
                return Err(hub_io_error(HubError::WrongRights));
            }
            let resource = state
                .resources
                .get_mut(&blob)
                .ok_or_else(|| hub_io_error(HubError::Invalid))?;
            Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)
                .map_err(hub_io_error)?;
            if resource.committing {
                return Err(hub_io_error(HubError::WrongRights));
            }
            if !matches!(resource.value, ResourceValue::Blob { sealed: true, .. }) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "blob producer has not published EOF",
                ));
            }
            resource.committing = true;
        }
        let _consumption = BlobCommitLease { hub: self, blob };
        let temporary = self.temporary_output_path(&destination)?;
        // Only clean up a file this operation successfully created. A
        // competing creator must never have its file removed on open failure.
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        let result = (|| {
            loop {
                let chunk = self
                    .read_blob_chunk(store, blob, self.blob_limits.maximum_chunk_bytes, true)
                    .map_err(hub_io_error)?;
                if chunk.is_empty() {
                    break;
                }
                file.write_all(&chunk)?;
            }
            file.sync_all()?;
            drop(file);
            self.replace_output_operation(store, blob, output, &temporary, operation)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            return result;
        }
        Ok(())
    }

    #[cfg(feature = "wasm-sketch-host")]
    fn replace_authorized_output(
        &self,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
        temporary: &Path,
    ) -> std::io::Result<()> {
        self.replace_output_operation(store, blob, output, temporary, None)
    }

    #[cfg(feature = "wasm-sketch-host")]
    fn replace_output_operation(
        &self,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
        temporary: &Path,
        operation: Option<OpaqueToken>,
    ) -> std::io::Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| hub_io_error(HubError::Closed))?;
        if state.closed {
            return Err(hub_io_error(HubError::Closed));
        }
        if let Some(token) = operation {
            let slot = state
                .operations
                .get(&token)
                .ok_or_else(|| hub_io_error(HubError::Closed))?;
            if slot.owner.store != store || slot.terminal.is_some() {
                return Err(hub_io_error(HubError::Closed));
            }
        }
        let resource = state
            .resources
            .get(&blob)
            .ok_or_else(|| hub_io_error(HubError::Closed))?;
        Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)
            .map_err(hub_io_error)?;
        let resource = state
            .resources
            .get(&output)
            .ok_or_else(|| hub_io_error(HubError::Closed))?;
        Self::validate_resource(resource, store, OUTPUT_RESOURCE_KIND, OUTPUT_RIGHT_COMMIT)
            .map_err(hub_io_error)?;
        let ResourceValue::ExactOutput(destination) = &resource.value else {
            return Err(hub_io_error(HubError::WrongKind));
        };
        // Revocation cannot interleave between this last authority check and
        // the filesystem commit. No await, callback, or guest re-entry occurs.
        crate::fs_replace_file(temporary, destination)?;
        let mut wakes = Vec::new();
        if let Some(token) = operation {
            if let Some(wake) = Self::terminal_locked(
                &mut state,
                token,
                TerminalResult {
                    terminal: Terminal::Completed,
                    resource: None,
                },
            )
            .map_err(hub_io_error)?
            {
                wakes.push(wake);
            }
        }
        wakes.extend(
            Self::close_resource_with_terminal_locked(&mut state, blob, Terminal::Closed)
                .map_err(hub_io_error)?,
        );
        wakes.extend(
            Self::close_resource_with_terminal_locked(&mut state, output, Terminal::Closed)
                .map_err(hub_io_error)?,
        );
        drop(state);
        for wake in wakes {
            wake.notify_one();
        }
        Ok(())
    }

    #[cfg(feature = "wasm-sketch-host")]
    fn exact_output_path(&self, store: u64, output: OpaqueToken) -> std::io::Result<PathBuf> {
        let state = self
            .state
            .lock()
            .map_err(|_| hub_io_error(HubError::Closed))?;
        let resource = state
            .resources
            .get(&output)
            .ok_or_else(|| hub_io_error(HubError::Invalid))?;
        Self::validate_resource(resource, store, OUTPUT_RESOURCE_KIND, OUTPUT_RIGHT_COMMIT)
            .map_err(hub_io_error)?;
        match &resource.value {
            ResourceValue::ExactOutput(path) => Ok(path.clone()),
            _ => Err(hub_io_error(HubError::WrongKind)),
        }
    }

    #[cfg(feature = "wasm-sketch-host")]
    fn temporary_output_path(&self, destination: &Path) -> std::io::Result<PathBuf> {
        let parent = destination
            .parent()
            .ok_or_else(|| hub_io_error(HubError::Invalid))?;
        let name = destination
            .file_name()
            .ok_or_else(|| hub_io_error(HubError::Invalid))?;
        for _ in 0..16 {
            let mut temporary_name = name.to_os_string();
            temporary_name.push(format!(
                ".kernal-api-{}.tmp",
                next_token().map_err(hub_io_error)?
            ));
            let temporary = parent.join(temporary_name);
            if !temporary.exists() {
                return Ok(temporary);
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not reserve a unique exact-output temporary path",
        ))
    }

    fn validate_resource(
        resource: &ResourceSlot,
        store: u64,
        kind: u8,
        rights: u8,
    ) -> Result<(), HubError> {
        if resource.reserved {
            return Err(HubError::Closed);
        }
        if resource.identity.kind != kind {
            return Err(HubError::WrongKind);
        }
        if resource.identity.rights & rights != rights {
            return Err(HubError::WrongRights);
        }
        if resource.owner.store != store && !resource.shareable {
            return Err(HubError::WrongRights);
        }
        Ok(())
    }

    fn create_resource_value(
        &self,
        store: u64,
        kind: u8,
        rights: u8,
        shareable: bool,
        value: ResourceValue,
    ) -> Result<OpaqueToken, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed {
            return Err(HubError::Closed);
        }
        if state.resources.len() >= self.maximum_resources {
            return Err(HubError::Quota);
        }
        let slot = match state.free_resource_slots.pop() {
            Some(slot) => slot,
            None => {
                state.next_resource_slot = state
                    .next_resource_slot
                    .checked_add(1)
                    .ok_or(HubError::Exhausted)?;
                state.next_resource_slot
            }
        };
        let generation = state
            .resource_generations
            .get(&slot)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(HubError::Exhausted)?;
        state.resource_generations.insert(slot, generation);
        let token = OpaqueToken(next_token()?);
        state.resources.insert(
            token,
            ResourceSlot {
                identity: ResourceIdentity {
                    scope: self.scope,
                    slot,
                    generation,
                    kind,
                    rights,
                },
                owner: Owner { store },
                shareable,
                value,
                reserved: true,
                committing: false,
            },
        );
        Ok(token)
    }

    pub(super) fn submit(
        &self,
        store: u64,
        resource: Option<OpaqueToken>,
        kind: u8,
        required_rights: u8,
    ) -> Result<(OpaqueToken, Arc<Notify>), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        if state.closed {
            return Err(HubError::Closed);
        }
        if state.operations.len() >= self.maximum_operations {
            return Err(HubError::Quota);
        }
        if let Some(resource) = resource {
            let slot = state.resources.get(&resource).ok_or(HubError::Invalid)?;
            if slot.committing {
                return Err(HubError::WrongRights);
            }
            if slot.reserved {
                return Err(HubError::Closed);
            }
            if slot.identity.scope != self.scope {
                return Err(HubError::Stale);
            }
            if slot.identity.kind != kind {
                return Err(HubError::WrongKind);
            }
            if slot.identity.rights & required_rights != required_rights {
                return Err(HubError::WrongRights);
            }
            // A resource created by another Store is usable only through the
            // explicit sharing bit.
            if slot.owner.store != store && !slot.shareable {
                return Err(HubError::WrongRights);
            }
        }
        let token = OpaqueToken(next_token()?);
        let notify = Arc::new(Notify::new());
        state.operations.insert(
            token,
            OperationSlot {
                owner: Owner { store },
                resource,
                terminal: None,
                deferred_completion: None,
                suspended: false,
                notify: Arc::clone(&notify),
                created_resource: None,
                pending_blob_write: None,
                pending_blob_read: None,
                is_blob_read: false,
                blob_read_result: None,
            },
        );
        Ok((token, notify))
    }

    pub(super) fn suspend(&self, token: OpaqueToken, store: u64) -> Result<Arc<Notify>, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let (notify, deferred) = {
            let operation = state.operations.get(&token).ok_or(HubError::Invalid)?;
            if operation.owner.store != store {
                return Err(HubError::Stale);
            }
            if operation.terminal.is_some() {
                // Poll and yield are separate guest imports. Native I/O can
                // complete between them. Preserve the result for the next
                // poll and return a ready waiter, not an import failure.
                let notify = Arc::clone(&operation.notify);
                state.suspends = state.suspends.saturating_add(1);
                drop(state);
                notify.notify_one();
                return Ok(notify);
            }
            (Arc::clone(&operation.notify), operation.deferred_completion)
        };
        let operation = state.operations.get_mut(&token).ok_or(HubError::Invalid)?;
        operation.suspended = true;
        operation.deferred_completion = None;
        state.suspends = state.suspends.saturating_add(1);
        drop(state);
        if let Some(deferred) = deferred {
            self.finish_deferred_completion(token, deferred)?;
        }
        Ok(notify)
    }

    /// Complete a synthetic operation only after its generated guest future
    /// has registered a suspension. Synthetic fixtures deliberately exercise
    /// a pending outcome; real native I/O may complete before suspension and
    /// is handled by the ready-waiter path above.
    fn complete_after_suspend(
        &self,
        token: OpaqueToken,
        result: TerminalResult,
        revoke: Option<OpaqueToken>,
    ) -> Result<(), HubError> {
        let deferred = DeferredCompletion { result, revoke };
        let ready = {
            let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
            let operation = state.operations.get_mut(&token).ok_or(HubError::Invalid)?;
            if operation.terminal.is_some() {
                return Ok(());
            }
            if !operation.suspended {
                operation.deferred_completion = Some(deferred);
                false
            } else {
                true
            }
        };
        if ready {
            self.finish_deferred_completion(token, deferred)?;
        }
        Ok(())
    }

    fn finish_deferred_completion(
        &self,
        token: OpaqueToken,
        deferred: DeferredCompletion,
    ) -> Result<(), HubError> {
        // Claim the terminal result and resource revocation under the one hub
        // mutex. A cancellation that wins first therefore leaves the resource
        // live; a close that wins first revokes it before either outcome is
        // observable. Notifications happen only after releasing the lock.
        let notifications = {
            let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
            if state
                .operations
                .get(&token)
                .ok_or(HubError::Invalid)?
                .terminal
                .is_some()
            {
                return Ok(());
            }
            let mut notifications = if let Some(resource) = deferred.revoke {
                Self::close_resource_with_terminal_locked(&mut state, resource, Terminal::Closed)?
            } else {
                Vec::new()
            };
            if let Some(notify) = Self::terminal_locked(&mut state, token, deferred.result)? {
                notifications.push(notify);
            }
            notifications
        };
        for notify in notifications {
            notify.notify_one();
        }
        self.drive_blob_writes()?;
        Ok(())
    }

    pub(super) fn take_terminal(
        &self,
        token: OpaqueToken,
        store: u64,
    ) -> Result<Option<TerminalResult>, HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let terminal = {
            let operation = state.operations.get(&token).ok_or(HubError::Invalid)?;
            if operation.owner.store != store {
                return Err(HubError::Stale);
            }
            operation.terminal
        };
        if terminal.is_some() {
            state.resumes = state.resumes.saturating_add(1);
            state.operations.remove(&token);
        }
        Ok(terminal)
    }

    pub(super) fn terminal(
        &self,
        token: OpaqueToken,
        result: TerminalResult,
    ) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let notify = Self::terminal_locked(&mut state, token, result)?;
        drop(state);
        if let Some(notify) = notify {
            // One generated guest future owns one operation. `notify_one`
            // preserves a completion that wins before waiter registration.
            notify.notify_one();
        }
        Ok(())
    }

    fn terminal_locked(
        state: &mut State,
        token: OpaqueToken,
        result: TerminalResult,
    ) -> Result<Option<Arc<Notify>>, HubError> {
        let (notify, created) = {
            let operation = state.operations.get_mut(&token).ok_or(HubError::Invalid)?;
            if operation.terminal.is_some() {
                return Ok(None);
            }
            operation.terminal = Some(result);
            operation.pending_blob_write = None;
            operation.pending_blob_read = None;
            (Arc::clone(&operation.notify), operation.created_resource)
        };
        if let Some(resource) = created {
            if result.terminal == Terminal::Completed {
                if let Some(resource) = state.resources.get_mut(&resource) {
                    resource.reserved = false;
                }
            } else if let Some(resource) = state.resources.remove(&resource) {
                state.free_resource_slots.push(resource.identity.slot);
            }
        }
        Ok(Some(notify))
    }

    pub(super) fn close_resource(&self, token: OpaqueToken) -> Result<(), HubError> {
        self.close_resource_with_terminal(token, Terminal::Closed)
    }

    /// Revoke a native resource generation and wake every operation borrowing
    /// it with the callback's semantic terminal reason.  This preserves a
    /// rejected navigation or timeout instead of racing it into a generic
    /// `Closed` result.
    pub(crate) fn revoke_external_resource(
        &self,
        token: OpaqueToken,
        terminal: Terminal,
    ) -> Result<(), HubError> {
        self.close_resource_with_terminal(token, terminal)
    }

    fn close_resource_with_terminal(
        &self,
        token: OpaqueToken,
        terminal: Terminal,
    ) -> Result<(), HubError> {
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let notifications = Self::close_resource_with_terminal_locked(&mut state, token, terminal)?;
        drop(state);
        for notify in notifications {
            notify.notify_one();
        }
        self.drive_blob_writes()?;
        Ok(())
    }

    fn close_resource_with_terminal_locked(
        state: &mut State,
        token: OpaqueToken,
        terminal: Terminal,
    ) -> Result<Vec<Arc<Notify>>, HubError> {
        let Some(resource) = state.resources.remove(&token) else {
            return if state.closed_resources.contains(&token) {
                Ok(Vec::new())
            } else {
                Err(HubError::Invalid)
            };
        };
        if let ResourceValue::Blob { buffer, .. } = &resource.value {
            state.buffered_blob_bytes = state.buffered_blob_bytes.saturating_sub(buffer.len());
        }
        state.closed_resources.insert(token);
        state.closed_resource_order.push_back(token);
        while state.closed_resource_order.len() > MAX_CLOSED_TOMBSTONES {
            if let Some(expired) = state.closed_resource_order.pop_front() {
                state.closed_resources.remove(&expired);
            }
        }
        state.free_resource_slots.push(resource.identity.slot);
        let mut notifications = Vec::new();
        for operation in state.operations.values_mut() {
            if operation.resource == Some(token) && operation.terminal.is_none() {
                operation.pending_blob_write = None;
                operation.pending_blob_read = None;
                operation.terminal = Some(TerminalResult {
                    terminal,
                    resource: None,
                });
                notifications.push(Arc::clone(&operation.notify));
            }
        }
        Ok(notifications)
    }

    pub(super) fn close_all(&self, terminal: Terminal) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        state.closed = true;
        state.resources.clear();
        state.buffered_blob_bytes = 0;
        state.free_resource_slots.clear();
        let mut notifications = Vec::new();
        for operation in state.operations.values_mut() {
            operation.blob_read_result = None;
            if operation.terminal.is_none() {
                operation.pending_blob_write = None;
                operation.pending_blob_read = None;
                operation.terminal = Some(TerminalResult {
                    terminal,
                    resource: None,
                });
                notifications.push(Arc::clone(&operation.notify));
            }
        }
        drop(state);
        for notify in notifications {
            notify.notify_one();
        }
    }

    pub(crate) fn snapshot(&self) -> HubSnapshot {
        let state = self.state.lock().expect("operation hub mutex poisoned");
        HubSnapshot {
            scope: self.scope,
            pending_operations: state
                .operations
                .values()
                .filter(|operation| operation.terminal.is_none())
                .count(),
            live_resources: state.resources.len(),
            suspends: state.suspends,
            resumes: state.resumes,
            buffered_blob_bytes: state.buffered_blob_bytes,
            peak_buffered_blob_bytes: state.peak_buffered_blob_bytes,
            pending_write_bytes: state
                .operations
                .values()
                .filter_map(|operation| operation.pending_blob_write.as_ref())
                .map(Vec::len)
                .sum(),
            completed_read_bytes: state
                .operations
                .values()
                .filter_map(|operation| operation.blob_read_result.as_ref())
                .map(Vec::len)
                .sum(),
            retained_transfer_capacity: state
                .resources
                .values()
                .map(|resource| match &resource.value {
                    ResourceValue::Blob { buffer, .. } => buffer.capacity(),
                    _ => 0,
                })
                .chain(state.operations.values().map(|operation| {
                    operation
                        .pending_blob_write
                        .as_ref()
                        .map_or(0, Vec::capacity)
                        .saturating_add(
                            operation.blob_read_result.as_ref().map_or(0, Vec::capacity),
                        )
                }))
                .fold(0_usize, usize::saturating_add),
        }
    }
}

#[cfg(feature = "wasm-sketch-host")]
struct BlobCommitLease<'a> {
    hub: &'a OperationHub,
    blob: OpaqueToken,
}

#[cfg(feature = "wasm-sketch-host")]
impl Drop for BlobCommitLease<'_> {
    fn drop(&mut self) {
        let _ = self.hub.close_resource(self.blob);
    }
}

fn next(counter: &AtomicU64) -> Result<u64, HubError> {
    counter
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map(|value| value + 1)
        .map_err(|_| HubError::Exhausted)
}

fn next_token() -> Result<u64, HubError> {
    let value = next(&NEXT_OPAQUE_TOKEN)?;
    if value > MAX_WIRE_TOKEN {
        return Err(HubError::Exhausted);
    }
    Ok(value)
}

fn status(terminal: Terminal) -> u8 {
    match terminal {
        Terminal::Completed => STATUS_COMPLETED,
        Terminal::Cancelled => STATUS_CANCELLED,
        Terminal::TimedOut => STATUS_TIMED_OUT,
        Terminal::Trapped => STATUS_TRAPPED,
        Terminal::OwnerExited => STATUS_OWNER_EXITED,
        Terminal::Closed => STATUS_CLOSED,
        Terminal::Rejected => STATUS_REJECTED,
    }
}

fn pack(status: u8, payload: Option<OpaqueToken>) -> u64 {
    let payload = payload.map_or(0, |token| token.0);
    debug_assert!(payload <= MAX_WIRE_TOKEN);
    (payload << 8) | u64::from(status)
}

#[cfg(feature = "wasm-sketch-host")]
fn hub_io_error(error: HubError) -> std::io::Error {
    let kind = match error {
        HubError::Quota => std::io::ErrorKind::WouldBlock,
        HubError::Invalid | HubError::Stale | HubError::WrongKind | HubError::WrongRights => {
            std::io::ErrorKind::PermissionDenied
        }
        HubError::Closed => std::io::ErrorKind::BrokenPipe,
        HubError::Exhausted => std::io::ErrorKind::Other,
    };
    std::io::Error::new(
        kind,
        format!("opaque resource operation rejected: {error:?}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn wire_output_commit_uses_only_the_scoped_host_grant() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("authorized");
        std::fs::write(&path, b"previous").unwrap();
        let hub = OperationHub::new(8, 4).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"replacement").unwrap();
        hub.seal_blob(1, blob).unwrap();
        let output = hub.grant_exact_output(1, &path).unwrap();
        assert!(hub
            .submit_wire(runtime.handle(), 2, OP_OUTPUT_COMMIT, blob.0, output.0)
            .is_err());
        assert!(hub
            .submit_wire(runtime.handle(), 1, OP_OUTPUT_COMMIT, blob.0, u64::MAX)
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"previous");
        let operation = hub
            .submit_wire(runtime.handle(), 1, OP_OUTPUT_COMMIT, blob.0, output.0)
            .unwrap();
        runtime.run(async {
            crate::async_engine::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    let status = hub.poll_wire(1, operation) as u8;
                    if status != STATUS_PENDING {
                        assert_eq!(status, STATUS_COMPLETED);
                        break;
                    }
                    hub.suspend_wire(1, operation).unwrap().notified().await;
                }
            })
            .await
            .unwrap();
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        assert_eq!(hub.snapshot().live_resources, 0);
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn output_commit_runs_on_supplied_runtime_and_cancellation_wins_before_rename() {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("final");
        let hub = OperationHub::new(8, 8).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"complete").unwrap();
        hub.seal_blob(1, blob).unwrap();
        let output = hub.grant_exact_output(1, &final_path).unwrap();
        let commit = hub
            .submit_output_commit(runtime.handle(), 1, blob, output)
            .unwrap();
        runtime.run(async {
            crate::async_engine::timeout(std::time::Duration::from_secs(2), async {
                loop {
                    if let Some(result) = hub.take_terminal(commit, 1).unwrap() {
                        assert_eq!(result.terminal, Terminal::Completed);
                        break;
                    }
                    crate::async_engine::yield_now().await;
                }
            })
            .await
            .unwrap();
        });
        assert_eq!(std::fs::read(&final_path).unwrap(), b"complete");
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"cancelled").unwrap();
        hub.seal_blob(1, blob).unwrap();
        let output = hub.grant_exact_output(1, &final_path).unwrap();
        let (commit, _) = hub
            .submit(1, Some(output), OUTPUT_RESOURCE_KIND, OUTPUT_RIGHT_COMMIT)
            .unwrap();
        hub.cancel_wire(1, commit.0).unwrap();
        assert!(hub
            .commit_blob_operation(1, blob, output, Some(commit))
            .is_err());
        assert_eq!(
            hub.take_terminal(commit, 1).unwrap().unwrap().terminal,
            Terminal::Cancelled
        );
        assert_eq!(std::fs::read(&final_path).unwrap(), b"complete");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn output_commit_refuses_a_blob_with_a_pending_reader() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("final");
        let hub = OperationHub::new(4, 4).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let output = hub.grant_exact_output(1, &destination).unwrap();
        let read = hub.submit_blob_read(1, blob, 4).unwrap();
        assert!(hub.commit_blob_to_output(1, blob, output).is_err());
        assert!(!destination.exists());
        assert_eq!(hub.take_blob_read(1, read), Ok(None));
    }

    #[test]
    fn commit_consumption_excludes_other_readers_and_generated_submissions() {
        let hub = OperationHub::new(4, 4).unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"data").unwrap();
        hub.seal_blob(1, blob).unwrap();
        hub.state
            .lock()
            .unwrap()
            .resources
            .get_mut(&blob)
            .unwrap()
            .committing = true;
        assert_eq!(hub.blob_read(1, blob, 4), Err(HubError::WrongRights));
        assert_eq!(hub.submit_blob_read(1, blob, 4), Err(HubError::WrongRights));
        assert_eq!(hub.read_blob_chunk(1, blob, 4, true).unwrap(), b"data");
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn revoked_output_cannot_replace_final_after_the_temporary_is_flushed() {
        for teardown in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let final_path = directory.path().join("final");
            let temporary = directory.path().join("staged");
            std::fs::write(&final_path, b"original").unwrap();
            std::fs::write(&temporary, b"replacement").unwrap();
            let hub = OperationHub::new(4, 4).unwrap();
            let blob = hub.create_blob(1).unwrap();
            let output = hub.grant_exact_output(1, &final_path).unwrap();
            hub.seal_blob(1, blob).unwrap();
            if teardown {
                hub.close_all(Terminal::Cancelled);
            } else {
                hub.close_resource(output).unwrap();
            }
            assert!(hub
                .replace_authorized_output(1, blob, output, &temporary)
                .is_err());
            assert_eq!(std::fs::read(&final_path).unwrap(), b"original");
            assert_eq!(std::fs::read(&temporary).unwrap(), b"replacement");
        }
    }

    #[test]
    fn read_submission_releases_capacity_before_its_result_is_collected() {
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"full").unwrap();
        let write = hub.submit_blob_write(1, blob, b"next").unwrap();
        assert_eq!(hub.take_terminal(write, 1), Ok(None));
        let read = hub.submit_blob_read(1, blob, 4).unwrap();
        assert_eq!(
            hub.take_terminal(write, 1).unwrap().unwrap().terminal,
            Terminal::Completed
        );
        assert_eq!(
            hub.take_blob_read(1, read),
            Ok(Some((Terminal::Completed, b"full".to_vec())))
        );
        assert_eq!(hub.blob_read(1, blob, 4).unwrap(), b"next");
    }

    #[test]
    fn generated_seal_completes_pending_eof_and_rejects_later_writes() {
        let hub = OperationHub::new(4, 1).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let blob = hub.create_blob(1).unwrap();
        let read = hub.submit_blob_read(1, blob, 1).unwrap();
        assert_eq!(
            hub.collect_blob_read_wire(1, read.0, 1, |_| panic!("not EOF yet")),
            Ok(0)
        );
        assert!(hub
            .submit_wire(runtime.handle(), 2, OP_BLOB_SEAL, blob.0, 0)
            .is_err());
        let seal = hub
            .submit_wire(runtime.handle(), 1, OP_BLOB_SEAL, blob.0, 0)
            .unwrap();
        assert_eq!(hub.poll_wire(1, seal) as u8, STATUS_COMPLETED);
        assert_eq!(
            hub.collect_blob_read_wire(1, read.0, 1, |bytes| assert!(bytes.is_empty())),
            Ok(u64::from(STATUS_COMPLETED))
        );
        let write = hub.submit_blob_write(1, blob, b"x").unwrap();
        assert_eq!(hub.poll_wire(1, write.0) as u8, STATUS_CLOSED);
    }

    #[test]
    fn retained_blob_capacity_blocks_other_blob_growth_until_released() {
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(1024, 1024, 1024).unwrap())
            .unwrap();
        let first = hub.create_blob(1).unwrap();
        let second = hub.create_blob(1).unwrap();
        hub.blob_write(1, first, &[7; 1024]).unwrap();
        drop(hub.blob_read(1, first, 512).unwrap());
        assert_eq!(hub.blob_write(1, second, &[8; 512]), Err(HubError::Quota));
        let pending = hub.submit_blob_write(1, second, &[8; 512]).unwrap();
        assert_eq!(hub.take_terminal(pending, 1), Ok(None));
        drop(hub.blob_read(1, first, 512).unwrap());
        assert_eq!(
            hub.take_terminal(pending, 1).unwrap().unwrap().terminal,
            Terminal::Completed
        );
        assert_eq!(hub.blob_read(1, second, 512).unwrap(), [8; 512]);
    }

    #[test]
    fn capacity_snapshot_includes_partial_blob_and_uncollected_read_allocations() {
        let hub = OperationHub::with_blob_limits(4, 1, BlobLimits::new(1024, 1024, 1024).unwrap())
            .unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, &[7; 1024]).unwrap();
        let read = hub.submit_blob_read(1, blob, 512).unwrap();
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.buffered_blob_bytes, 512);
        assert_eq!(snapshot.completed_read_bytes, 512);
        assert!(
            snapshot.retained_transfer_capacity >= 1536,
            "partial drain retains backing capacity plus the read result"
        );
        drop(hub.take_blob_read(1, read).unwrap());
        drop(hub.blob_read(1, blob, 512).unwrap());
        assert_eq!(hub.snapshot().retained_transfer_capacity, 0);
        hub.close_all(Terminal::Closed);
        assert_eq!(hub.snapshot().retained_transfer_capacity, 0);
    }

    #[test]
    fn fully_drained_blobs_release_their_backing_allocation() {
        for asynchronous in [false, true] {
            let hub =
                OperationHub::with_blob_limits(4, 2, BlobLimits::new(1024, 1024, 1024).unwrap())
                    .unwrap();
            let blob = hub.create_blob(1).unwrap();
            hub.blob_write(1, blob, &[7; 1024]).unwrap();
            if asynchronous {
                let read = hub.submit_blob_read(1, blob, 1024).unwrap();
                hub.take_blob_read(1, read).unwrap().unwrap();
            } else {
                hub.blob_read(1, blob, 1024).unwrap();
            }
            let state = hub.state.lock().unwrap();
            let ResourceValue::Blob { buffer, .. } = &state.resources[&blob].value else {
                panic!("blob");
            };
            assert_eq!(
                buffer.capacity(),
                0,
                "empty live blobs must not retain unaccounted allocation"
            );
            assert_eq!(state.buffered_blob_bytes, 0);
        }
    }

    #[test]
    fn wire_read_collection_is_bounded_typed_and_consumed_once() {
        let hub = OperationHub::with_blob_limits(4, 1, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let read = hub.submit_blob_read(1, blob, 4).unwrap();
        assert_eq!(
            hub.collect_blob_read_wire(1, read.0, 4, |_| panic!("pending copy")),
            Ok(0)
        );
        hub.blob_write(1, blob, b"data").unwrap();
        assert_eq!(hub.poll_wire(1, read.0) as u8, STATUS_ERROR);
        assert_eq!(
            hub.collect_blob_read_wire(2, read.0, 4, |_| panic!("wrong owner")),
            Err(HubError::Stale)
        );
        assert_eq!(
            hub.collect_blob_read_wire(1, read.0, 3, |_| panic!("short destination")),
            Err(HubError::Quota)
        );
        let mut copied = Vec::new();
        assert_eq!(
            hub.collect_blob_read_wire(1, read.0, 4, |bytes| copied.extend_from_slice(bytes)),
            Ok((4 << 8) | u64::from(STATUS_COMPLETED))
        );
        assert_eq!(copied, b"data");
        assert_eq!(hub.snapshot().completed_read_bytes, 0);
        assert!(hub
            .collect_blob_read_wire(1, read.0, 4, |_| panic!("double collection"))
            .is_err());
        let write = hub.submit_blob_write(1, blob, b"next").unwrap();
        assert_eq!(
            hub.collect_blob_read_wire(1, write.0, 4, |_| panic!("wrong kind")),
            Err(HubError::WrongKind)
        );
        assert_eq!(hub.poll_wire(1, write.0) as u8, STATUS_COMPLETED);
    }

    #[test]
    fn guest_write_rejects_before_copy_and_retains_only_owned_bytes() {
        let hub = OperationHub::with_blob_limits(4, 1, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        assert_eq!(
            hub.submit_blob_write_wire(1, blob.0, 5, || panic!("oversized copy")),
            Err(HubError::Quota)
        );
        assert_eq!(
            hub.submit_blob_write_wire(2, blob.0, 4, || panic!("unauthorized copy")),
            Err(HubError::WrongRights)
        );
        let mut guest_bytes = *b"data";
        let write = hub
            .submit_blob_write_wire(1, blob.0, 4, || guest_bytes.to_vec())
            .unwrap();
        guest_bytes.fill(0);
        assert_eq!(hub.poll_wire(1, write) as u8, STATUS_COMPLETED);
        assert_eq!(hub.blob_read(1, blob, 4).unwrap(), b"data");
    }

    #[test]
    fn wire_blob_creation_is_scoped_and_cancelled_creation_reclaims_capacity() {
        let hub = OperationHub::new(4, 1).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            hub.submit_wire(runtime.handle(), 1, OP_BLOB_CREATE, 1, 0),
            Err(HubError::Invalid)
        );
        let cancelled = hub
            .submit_wire(runtime.handle(), 1, OP_BLOB_CREATE, 0, 0)
            .unwrap();
        hub.cancel_wire(1, cancelled).unwrap();
        assert_eq!(hub.snapshot().live_resources, 0);
        assert_eq!(hub.poll_wire(1, cancelled) as u8, STATUS_CANCELLED);

        let operation = hub
            .submit_wire(runtime.handle(), 1, OP_BLOB_CREATE, 0, 0)
            .unwrap();
        let wake = hub.suspend_wire(1, operation).unwrap();
        runtime.run(async { wake.notified().await });
        let result = hub.poll_wire(1, operation);
        assert_eq!(result as u8, STATUS_COMPLETED);
        let blob = OpaqueToken(result >> 8);
        assert_eq!(
            hub.blob_write(2, blob, b"foreign"),
            Err(HubError::WrongRights)
        );
        assert_eq!(hub.blob_write(1, blob, b"owned"), Ok(5));
        let close = hub
            .submit_wire(runtime.handle(), 1, OP_SYNTHETIC_RESOURCE_CLOSE, blob.0, 0)
            .unwrap();
        let wake = hub.suspend_wire(1, close).unwrap();
        runtime.run(async { wake.notified().await });
        assert_eq!(hub.poll_wire(1, close) as u8, STATUS_COMPLETED);
        assert_eq!(hub.snapshot().live_resources, 0);
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
    }

    #[test]
    fn generated_create_at_operation_quota_reclaims_its_reservation() {
        let hub = OperationHub::new(1, 1).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (occupied, _) = hub.submit(1, None, 0, 0).unwrap();
        assert_eq!(
            hub.submit_wire(runtime.handle(), 1, OP_SYNTHETIC_RESOURCE_CREATE, 0, 1),
            Err(HubError::Quota)
        );
        assert_eq!(hub.snapshot().live_resources, 0);
        hub.cancel_wire(1, occupied.0).unwrap();
        hub.take_terminal(occupied, 1).unwrap();
        assert!(hub
            .submit_wire(runtime.handle(), 1, OP_SYNTHETIC_RESOURCE_CREATE, 0, 1)
            .is_ok());
    }

    #[test]
    fn teardown_reclaims_unconsumed_read_results_and_pending_write_buffers() {
        for reason in [
            Terminal::Cancelled,
            Terminal::Trapped,
            Terminal::TimedOut,
            Terminal::OwnerExited,
            Terminal::Closed,
        ] {
            let hub =
                OperationHub::with_blob_limits(8, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
            let blob = hub.create_blob(1).unwrap();
            hub.blob_write(1, blob, b"read").unwrap();
            let read = hub.submit_blob_read(1, blob, 4).unwrap();
            hub.blob_write(1, blob, b"full").unwrap();
            let write = hub.submit_blob_write(1, blob, b"wait").unwrap();
            let before = hub.snapshot();
            assert_eq!(before.completed_read_bytes, 4);
            assert_eq!(before.pending_write_bytes, 4);
            assert_eq!(before.buffered_blob_bytes, 4);
            hub.close_all(reason);
            let after = hub.snapshot();
            assert_eq!(
                (
                    after.completed_read_bytes,
                    after.pending_write_bytes,
                    after.buffered_blob_bytes,
                    after.live_resources,
                    after.pending_operations
                ),
                (0, 0, 0, 0, 0)
            );
            assert_eq!(hub.take_blob_read(1, read), Err(HubError::Closed));
            assert_eq!(
                hub.take_terminal(write, 1).unwrap().unwrap().terminal,
                reason
            );
        }
    }

    #[test]
    fn pending_reads_distinguish_data_eof_and_cancellation() {
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let read = hub.submit_blob_read(1, blob, 4).unwrap();
        assert_eq!(hub.take_blob_read(1, read), Ok(None));
        let wake = hub.suspend(read, 1).unwrap();
        hub.submit_blob_write(1, blob, b"data").unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            crate::async_engine::timeout(std::time::Duration::from_secs(1), wake.notified())
                .await
                .unwrap();
        });
        assert_eq!(
            hub.take_blob_read(1, read),
            Ok(Some((Terminal::Completed, b"data".to_vec())))
        );
        let cancelled = hub.submit_blob_read(1, blob, 4).unwrap();
        hub.cancel_wire(1, cancelled.0).unwrap();
        assert_eq!(
            hub.take_blob_read(1, cancelled),
            Ok(Some((Terminal::Cancelled, vec![])))
        );
        let eof = hub.submit_blob_read(1, blob, 4).unwrap();
        assert_eq!(hub.take_blob_read(1, eof), Ok(None));
        hub.seal_blob(1, blob).unwrap();
        assert_eq!(
            hub.take_blob_read(1, eof),
            Ok(Some((Terminal::Completed, vec![])))
        );
    }

    #[test]
    fn pending_blob_write_resumes_after_pull_and_cancellation_reclaims_input() {
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        hub.blob_write(1, blob, b"full").unwrap();
        let write = hub.submit_blob_write(1, blob, b"next").unwrap();
        assert_eq!(hub.take_terminal(write, 1), Ok(None));
        let wake = hub.suspend(write, 1).unwrap();
        assert_eq!(hub.blob_read(1, blob, 4).unwrap(), b"full");
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            crate::async_engine::timeout(std::time::Duration::from_secs(1), wake.notified())
                .await
                .unwrap();
        });
        assert_eq!(
            hub.take_terminal(write, 1).unwrap().unwrap().terminal,
            Terminal::Completed
        );
        let cancelled = hub.submit_blob_write(1, blob, b"lost").unwrap();
        hub.cancel_wire(1, cancelled.0).unwrap();
        assert!(hub.state.lock().unwrap().operations[&cancelled]
            .pending_blob_write
            .is_none());
        assert_eq!(hub.blob_read(1, blob, 4).unwrap(), b"next");
        assert!(hub.blob_read(1, blob, 4).unwrap().is_empty());
        assert_eq!(
            hub.take_terminal(cancelled, 1).unwrap().unwrap().terminal,
            Terminal::Cancelled
        );
        hub.close_all(Terminal::Closed);
        assert_eq!(hub.snapshot().pending_operations, 0);
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
    }

    #[test]
    fn generated_close_resumes_another_blobs_capacity_waiter() {
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let first = hub.create_blob(1).unwrap();
        let second = hub.create_blob(1).unwrap();
        hub.blob_write(1, first, b"full").unwrap();
        let write = hub.submit_blob_write(1, second, b"next").unwrap();
        let close = hub
            .submit_wire(runtime.handle(), 1, OP_SYNTHETIC_RESOURCE_CLOSE, first.0, 0)
            .unwrap();
        let wake = hub.suspend_wire(1, close).unwrap();
        runtime.run(async { wake.notified().await });
        assert_eq!(hub.poll_wire(1, close) as u8, STATUS_COMPLETED);
        assert!(
            hub.take_terminal(write, 1).unwrap().is_some(),
            "generated close must drive writes"
        );
    }

    #[test]
    fn read_progress_drives_previously_skipped_writers_without_collection() {
        let hub = OperationHub::with_blob_limits(8, 4, BlobLimits::new(4, 4, 8).unwrap()).unwrap();
        let a = hub.create_blob(1).unwrap();
        let b = hub.create_blob(1).unwrap();
        let c = hub.create_blob(1).unwrap();
        let retained = hub.create_blob(1).unwrap();
        hub.blob_write(1, a, b"full").unwrap();
        hub.blob_write(1, retained, b"full").unwrap();
        hub.submit_blob_write(1, b, b"next").unwrap();
        let last = hub.submit_blob_write(1, c, b"last").unwrap();
        // Release backing allocations, not merely payload-length credits.
        hub.submit_blob_read(1, b, 4).unwrap();
        drop(hub.blob_read(1, a, 4).unwrap());
        assert!(
            hub.take_terminal(last, 1).unwrap().is_some(),
            "read progress must revisit skipped writers"
        );
    }

    #[test]
    fn closing_a_blob_releases_shared_capacity_to_another_pending_writer() {
        let hub = OperationHub::with_blob_limits(4, 2, BlobLimits::new(4, 4, 4).unwrap()).unwrap();
        let first = hub.create_blob(1).unwrap();
        let second = hub.create_blob(1).unwrap();
        hub.blob_write(1, first, b"full").unwrap();
        let write = hub.submit_blob_write(1, second, b"next").unwrap();
        assert_eq!(hub.take_terminal(write, 1), Ok(None));
        hub.close_resource(first).unwrap();
        assert_eq!(
            hub.take_terminal(write, 1).unwrap().unwrap().terminal,
            Terminal::Completed
        );
        assert_eq!(hub.blob_read(1, second, 4).unwrap(), b"next");
    }

    #[test]
    fn stale_cross_owner_and_close_races_are_typed_and_bounded() {
        let hub = OperationHub::new(1, 1).unwrap();
        let resource = hub.create_resource(0, 7, 0b11, true).unwrap();
        hub.activate_for_test(resource);
        let (operation, _) = hub.submit(0, Some(resource), 7, 0b01).unwrap();
        assert!(matches!(hub.submit(0, None, 0, 0), Err(HubError::Quota)));
        assert_eq!(hub.take_terminal(operation, 2), Err(HubError::Stale));
        hub.close_resource(resource).unwrap();
        hub.terminal(
            operation,
            TerminalResult {
                terminal: Terminal::Completed,
                resource: None,
            },
        )
        .unwrap();
        assert_eq!(
            hub.take_terminal(operation, 0),
            Ok(Some(TerminalResult {
                terminal: Terminal::Closed,
                resource: None
            }))
        );
        assert_eq!(hub.snapshot().pending_operations, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
    }

    #[test]
    fn nonshareable_resource_rejects_another_store_before_operation_creation() {
        let hub = OperationHub::new(2, 1).unwrap();
        let resource = hub.create_resource(0, 1, 1, false).unwrap();
        hub.activate_for_test(resource);
        assert!(matches!(
            hub.submit(1, Some(resource), 1, 1),
            Err(HubError::WrongRights)
        ));
        assert_eq!(hub.snapshot().pending_operations, 0);
    }

    #[test]
    fn terminal_consumption_reclaims_quota_and_close_is_idempotent() {
        let hub = OperationHub::new(1, 1).unwrap();
        let resource = hub.create_resource(3, 9, 1, false).unwrap();
        hub.activate_for_test(resource);
        let (first, _) = hub.submit(3, Some(resource), 9, 1).unwrap();
        hub.terminal(
            first,
            TerminalResult {
                terminal: Terminal::Completed,
                resource: Some(resource),
            },
        )
        .unwrap();
        assert!(hub.take_terminal(first, 3).unwrap().is_some());
        assert!(hub.submit(3, Some(resource), 9, 1).is_ok());
        hub.close_resource(resource).unwrap();
        hub.close_resource(resource).unwrap();
        let replacement = hub.create_resource(3, 9, 1, false).unwrap();
        assert_ne!(resource, replacement, "stale token cannot name reused slot");
    }

    #[test]
    fn external_webview_uses_the_same_generation_safe_authority() {
        let hub = OperationHub::new(4, 2).unwrap();
        let (resource, open) = hub.begin_external_webview_open(41).unwrap();
        hub.finish_external_open(open, resource);
        assert_eq!(
            hub.observe_terminal(41, open),
            Ok(Some(TerminalResult {
                terminal: Terminal::Completed,
                resource: Some(resource),
            }))
        );
        let wait = hub.begin_external_webview_wait(41, resource).unwrap();
        assert_eq!(
            hub.begin_external_webview_wait(42, resource),
            Err(HubError::WrongRights),
            "another logical instance cannot borrow this external resource"
        );
        hub.finish_external_operation(wait, Terminal::Completed);
        assert_eq!(
            hub.observe_terminal(41, wait),
            Ok(Some(TerminalResult {
                terminal: Terminal::Completed,
                resource: None,
            }))
        );
        let close = hub.begin_external_webview_close(41, resource).unwrap();
        hub.finish_external_operation(close, Terminal::Completed);
        assert_eq!(
            hub.observe_terminal(41, close),
            Ok(Some(TerminalResult {
                terminal: Terminal::Completed,
                resource: None,
            }))
        );
        hub.close_resource(resource).unwrap();
        assert_eq!(
            hub.begin_external_webview_wait(41, resource),
            Err(HubError::Closed),
            "closed native backing cannot revive its generation"
        );
    }

    #[test]
    fn native_terminal_cleanup_reclaims_every_handle_and_rejects_stale_use() {
        for terminal in [Terminal::TimedOut, Terminal::Cancelled, Terminal::Closed] {
            let hub = OperationHub::new(4, 1).unwrap();
            let (resource, open) = hub.begin_external_webview_open(7).unwrap();
            hub.finish_external_open(open, resource);
            assert!(matches!(
                hub.observe_terminal(7, open),
                Ok(Some(TerminalResult {
                    terminal: Terminal::Completed,
                    resource: Some(_),
                }))
            ));
            let waiter = hub.begin_external_webview_wait(7, resource).unwrap();
            hub.revoke_external_resource(resource, terminal).unwrap();
            assert_eq!(
                hub.observe_terminal(7, waiter),
                Ok(Some(TerminalResult {
                    terminal,
                    resource: None,
                }))
            );
            assert_eq!(
                hub.begin_external_webview_wait(7, resource),
                Err(HubError::Closed),
                "a terminal native event must not revive the old generation"
            );
            let snapshot = hub.snapshot();
            assert_eq!(snapshot.pending_operations, 0, "{terminal:?}");
            assert_eq!(snapshot.live_resources, 0, "{terminal:?}");
        }
    }

    #[test]
    fn terminal_between_guest_poll_and_yield_still_wakes_the_owner() {
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        for terminal in [Terminal::Completed, Terminal::Cancelled, Terminal::Closed] {
            let hub = OperationHub::new(1, 1).unwrap();
            let (operation, _) = hub.submit(4, None, 0, 0).unwrap();
            assert_eq!(hub.poll_wire(4, operation.0), 0);
            hub.terminal(
                operation,
                TerminalResult {
                    terminal,
                    resource: None,
                },
            )
            .unwrap();
            assert!(matches!(
                hub.suspend_wire(5, operation.0),
                Err(HubError::Stale)
            ));
            let wake = hub
                .suspend_wire(4, operation.0)
                .expect("completion must not reject yield");
            runtime.run(async {
                crate::async_engine::timeout(std::time::Duration::from_secs(1), wake.notified())
                    .await
                    .expect("terminal operation must wake immediately");
            });
            assert_eq!(hub.poll_wire(4, operation.0) as u8, status(terminal));
            assert!(hub.suspend_wire(4, operation.0).is_err());
        }
    }

    #[test]
    fn completion_before_waiter_registration_is_not_lost() {
        let hub = OperationHub::new(1, 1).unwrap();
        let (operation, _) = hub.submit(4, None, 0, 0).unwrap();
        let wake = hub.suspend(operation, 4).unwrap();
        hub.terminal(
            operation,
            TerminalResult {
                terminal: Terminal::Cancelled,
                resource: None,
            },
        )
        .unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async { wake.notified().await });
        assert_eq!(
            hub.take_terminal(operation, 4),
            Ok(Some(TerminalResult {
                terminal: Terminal::Cancelled,
                resource: None,
            }))
        );
        assert_eq!(hub.snapshot().pending_operations, 0);
    }

    #[test]
    fn common_dispatch_reserves_resource_before_scheduling_and_returns_it_once() {
        let hub = OperationHub::new(2, 1).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let operation = hub
            .dispatch(
                runtime.handle(),
                0,
                Request::SyntheticCreate {
                    kind: 5,
                    rights: 1,
                    shareable: false,
                },
            )
            .unwrap();
        let wake = hub.suspend(operation, 0).unwrap();
        runtime.run(async { wake.notified().await });
        let result = hub.take_terminal(operation, 0).unwrap().unwrap();
        assert_eq!(result.terminal, Terminal::Completed);
        assert!(result.resource.is_some());
        assert_eq!(hub.snapshot().live_resources, 1);
    }

    #[test]
    fn generated_close_stays_pending_until_its_guest_waiter_parks() {
        let hub = OperationHub::new(2, 1).unwrap();
        let resource = hub.create_resource(0, 5, 1, false).unwrap();
        hub.activate_for_test(resource);
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();

        let operation = hub
            .dispatch(runtime.handle(), 0, Request::Close { resource })
            .unwrap();

        // Drive the detached completion past its scheduler yield before the
        // generated synchronous `operation_yield` import is invoked. The old
        // implementation terminalized here, making the subsequent suspend
        // fail nondeterministically in the real Wasm artifact.
        runtime.run(async {
            crate::async_engine::yield_now().await;
            crate::async_engine::yield_now().await;
        });
        assert_eq!(hub.take_terminal(operation, 0), Ok(None));
        assert_eq!(hub.snapshot().live_resources, 1);

        let wake = hub.suspend(operation, 0).unwrap();
        runtime.run(async { wake.notified().await });
        assert_eq!(
            hub.take_terminal(operation, 0),
            Ok(Some(TerminalResult {
                terminal: Terminal::Completed,
                resource: None,
            }))
        );
        assert_eq!(
            hub.begin_external_webview_wait(0, resource),
            Err(HubError::Closed),
            "close revokes the generation only after the guest is parked"
        );
        assert_eq!(hub.snapshot().suspends, 1);
        assert_eq!(hub.snapshot().resumes, 1);
        assert_eq!(hub.snapshot().pending_operations, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
    }

    #[test]
    fn cancelled_deferred_close_cannot_revoke_its_resource() {
        let hub = OperationHub::new(2, 1).unwrap();
        let resource = hub.create_resource(0, 5, 1, false).unwrap();
        hub.activate_for_test(resource);
        let (operation, _) = hub.submit(0, None, 0, 0).unwrap();
        let deferred = DeferredCompletion {
            result: TerminalResult {
                terminal: Terminal::Completed,
                resource: None,
            },
            revoke: Some(resource),
        };

        hub.cancel_wire(0, operation.0).unwrap();
        // This models a detached close completion that was copied from the
        // deferred slot just as cancellation won. It must be a no-op: a
        // cancelled close does not acquire authority to revoke its resource.
        hub.finish_deferred_completion(operation, deferred).unwrap();

        assert_eq!(
            hub.take_terminal(operation, 0),
            Ok(Some(TerminalResult {
                terminal: Terminal::Cancelled,
                resource: None,
            }))
        );
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.pending_operations, 0);
        assert_eq!(snapshot.live_resources, 1);
        assert!(hub.submit(0, Some(resource), 5, 1).is_ok());
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.pending_operations, 1);
        assert_eq!(snapshot.live_resources, 1);
    }

    #[test]
    fn wire_submit_and_poll_use_one_packed_terminal_result() {
        let hub = OperationHub::new(2, 1).unwrap();
        let runtime = crate::async_engine::RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let operation = hub
            .submit_wire(runtime.handle(), 0, OP_SYNTHETIC_RESOURCE_CREATE, 0, 1)
            .unwrap();
        let wake = hub.suspend_wire(0, operation).unwrap();
        runtime.run(async { wake.notified().await });
        let packed = hub.poll_wire(0, operation);
        assert_eq!(packed & 0xff, u64::from(STATUS_COMPLETED));
        assert_ne!(
            packed >> 8,
            0,
            "create payload is the opaque resource token"
        );
        assert_eq!(hub.poll_wire(0, operation) & 0xff, u64::from(STATUS_ERROR));
    }

    #[test]
    fn opaque_blob_transfers_sixty_four_mebibytes_in_bounded_pull_chunks() {
        let limits = BlobLimits::new(64 * 1024, 64 * 1024, 64 * 1024).unwrap();
        let hub = OperationHub::with_blob_limits(4, 2, limits).unwrap();
        let blob = hub.create_blob(7).unwrap();
        let chunk = vec![0xA5; limits.maximum_chunk_bytes];
        let mut transferred = 0;
        while transferred < 64 * 1024 * 1024 {
            assert_eq!(hub.blob_write(7, blob, &chunk), Ok(chunk.len()));
            assert_eq!(hub.blob_read(7, blob, chunk.len()), Ok(chunk.clone()));
            transferred += chunk.len();
            assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
        }
        let snapshot = hub.snapshot();
        assert_eq!(
            snapshot.peak_buffered_blob_bytes,
            limits.maximum_chunk_bytes
        );
        assert_eq!(snapshot.live_resources, 1);
    }

    #[test]
    fn blob_capacity_is_released_only_by_a_bounded_pull_and_close_reclaims_it() {
        let limits = BlobLimits::new(4, 8, 8).unwrap();
        let hub = OperationHub::with_blob_limits(2, 2, limits).unwrap();
        let blob = hub.create_blob(1).unwrap();
        assert_eq!(hub.blob_write(1, blob, b"1234"), Ok(4));
        assert_eq!(hub.blob_write(1, blob, b"5678"), Ok(4));
        assert_eq!(hub.blob_write(1, blob, b"x"), Err(HubError::Quota));
        assert_eq!(hub.blob_read(1, blob, 4), Ok(b"1234".to_vec()));
        assert_eq!(hub.blob_write(1, blob, b"x"), Ok(1));
        assert_eq!(hub.snapshot().buffered_blob_bytes, 5);
        hub.close_resource(blob).unwrap();
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
        assert_eq!(hub.blob_read(1, blob, 1), Err(HubError::Invalid));
    }

    #[test]
    fn blob_scope_and_chunk_limits_reject_before_copying() {
        let limits = BlobLimits::new(4, 8, 8).unwrap();
        let hub = OperationHub::with_blob_limits(2, 2, limits).unwrap();
        let blob = hub.create_blob(1).unwrap();
        assert_eq!(hub.blob_write(2, blob, b"a"), Err(HubError::WrongRights));
        assert_eq!(hub.blob_write(1, blob, b"12345"), Err(HubError::Quota));
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
        hub.close_all(Terminal::Trapped);
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn exact_output_commits_only_the_authorized_path_after_bounded_blob_flush() {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("capture.png");
        std::fs::write(&final_path, b"old").unwrap();
        let limits = BlobLimits::new(4, 8, 8).unwrap();
        let hub = OperationHub::with_blob_limits(4, 4, limits).unwrap();
        let blob = hub.create_blob(11).unwrap();
        let output = hub.grant_exact_output(11, &final_path).unwrap();
        hub.blob_write(11, blob, b"png-").unwrap();
        hub.blob_write(11, blob, b"byte").unwrap();
        hub.seal_blob(11, blob).unwrap();
        hub.commit_blob_to_output(11, blob, output).unwrap();
        assert_eq!(std::fs::read(&final_path).unwrap(), b"png-byte");
        assert_eq!(hub.snapshot().buffered_blob_bytes, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("kernal-api-")));
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn exact_output_handle_cannot_be_used_by_another_instance() {
        let directory = tempfile::tempdir().unwrap();
        let final_path = directory.path().join("capture.png");
        let hub = OperationHub::with_blob_limits(4, 4, BlobLimits::new(4, 8, 8).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let output = hub.grant_exact_output(1, &final_path).unwrap();
        hub.blob_write(1, blob, b"data").unwrap();
        let error = hub.commit_blob_to_output(2, blob, output).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(!final_path.exists());
        assert_eq!(hub.snapshot().buffered_blob_bytes, 4);
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn failed_exact_output_replace_removes_the_temporary_and_never_replaces_final() {
        let directory = tempfile::tempdir().unwrap();
        // A directory is a canonical, exact host object but not a replaceable
        // file. It gives this test a real platform rename failure without a
        // test-only filesystem side channel.
        let final_path = directory.path().join("not-a-file");
        std::fs::create_dir(&final_path).unwrap();
        let hub = OperationHub::with_blob_limits(4, 4, BlobLimits::new(4, 8, 8).unwrap()).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let output = hub.grant_exact_output(1, &final_path).unwrap();
        hub.blob_write(1, blob, b"data").unwrap();
        hub.seal_blob(1, blob).unwrap();
        assert!(hub.commit_blob_to_output(1, blob, output).is_err());
        assert!(
            final_path.is_dir(),
            "a failed commit must not replace final"
        );
        assert!(std::fs::read_dir(directory.path())
            .unwrap()
            .all(|entry| !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("kernal-api-")));
        hub.close_all(Terminal::Cancelled);
        let snapshot = hub.snapshot();
        assert_eq!(snapshot.live_resources, 0);
        assert_eq!(snapshot.buffered_blob_bytes, 0);
        assert_eq!(snapshot.pending_operations, 0);
    }

    #[cfg(feature = "wasm-sketch-host")]
    #[test]
    fn empty_unsealed_blob_cannot_commit_a_truncated_output() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("output");
        std::fs::write(&destination, b"previous").unwrap();
        let hub = OperationHub::new(4, 4).unwrap();
        let blob = hub.create_blob(1).unwrap();
        let output = hub.grant_exact_output(1, &destination).unwrap();
        assert_eq!(
            hub.commit_blob_to_output(1, blob, output)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(std::fs::read(&destination).unwrap(), b"previous");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
        hub.blob_write(1, blob, b"complete").unwrap();
        hub.seal_blob(1, blob).unwrap();
        assert_eq!(hub.blob_write(1, blob, b"late"), Err(HubError::Closed));
        hub.commit_blob_to_output(1, blob, output).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"complete");
    }
}
