//! Manifest-level guard: ordinary users must not inherit the sketch host.

#[test]
fn wasm_engine_is_opt_in_and_not_part_of_full() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).expect("Cargo.toml");

    assert!(
        manifest.contains("wasm-sketch-host = [\"dep:wasmtime\"]"),
        "the Wasm engine must remain behind its explicit feature"
    );
    let full_start = manifest.find("full = [").expect("full feature");
    let full_end = manifest[full_start..]
        .find("]\n\n[dependencies]")
        .expect("end of full feature")
        + full_start;
    assert!(
        !manifest[full_start..full_end].contains("wasm-sketch-host"),
        "full must not pull the heavyweight Wasm host"
    );
    assert!(
        manifest.contains("wasmtime = { version = \"=45.0.0\", default-features = false"),
        "Wasmtime must be exact-pinned with defaults disabled"
    );
    assert!(
        !manifest.contains("wasmtime-wasi"),
        "the sketch host must not use broad ambient WASI"
    );
}
