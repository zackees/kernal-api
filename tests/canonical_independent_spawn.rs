//! Compile-time identity contract for issue #189.
#![cfg(feature = "independent-spawn")]

use kernal_api::{
    IndependentBackend, IndependentChild, LaunchSpec, Readiness, SpawnHandle, SpawnLifetime,
    SpawnMode, SpawnOptions,
};

fn backend_to_facade_mode(value: running_process::SpawnMode) -> SpawnMode {
    value
}

fn facade_to_backend_mode(value: SpawnMode) -> running_process::SpawnMode {
    value
}

fn backend_to_facade_options(value: running_process::SpawnOptions) -> SpawnOptions {
    value
}

fn facade_to_backend_options(value: SpawnOptions) -> running_process::SpawnOptions {
    value
}

#[test]
fn canonical_root_types_have_cross_namespace_identity() {
    let _ = facade_to_backend_mode(SpawnMode::Inherited);
    let _ = backend_to_facade_mode(running_process::SpawnMode::Inherited);
    let _ = facade_to_backend_options(SpawnOptions::default());
    let _ = backend_to_facade_options(running_process::SpawnOptions::default());

    let _: IndependentBackend = running_process::IndependentBackend::ExternalBroker {
        endpoint: "/private/broker".into(),
    };
    let _: SpawnLifetime = running_process::SpawnLifetime::KillOnDrop;
}

#[test]
fn module_namespace_preserves_canonical_signatures() {
    let _: fn(
        &running_process::independent_spawn::LaunchSpec,
        &running_process::SpawnOptions,
        &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<running_process::SpawnHandle> = kernal_api::spawn_independent;

    let _: Option<SpawnHandle> = None;

    let _: fn(
        &running_process::independent_spawn::LaunchSpec,
        &std::path::Path,
        std::time::Duration,
        &std::sync::atomic::AtomicBool,
    ) -> std::io::Result<running_process::independent_spawn::IndependentChild> =
        kernal_api::independent_spawn::spawn;

    let _: Option<LaunchSpec> = None;
    let _: Option<Readiness> = None;
    let _: Option<IndependentChild> = None;
}

#[test]
fn inherited_is_the_canonical_default() {
    assert_eq!(SpawnMode::default(), SpawnMode::Inherited);
    assert_eq!(SpawnOptions::default().mode, SpawnMode::Inherited);
}
