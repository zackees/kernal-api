//! #189: compile-time proof that the opt-in scoped spawn surface is not a
//! facade mirror.
#![cfg(feature = "independent-spawn")]

use std::io;
use std::sync::atomic::AtomicBool;

#[test]
fn canonical_spawn_contract_has_identical_type_identity() {
    fn accepts_backend_mode(_: running_process::SpawnMode) {}
    fn accepts_facade_mode(_: kernal_api::SpawnMode) {}
    accepts_backend_mode(kernal_api::SpawnMode::Inherited);
    accepts_facade_mode(running_process::SpawnMode::Independent);

    fn accepts_backend_options(_: running_process::SpawnOptions) {}
    fn accepts_facade_options(_: kernal_api::SpawnOptions) {}
    let options = kernal_api::SpawnOptions::default();
    assert_eq!(options.mode, kernal_api::SpawnMode::Inherited);
    accepts_backend_options(options.clone());
    accepts_facade_options(running_process::SpawnOptions::default());

    let _: fn(running_process::SpawnLifetime) = |_: kernal_api::SpawnLifetime| {};
    let _: fn(kernal_api::SpawnLifetime) = |_: running_process::SpawnLifetime| {};
    let _: fn(running_process::IndependentBackend) = |_: kernal_api::IndependentBackend| {};
    let _: fn(kernal_api::IndependentBackend) = |_: running_process::IndependentBackend| {};
    let _: fn(running_process::SpawnHandle) = |_: kernal_api::SpawnHandle| {};
    let _: fn(kernal_api::SpawnHandle) = |_: running_process::SpawnHandle| {};
    let _: fn(running_process::SpawnExit) = |_: kernal_api::SpawnExit| {};
    let _: fn(kernal_api::SpawnExit) = |_: running_process::SpawnExit| {};
    let _: fn(running_process::independent_spawn::LaunchSpec) = |_: kernal_api::LaunchSpec| {};
    let _: fn(kernal_api::LaunchSpec) = |_: running_process::independent_spawn::LaunchSpec| {};
    let _: fn(running_process::independent_spawn::Readiness) = |_: kernal_api::Readiness| {};
    let _: fn(kernal_api::Readiness) = |_: running_process::independent_spawn::Readiness| {};

    let _: fn(
        &running_process::independent_spawn::LaunchSpec,
        &running_process::SpawnOptions,
        &AtomicBool,
    ) -> io::Result<running_process::SpawnHandle> = kernal_api::spawn_with_options;
}
