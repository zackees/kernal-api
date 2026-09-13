#![cfg(any(
    feature = "daemon-registration",
    feature = "daemon-registration-v2",
    feature = "daemon-frame-v1"
))]

#[cfg(feature = "daemon-registration")]
fn cache_root_to_backend<'a>(
    value: kernal_api::daemon_registration::CacheRoot<'a>,
) -> running_process::daemon_registration_compat::CacheRoot<'a> {
    value
}
#[cfg(feature = "daemon-registration")]
fn cache_root_to_facade<'a>(
    value: running_process::daemon_registration_compat::CacheRoot<'a>,
) -> kernal_api::daemon_registration::CacheRoot<'a> {
    value
}
#[cfg(feature = "daemon-registration")]
fn root_kind_to_backend(
    value: kernal_api::daemon_registration::CacheRootKind,
) -> running_process::daemon_registration_compat::CacheRootKind {
    value
}
#[cfg(feature = "daemon-registration")]
fn error_to_facade(
    value: running_process::daemon_registration_compat::DaemonRegistrationError,
) -> kernal_api::daemon_registration::DaemonRegistrationError {
    value
}

#[cfg(feature = "daemon-registration-v2")]
fn v2_error_to_backend(
    value: kernal_api::daemon_registration_v2::DaemonRegistrationV2Error,
) -> running_process::daemon_registration_v2_compat::DaemonRegistrationV2Error {
    value
}
#[cfg(feature = "daemon-registration-v2")]
fn v2_builder_to_facade(
    value: running_process::daemon_registration_v2_compat::ServiceDefinitionBuilder,
) -> kernal_api::daemon_registration_v2::ServiceDefinitionBuilder {
    value
}

#[cfg(feature = "daemon-frame-v1")]
fn frame_to_backend(
    value: kernal_api::daemon_frame_v1::DaemonFrame,
) -> running_process::daemon_frame_v1::DaemonFrame {
    value
}

#[cfg(feature = "daemon-frame-v1")]
fn frame_to_facade(
    value: running_process::daemon_frame_v1::DaemonFrame,
) -> kernal_api::daemon_frame_v1::DaemonFrame {
    value
}

#[cfg(feature = "daemon-registration")]
#[test]
fn borrowed_roots_cross_namespaces_without_reallocation() {
    let path = String::from("/cache/borrowed-path");
    let manifest = kernal_api::daemon_registration::CacheManifestBuilder::new("identity", "1.0.0")
        .root(
            kernal_api::daemon_registration::CacheRootKind::CacheData,
            &path,
        )
        .build()
        .expect("build manifest");
    let root = manifest.roots().next().expect("borrowed root");
    let original = root.path().as_ptr();
    let root = cache_root_to_facade(cache_root_to_backend(root));
    assert_eq!(root.path(), path);
    assert_eq!(root.path().as_ptr(), original);
}

#[test]
fn compatibility_namespaces_are_exact_backend_identities() {
    #[cfg(feature = "daemon-registration")]
    let _ = (
        cache_root_to_backend,
        cache_root_to_facade,
        root_kind_to_backend,
        error_to_facade,
    );
    #[cfg(feature = "daemon-registration-v2")]
    let _ = (v2_error_to_backend, v2_builder_to_facade);
    #[cfg(feature = "daemon-frame-v1")]
    let _ = (frame_to_backend, frame_to_facade);
}
