#![cfg(feature = "wasm-sketch-host")]

use kernal_api::async_engine::{RuntimeBuilder, RuntimeHandle};
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchModulePolicy, ThreadedRootOutcome,
};

#[test]
fn admitted_threaded_profile_executes_its_start_once_with_a_facade_runtime_handle() {
    let bytes = threaded_root_wasm(false);
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy");
    let sketch = compiler.admit(&bytes, policy).expect("admission");
    let runtime = RuntimeBuilder::current_thread().enable_all().build().expect("runtime");

    let outcome = runtime.run(async {
        sketch.execute_threaded_root(RuntimeHandle::current().expect("runtime handle"))
    });
    assert_eq!(outcome.expect("root execution"), ThreadedRootOutcome::Started);
    assert_eq!(compiler.compiled_module_count(), 1, "root execution must not compile again");
}

#[test]
fn proc_exit_zero_is_a_controlled_root_completion() {
    let bytes = threaded_root_wasm(true);
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy");
    let sketch = compiler.admit(&bytes, policy).expect("admission");
    let runtime = RuntimeBuilder::current_thread().enable_all().build().expect("runtime");

    let outcome = runtime.run(async {
        sketch.execute_threaded_root(RuntimeHandle::current().expect("runtime handle"))
    });
    assert_eq!(outcome.expect("normal proc_exit"), ThreadedRootOutcome::Exited);
}

fn leb(mut value: u32, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 { byte |= 0x80; }
        output.push(byte);
        if value == 0 { return; }
    }
}
fn text(value: &str, output: &mut Vec<u8>) { leb(value.len() as u32, output); output.extend(value.as_bytes()); }
fn section(id: u8, contents: Vec<u8>, output: &mut Vec<u8>) { output.push(id); leb(contents.len() as u32, output); output.extend(contents); }
fn custom(name: &str, contents: &[u8], output: &mut Vec<u8>) { let mut body = Vec::new(); text(name, &mut body); body.extend(contents); section(0, body, output); }

// Minimal structurally valid ThreadedRustV1 artifact. The start function is
// intentionally unexported: Wasmtime runs it during instantiate, not by
// calling exported `_start` or `kernal-api-run` separately.
fn threaded_root_wasm(proc_exit: bool) -> Vec<u8> {
    let mut wasm = b"\0asm\x01\0\0\0".to_vec();
    let mut types = Vec::new();
    leb(9, &mut types);
    types.extend([
        0x60, 0, 0, 0x60, 1, 0x7f, 1, 0x7f,
        0x60, 3, 0x7f, 0x7e, 0x7f, 1, 0x7f,
        0x60, 2, 0x7f, 0x7f, 1, 0x7f,
        0x60, 4, 0x7f, 0x7f, 0x7f, 0x7f, 1, 0x7f,
        0x60, 1, 0x7f, 0, 0x60, 0, 1, 0x7f,
        0x60, 0, 1, 0x7f, 0x60, 2, 0x7f, 0x7f, 0,
    ]);
    section(1, types, &mut wasm);

    let mut imports = Vec::new(); leb(9, &mut imports);
    text("env", &mut imports); text("memory", &mut imports); imports.extend([2, 3]); leb(17, &mut imports); leb(16_384, &mut imports);
    text("kernal-api:v1", &mut imports); text("kernel-yield", &mut imports); imports.extend([0, 0]);
    text("wasi", &mut imports); text("thread-spawn", &mut imports); imports.extend([0, 1]);
    for (name, ty) in [("clock_time_get", 2), ("environ_get", 3), ("environ_sizes_get", 3), ("fd_write", 4), ("proc_exit", 5), ("sched_yield", 6)] {
        text("wasi_snapshot_preview1", &mut imports); text(name, &mut imports); imports.push(0); leb(ty, &mut imports);
    }
    section(2, imports, &mut wasm);
    let mut functions = Vec::new(); leb(5, &mut functions); for ty in [0, 7, 8, 7, 0] { leb(ty, &mut functions); } section(3, functions, &mut wasm);
    let mut exports = Vec::new(); leb(5, &mut exports);
    text("memory", &mut exports); exports.push(2); leb(0, &mut exports);
    for (name, index) in [("_start", 8), ("__main_void", 9), ("wasi_thread_start", 10), ("kernal-api-run", 11)] { text(name, &mut exports); exports.push(0); leb(index, &mut exports); }
    section(7, exports, &mut wasm);
    section(8, vec![12], &mut wasm);
    let mut code = Vec::new(); leb(5, &mut code);
    code.extend([2, 0, 0x0b, 4, 0, 0x41, 0, 0x0b, 2, 0, 0x0b, 4, 0, 0x41, 0, 0x0b]);
    if proc_exit {
        // Imported function index 6 is `wasi_snapshot_preview1::proc_exit`.
        code.extend([6, 0, 0x41, 0, 0x10, 6, 0x0b]);
    } else {
        code.extend([2, 0, 0x0b]);
    }
    section(10, code, &mut wasm);
    let mut features = Vec::new();
    let names = ["atomics", "bulk-memory", "bulk-memory-opt", "call-indirect-overlong", "extended-const", "multivalue", "mutable-globals", "nontrapping-fptoint", "reference-types", "sign-ext"];
    leb(names.len() as u32, &mut features); for name in names { features.push(b'+'); text(name, &mut features); }
    custom("target_features", &features, &mut wasm);
    wasm
}
