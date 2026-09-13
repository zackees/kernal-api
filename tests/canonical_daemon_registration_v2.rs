//! Compile-time identity contracts for canonical v2 registration aliases.
#![cfg(feature = "daemon-registration-v2")]

use kernal_api::daemon_registration_v2::canonical::{
    read_service_definition_v2, LoadedServiceDefinitionV2, ServiceDefinition,
    ServiceDefinitionBuilder, ServiceDefinitionError,
};

fn backend_to_facade_definition(
    value: running_process::daemon_registration_v2::ServiceDefinition,
) -> ServiceDefinition {
    value
}
fn facade_to_backend_definition(
    value: ServiceDefinition,
) -> running_process::daemon_registration_v2::ServiceDefinition {
    value
}
fn backend_to_facade_builder(
    value: running_process::daemon_registration_v2::ServiceDefinitionBuilder,
) -> ServiceDefinitionBuilder {
    value
}
fn facade_to_backend_builder(
    value: ServiceDefinitionBuilder,
) -> running_process::daemon_registration_v2::ServiceDefinitionBuilder {
    value
}
fn backend_to_facade_error(
    value: running_process::daemon_registration_v2::ServiceDefinitionError,
) -> ServiceDefinitionError {
    value
}
fn facade_to_backend_error(
    value: ServiceDefinitionError,
) -> running_process::daemon_registration_v2::ServiceDefinitionError {
    value
}
fn backend_to_facade_loaded(
    value: running_process::daemon_registration_v2::LoadedServiceDefinitionV2,
) -> LoadedServiceDefinitionV2 {
    value
}
fn facade_to_backend_loaded(
    value: LoadedServiceDefinitionV2,
) -> running_process::daemon_registration_v2::LoadedServiceDefinitionV2 {
    value
}

#[test]
fn canonical_v2_registration_types_are_assignable_in_both_namespaces() {
    let _ = (
        backend_to_facade_definition,
        facade_to_backend_definition,
        backend_to_facade_builder,
        facade_to_backend_builder,
        backend_to_facade_error,
        facade_to_backend_error,
        backend_to_facade_loaded,
        facade_to_backend_loaded,
    );

    let _: fn(&std::path::Path, &str) -> Result<LoadedServiceDefinitionV2, ServiceDefinitionError> =
        read_service_definition_v2;
}
