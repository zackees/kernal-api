//! Compile-time identity checks for contained process migration primitives.

use kernal_api::containment::{ContainedProcessGroup, SpawnedChild};

fn backend_to_facade_group(value: running_process::ContainedProcessGroup) -> ContainedProcessGroup {
    value
}
fn facade_to_backend_group(value: ContainedProcessGroup) -> running_process::ContainedProcessGroup {
    value
}
fn backend_to_facade_child(value: running_process::SpawnedChild) -> SpawnedChild {
    value
}
fn facade_to_backend_child(value: SpawnedChild) -> running_process::SpawnedChild {
    value
}

#[test]
fn canonical_containment_types_are_assignable_in_both_namespaces() {
    let _ = (
        backend_to_facade_group,
        facade_to_backend_group,
        backend_to_facade_child,
        facade_to_backend_child,
    );
    assert_eq!(
        kernal_api::containment::ORIGINATOR_ENV_VAR,
        running_process::ORIGINATOR_ENV_VAR
    );
}
