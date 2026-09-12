#![cfg(feature = "wasmtime-core-wasm")]

mod positive {
    use fp_bindgen::prelude::*;

    fp_import! {
        fn host_flag(flag: bool) -> bool;
    }

    fp_export! {
        fn guest_total(total: u64) -> u64;
    }

    #[test]
    fn canonical_macro_materializes_scalar_output() {
        let output = std::env::temp_dir().join(format!(
            "fp-bindgen-wasmtime-core-wasm-positive-{}",
            std::process::id()
        ));
        fp_bindgen!(BindingConfig {
            bindings_type: BindingsType::RustWasmtimeCoreWasm,
            path: output.to_str().unwrap(),
        });
        assert!(output.join("src/lib.rs").is_file());
        assert!(output.join("wasmtime45_host_linker.rs").is_file());
        let _ = std::fs::remove_dir_all(output);
    }
}

mod rejected {
    use fp_bindgen::prelude::*;

    fp_import! {
        fn host_bulk(value: String);
    }

    fp_export! {}

    #[test]
    #[should_panic(
        expected = "fp-bindgen binding generation failed: import function `host_bulk` has unsupported argument `value` type `String`"
    )]
    fn canonical_macro_reports_rejected_declaration() {
        fp_bindgen!(BindingConfig {
            bindings_type: BindingsType::RustWasmtimeCoreWasm,
            path: "target/fp-bindgen-wasmtime-core-wasm-rejected",
        });
    }
}
