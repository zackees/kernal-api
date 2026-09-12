#![cfg(feature = "wasm-sketch-host")]

use kernal_api::async_engine::RuntimeBuilder;
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchExecutionError, SketchModuleError,
    SketchModulePolicy,
};

#[test]
#[ignore = "build the actual screenshot guest and supply KERNAL_API_SCREENSHOT_ARTIFACT_WASM"]
fn actual_screenshot_guest_admits_but_cannot_execute_without_webview_grants() {
    let artifact = std::env::var_os("KERNAL_API_SCREENSHOT_ARTIFACT_WASM")
        .expect("explicit actual Rust screenshot artifact required");
    let bytes = std::fs::read(artifact).expect("read screenshot artifact");
    // The build script appends metadata after the real import section. Change
    // the first ABI namespace in place, preserving every section length, and
    // require rejection before compilation rather than broadening the import
    // allowlist to accommodate this smaller application.
    let mut foreign = bytes.clone();
    let offset = foreign
        .windows(b"kernal-api:v1".len())
        .position(|window| window == b"kernal-api:v1")
        .expect("actual generated ABI import namespace");
    foreign[offset..offset + b"kernal-api:v1".len()].copy_from_slice(b"evilxx-api:v1");
    let rejected = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    assert!(matches!(
        rejected.admit(
            &foreign,
            SketchModulePolicy::threaded_rust_v1(foreign.len() + 1, 16_384).unwrap()
        ),
        Err(SketchModuleError::ForbiddenImport { .. }),
    ));
    assert_eq!(rejected.compiled_module_count(), 0);
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy");
    let sketch = compiler
        .admit(&bytes, policy)
        .expect("closed-profile admission");
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let outcome = runtime.run(sketch.execute_threaded_root(runtime.handle()));
    assert_eq!(
        outcome,
        Err(SketchExecutionError::Trapped),
        "ungranted command must trap, not time out or succeed"
    );
    assert_eq!(
        sketch.execution_limits_snapshot().active_root_executions(),
        0
    );
    assert_eq!(compiler.compiled_module_count(), 1);
}
