#![cfg(feature = "wasm-sketch-host")]

use kernal_api::async_engine::{RuntimeBuilder, RuntimeHandle};
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchModulePolicy, ThreadedRootOutcome,
};

#[test]
#[ignore = "run scripts/build-generated-core-smoke.sh to supply the linked Rust artifact"]
fn supplied_generated_guest_admits_and_executes() {
    let path = std::env::var_os("KERNAL_API_GENERATED_ARTIFACT_WASM")
        .expect("provide the real generated guest artifact");
    let bytes = std::fs::read(path).expect("read linked artifact");
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy");
    let sketch = compiler.admit(&bytes, policy).expect("generated admission");
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let outcome = runtime
        .run(async {
            sketch
                .execute_threaded_root(RuntimeHandle::current().expect("runtime handle"))
                .await
        })
        .expect("generated root and child execution");
    assert!(matches!(
        outcome,
        ThreadedRootOutcome::Started | ThreadedRootOutcome::Exited
    ));
    assert_eq!(compiler.compiled_module_count(), 1);
    sketch
        .close_threaded_root()
        .expect("close generated sketch");
    let counters = compiler.execution_limits_snapshot();
    assert_eq!(counters.live_guest_threads(), 0);
    assert_eq!(counters.live_stores(), 0);
    assert_eq!(counters.live_instances(), 0);
    assert_eq!(counters.active_root_executions(), 0);
    assert_eq!(counters.reserved_shared_memory_bytes(), 0);
    eprintln!("generated ABI v1; Wasmtime 45; host={}-{}; guest=wasm32-wasip1-threads; root+child contract calls passed", std::env::consts::ARCH, std::env::consts::OS);
}
