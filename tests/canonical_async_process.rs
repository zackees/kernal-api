//! Compile-time identity contract for semantic asynchronous child primitives.

use kernal_api::async_process::{
    AsyncProcess, AsyncProcessBuilder, AsyncProcessError, AsyncProcessSession, AsyncStdio,
    ProcessPriority, ProcessTreeKill, SpawnAdmission, StreamKind,
};

fn backend_to_facade_process(value: running_process::AsyncProcess) -> AsyncProcess {
    value
}
fn facade_to_backend_process(value: AsyncProcess) -> running_process::AsyncProcess {
    value
}
fn backend_to_facade_builder(value: running_process::AsyncProcessBuilder) -> AsyncProcessBuilder {
    value
}
fn facade_to_backend_builder(value: AsyncProcessBuilder) -> running_process::AsyncProcessBuilder {
    value
}
fn backend_to_facade_session(value: running_process::AsyncProcessSession) -> AsyncProcessSession {
    value
}
fn facade_to_backend_session(value: AsyncProcessSession) -> running_process::AsyncProcessSession {
    value
}
fn backend_to_facade_stdio(value: running_process::AsyncStdio) -> AsyncStdio {
    value
}
fn facade_to_backend_stdio(value: AsyncStdio) -> running_process::AsyncStdio {
    value
}
fn backend_to_facade_stream(value: running_process::StreamKind) -> StreamKind {
    value
}
fn facade_to_backend_stream(value: StreamKind) -> running_process::StreamKind {
    value
}
fn backend_to_facade_error(value: running_process::ProcessError) -> AsyncProcessError {
    value
}
fn facade_to_backend_error(value: AsyncProcessError) -> running_process::ProcessError {
    value
}
fn backend_to_facade_priority(value: running_process::ProcessPriority) -> ProcessPriority {
    value
}
fn facade_to_backend_priority(value: ProcessPriority) -> running_process::ProcessPriority {
    value
}
fn backend_to_facade_admission(value: running_process::SpawnAdmission) -> SpawnAdmission {
    value
}
fn facade_to_backend_admission(value: SpawnAdmission) -> running_process::SpawnAdmission {
    value
}
fn backend_to_facade_tree_kill(value: running_process::ProcessTreeKill) -> ProcessTreeKill {
    value
}
fn facade_to_backend_tree_kill(value: ProcessTreeKill) -> running_process::ProcessTreeKill {
    value
}

async fn canonical_control_kill_tree(
    control: &kernal_api::async_process::AsyncProcessSessionControl,
) -> Result<ProcessTreeKill, AsyncProcessError> {
    control.kill_tree(std::time::Duration::from_secs(1)).await
}

#[test]
fn canonical_async_process_types_are_assignable_in_both_namespaces() {
    let _ = (
        backend_to_facade_process,
        facade_to_backend_process,
        backend_to_facade_builder,
        facade_to_backend_builder,
        backend_to_facade_session,
        facade_to_backend_session,
        backend_to_facade_stdio,
        facade_to_backend_stdio,
        backend_to_facade_stream,
        facade_to_backend_stream,
        backend_to_facade_error,
        facade_to_backend_error,
        backend_to_facade_priority,
        facade_to_backend_priority,
        backend_to_facade_admission,
        facade_to_backend_admission,
        backend_to_facade_tree_kill,
        facade_to_backend_tree_kill,
    );
}

#[test]
fn tree_kill_outcome_retains_native_diagnostic_variants() {
    let tree = ProcessTreeKill::TreeKilled;
    let direct = ProcessTreeKill::ProcessKilled;
    assert!(matches!(tree, running_process::ProcessTreeKill::TreeKilled));
    assert!(matches!(
        direct,
        running_process::ProcessTreeKill::ProcessKilled
    ));
    let _ = canonical_control_kill_tree;
}

#[test]
fn canonical_builder_accepts_both_namespaces_without_conversion() {
    let admission = SpawnAdmission::new(|| Ok(()));
    let canonical: running_process::SpawnAdmission = admission;
    let builder = AsyncProcessBuilder::new("unused")
        .spawn_admission(canonical)
        .priority(running_process::ProcessPriority::Low)
        .priority_best_effort(ProcessPriority::High);
    let _: running_process::AsyncProcessBuilder = builder;
}
