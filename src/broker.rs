//! Canonical broker identity, probe, client refusal, and frozen-v1 frame API.
//!
//! This opt-in module is a direct namespace delegation to the selected
//! `running-process` client contract. It intentionally preserves type identity
//! for handles, endpoint identity, route/refusal errors, and frame bytes; it
//! does not expose a whole substrate crate or its platform-private types.

// The broker is a distinct, opt-in capability boundary. Re-exporting its
// public submodules keeps frozen protocol/route/error type identities while
// still avoiding a `running_process::*` crate-wide export.
pub use running_process::broker::*;

pub use running_process::backend_identity::{
    encode_framed, read_frame, read_frame_with_cap, try_decode_framed, write_frame,
};
pub use running_process::backend_identity::{
    endpoint_probe_request_from_frame, endpoint_probe_response_frame, handle_endpoint_probe,
    read_daemon_identity_file, read_endpoint_probe_request, remove_daemon_identity_file,
    try_read_daemon_identity_file, write_daemon_identity_file, write_endpoint_probe_response,
    BackendEndpointMux, BackendHandle, BackendHandleError, Connection, DaemonIdentityHashPolicy,
    DaemonProcess, DecodedFramed, Endpoint, EndpointNameError, EndpointProbeRequest,
    EndpointProbeServerError, Frame, FrameKind, FramingError, LegacyClassification, MuxError,
    MuxPoll, PayloadEncoding, BACKEND_HANDLE_PROBE_PAYLOAD_PROTOCOL, ENVELOPE_VERSION,
    FRAME_HEADER_BYTES, MAX_FRAME_BYTES, MAX_HELLO_BYTES, PROTOCOL_VERSION,
};
pub use running_process::broker::client::{
    connect_to_backend, BackendConnection, BackendConnectionRoute, BrokerClientError,
    ConnectBackendRequest, RefusalKind,
};

/// Canonical PID/daemon identity verification and retained-handle control.
/// The direct namespace preserves `ProcessHandle` identity: verify with
/// `verify_daemon_process_for_control`, then call `force_kill_handle` on that
/// same object instead of reopening a PID that may have been reused.
pub use running_process::broker::backend_lifecycle::verify_pid;
