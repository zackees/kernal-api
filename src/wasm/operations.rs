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
        let request = match kind {
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
        buffer.extend(bytes);
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
        if maximum_bytes > self.blob_limits.maximum_chunk_bytes {
            return Err(HubError::Quota);
        }
        let mut state = self.state.lock().map_err(|_| HubError::Closed)?;
        let resource = state.resources.get_mut(&blob).ok_or(HubError::Invalid)?;
        Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)?;
        let ResourceValue::Blob { buffer, .. } = &mut resource.value else {
            return Err(HubError::WrongKind);
        };
        let count = maximum_bytes.min(buffer.len());
        let result: Vec<_> = buffer.drain(..count).collect();
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
        if bytes.len() > self.blob_limits.maximum_chunk_bytes {
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
            if pending.saturating_add(bytes.len()) > self.blob_limits.maximum_sketch_bytes {
                state.operations.remove(&operation);
                return Err(HubError::Quota);
            }
            let slot = state
                .operations
                .get_mut(&operation)
                .ok_or(HubError::Invalid)?;
            if slot.terminal.is_none() {
                slot.pending_blob_write = Some(bytes.to_vec());
            }
        }
        self.drive_blob_writes()?;
        Ok(operation)
    }

    fn drive_blob_writes(&self) -> Result<(), HubError> {
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
            let terminal = match state.resources.get(&resource).map(|slot| &slot.value) {
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
        for wake in wakes {
            wake.notify_one();
        }
        self.drive_blob_reads()?;
        Ok(())
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
        }
        self.drive_blob_reads()?;
        self.drive_blob_writes()?;
        Ok(token)
    }

    fn drive_blob_reads(&self) -> Result<(), HubError> {
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
        for wake in wakes {
            wake.notify_one();
        }
        Ok(())
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
    /// atomically replace the exact final path only after flush and close. No
    /// hub lock, Store, or guest memory reference survives a filesystem call.
    #[cfg(feature = "wasm-sketch-host")]
    pub(crate) fn commit_blob_to_output(
        &self,
        store: u64,
        blob: OpaqueToken,
        output: OpaqueToken,
    ) -> std::io::Result<()> {
        let destination = self.exact_output_path(store, output)?;
        {
            let state = self
                .state
                .lock()
                .map_err(|_| hub_io_error(HubError::Closed))?;
            let resource = state
                .resources
                .get(&blob)
                .ok_or_else(|| hub_io_error(HubError::Invalid))?;
            Self::validate_resource(resource, store, BLOB_RESOURCE_KIND, BLOB_RIGHT_READ)
                .map_err(hub_io_error)?;
            if !matches!(resource.value, ResourceValue::Blob { sealed: true, .. }) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "blob producer has not published EOF",
                ));
            }
        }
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
                    .blob_read(store, blob, self.blob_limits.maximum_chunk_bytes)
                    .map_err(hub_io_error)?;
                if chunk.is_empty() {
                    break;
                }
                file.write_all(&chunk)?;
            }
            file.sync_all()?;
            drop(file);
            crate::fs_replace_file(&temporary, &destination)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            return result;
        }
        self.close_resource(blob).map_err(hub_io_error)?;
        self.close_resource(output).map_err(hub_io_error)?;
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
                return Err(HubError::Closed);
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
    /// has registered a suspension. This is needed for close because its
    /// detached scheduler task can otherwise win the synchronous import path
    /// and make `operation_yield` reject an already-terminal operation.
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
        }
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
