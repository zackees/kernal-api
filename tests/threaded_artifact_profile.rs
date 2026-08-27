#![cfg(feature = "wasm-sketch-host")]

use std::path::PathBuf;

use kernal_api::wasm::{
    threaded_artifact_manifest_for_test, SketchCompiler, SketchCompilerConfig, SketchModulePolicy,
};

const ARTIFACT_ENV: &str = "KERNAL_API_THREADED_SMOKE_WASM";

#[test]
fn real_threaded_rust_guest_is_a_red_admission_characterization() {
    let Ok(path) = std::env::var(ARTIFACT_ENV) else {
        // Ordinary default and all-feature test runs have no guest artifact.
        // The Soldr scripts provide one explicitly for this RED characterization.
        return;
    };
    let path = PathBuf::from(path);
    let bytes = std::fs::read(&path).expect("read KERNAL_API_THREADED_SMOKE_WASM");

    let manifest = threaded_artifact_manifest_for_test(&bytes).expect("inspect real Wasm artifact");
    assert_eq!(
        threaded_artifact_manifest_for_test(&bytes).expect("repeat inspection"),
        manifest,
        "artifact manifest must be deterministic",
    );
    assert!(manifest.contains("import module=kernal-api:v1 name=kernel-yield"));
    assert!(manifest.contains("type index="));
    eprintln!("{manifest}");

    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::new(bytes.len().saturating_add(1), 65_536).expect("policy");
    let error = match compiler.admit(&bytes, policy) {
        Ok(_) => panic!("#25 must reject the real threaded Rust artifact in this RED slice"),
        Err(error) => error,
    };
    assert_eq!(compiler.compiled_module_count(), 0, "rejection is pre-compilation");
    eprintln!("threaded-artifact-admission-red={}", error.code());
}
