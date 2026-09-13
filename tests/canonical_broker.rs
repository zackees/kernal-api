//! Compile-time identity contracts for the opt-in canonical broker namespace.
#![cfg(feature = "broker")]

use kernal_api::broker::verify_pid::{ProcessHandle, VerifyPidError};
use kernal_api::broker::{BackendHandle, BrokerClientError, Endpoint, Frame, RefusalKind};

fn backend_to_facade_handle(
    value: running_process::backend_identity::BackendHandle,
) -> BackendHandle {
    value
}
fn facade_to_backend_handle(
    value: BackendHandle,
) -> running_process::backend_identity::BackendHandle {
    value
}
fn backend_to_facade_endpoint(value: running_process::backend_identity::Endpoint) -> Endpoint {
    value
}
fn facade_to_backend_endpoint(value: Endpoint) -> running_process::backend_identity::Endpoint {
    value
}
fn backend_to_facade_frame(value: running_process::backend_identity::Frame) -> Frame {
    value
}
fn facade_to_backend_frame(value: Frame) -> running_process::backend_identity::Frame {
    value
}
fn backend_to_facade_error(
    value: running_process::broker::client::BrokerClientError,
) -> BrokerClientError {
    value
}
fn facade_to_backend_error(
    value: BrokerClientError,
) -> running_process::broker::client::BrokerClientError {
    value
}
fn backend_to_facade_refusal(value: running_process::broker::client::RefusalKind) -> RefusalKind {
    value
}
fn facade_to_backend_refusal(value: RefusalKind) -> running_process::broker::client::RefusalKind {
    value
}
fn backend_to_facade_process_handle(
    value: running_process::broker::backend_lifecycle::verify_pid::ProcessHandle,
) -> ProcessHandle {
    value
}
fn facade_to_backend_process_handle(
    value: ProcessHandle,
) -> running_process::broker::backend_lifecycle::verify_pid::ProcessHandle {
    value
}
fn backend_to_facade_verify_error(
    value: running_process::broker::backend_lifecycle::verify_pid::VerifyPidError,
) -> VerifyPidError {
    value
}
fn facade_to_backend_verify_error(
    value: VerifyPidError,
) -> running_process::broker::backend_lifecycle::verify_pid::VerifyPidError {
    value
}

fn facade_to_backend_control_verify(
    value: fn(
        &running_process::broker::backend_lifecycle::DaemonProcess,
    ) -> Result<
        running_process::broker::backend_lifecycle::verify_pid::ProcessHandle,
        running_process::broker::backend_lifecycle::verify_pid::VerifyPidError,
    >,
) -> fn(
    &running_process::broker::backend_lifecycle::DaemonProcess,
) -> Result<
    running_process::broker::backend_lifecycle::verify_pid::ProcessHandle,
    running_process::broker::backend_lifecycle::verify_pid::VerifyPidError,
> {
    value
}

fn facade_to_backend_held_kill(
    value: fn(&ProcessHandle) -> Result<(), VerifyPidError>,
) -> fn(
    &running_process::broker::backend_lifecycle::verify_pid::ProcessHandle,
) -> Result<(), running_process::broker::backend_lifecycle::verify_pid::VerifyPidError> {
    value
}

#[test]
fn canonical_broker_types_are_assignable_in_both_namespaces() {
    // Function bodies above are the identity proof without constructing opaque
    // handles, protocol frames, or error values.
    let _ = (
        backend_to_facade_process_handle,
        facade_to_backend_process_handle,
        backend_to_facade_endpoint,
        facade_to_backend_endpoint,
        backend_to_facade_frame,
        facade_to_backend_frame,
        backend_to_facade_error,
        facade_to_backend_error,
        backend_to_facade_refusal,
        facade_to_backend_refusal,
        backend_to_facade_handle,
        facade_to_backend_handle,
        backend_to_facade_verify_error,
        facade_to_backend_verify_error,
        facade_to_backend_control_verify(
            kernal_api::broker::verify_pid::verify_daemon_process_for_control,
        ),
        facade_to_backend_held_kill(kernal_api::broker::verify_pid::force_kill_handle),
    );
}
