#![cfg(feature = "wasm-sketch-host")]

use kernal_api::wasm::{
    SketchAdmissionError, SketchAdmissionPolicy, SketchEngine, SketchEngineConfig,
};

fn engine() -> SketchEngine {
    SketchEngine::new(SketchEngineConfig::default()).expect("private Cranelift engine")
}

fn policy() -> SketchAdmissionPolicy {
    SketchAdmissionPolicy::new(1024 * 1024, 16).expect("bounded policy")
}

fn wasm(source: &str) -> Vec<u8> {
    wat::parse_str(source).expect("test-only WAT fixture")
}

fn valid_module() -> Vec<u8> {
    wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16 shared))
              (import "wasi" "thread-spawn" (func (param i32) (result i32)))
              (import "kernal-api:v1" "kernel-yield" (func))
              (func (export "kernal-api-run"))
            )
        "#,
    )
}

#[test]
fn valid_versioned_kernel_module_is_admitted_once_with_bounded_shared_memory() {
    let module = engine().admit(&valid_module(), policy()).expect("admit module");

    assert!(module.module_bytes() > 0);
    assert_eq!(module.shared_memory().minimum_pages(), 1);
    assert_eq!(module.shared_memory().maximum_pages(), 16);
    assert_eq!(module.shared_memory().maximum_bytes(), 16 * 64 * 1024);
}

#[test]
fn ambient_wasi_is_rejected_before_any_instance_exists() {
    let bytes = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16 shared))
              (import "wasi_snapshot_preview1" "fd_write" (func))
              (func (export "kernal-api-run"))
            )
        "#,
    );

    assert!(matches!(
        engine().admit(&bytes, policy()),
        Err(SketchAdmissionError::ForbiddenImport { module, name })
            if module == "wasi_snapshot_preview1" && name == "fd_write"
    ));
}

#[test]
fn versioned_namespace_does_not_admit_unspecified_kernel_entries() {
    let bytes = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16 shared))
              (import "kernal-api:v1" "ambient-by-another-name" (func))
              (func (export "kernal-api-run"))
            )
        "#,
    );

    assert!(matches!(
        engine().admit(&bytes, policy()),
        Err(SketchAdmissionError::ForbiddenImport { module, name })
            if module == "kernal-api:v1" && name == "ambient-by-another-name"
    ));
}

#[test]
fn shared_memory_must_be_the_owned_bounded_compatibility_import() {
    let unshared = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16))
              (func (export "kernal-api-run"))
            )
        "#,
    );
    assert!(matches!(
        engine().admit(&unshared, policy()),
        Err(SketchAdmissionError::UnsharedMemory)
    ));

    let oversized = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 17 shared))
              (func (export "kernal-api-run"))
            )
        "#,
    );
    assert!(matches!(
        engine().admit(&oversized, policy()),
        Err(SketchAdmissionError::SharedMemoryExceedsPolicy { policy_pages: 16, .. })
    ));
}

#[test]
fn compatibility_and_entrypoint_abi_mismatches_are_rejected() {
    let bad_thread_spawn = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16 shared))
              (import "wasi" "thread-spawn" (func (param i64) (result i32)))
              (func (export "kernal-api-run"))
            )
        "#,
    );
    assert!(matches!(
        engine().admit(&bad_thread_spawn, policy()),
        Err(SketchAdmissionError::ImportTypeMismatch { module, name })
            if module == "wasi" && name == "thread-spawn"
    ));

    let bad_entrypoint = wasm(
        r#"
            (module
              (import "env" "memory" (memory 1 16 shared))
              (func (export "kernal-api-run") (param i32))
            )
        "#,
    );
    assert!(matches!(
        engine().admit(&bad_entrypoint, policy()),
        Err(SketchAdmissionError::EntrypointMismatch)
    ));
}

#[test]
fn module_input_is_bounded_before_compilation() {
    let bytes = vec![0_u8; 8];
    let policy = SketchAdmissionPolicy::new(7, 16).expect("policy");
    assert!(matches!(
        engine().admit(&bytes, policy),
        Err(SketchAdmissionError::ModuleTooLarge {
            actual_bytes: 8,
            maximum_bytes: 7,
        })
    ));
}
