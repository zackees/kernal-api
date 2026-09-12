//! Issue #36: characterize the actual pinned legacy encoding before replacing
//! it. The historical SPEC's 24-bit claim differs from its executable helper.

#[path = "fixtures/legacy_mem.rs"]
mod legacy_mem;

#[test]
fn pinned_legacy_encoder_preserves_lengths_across_the_documented_24_bit_limit() {
    // A Wasm32 offset, never a dereferenced host pointer.
    let offset = std::ptr::without_provenance::<u8>(0x1234_5678);
    for length in [0x00ff_ffff, 0x0100_0000, 0x0100_0001, u32::MAX] {
        let (decoded_offset, decoded_length) =
            legacy_mem::from_fat_ptr(legacy_mem::to_fat_ptr(offset, length));
        assert_eq!(decoded_offset, offset);
        assert_eq!(decoded_length, length);
    }
}

#[test]
fn a_seventeen_mib_value_remains_one_whole_value_in_the_legacy_encoding() {
    let payload = vec![0x5a_u8; 17 * 1024 * 1024];
    let offset = std::ptr::without_provenance::<u8>(0x1000);
    let encoded = legacy_mem::to_fat_ptr(offset, payload.len().try_into().unwrap());
    let (_, decoded_length) = legacy_mem::from_fat_ptr(encoded);
    assert_eq!(decoded_length as usize, payload.len());
    assert!(decoded_length > 16_777_215);
    // This characterizes packing, not a full historical RPC execution. There
    // are no chunk credits, resource generations, or cancellation fields in
    // this representation; accepting this length proves no streaming policy.
    assert_eq!(encoded >> 32, 0x1000);
}

mod generated_boundary {
    use fp_bindgen::prelude::*;

    fp_import! {
        fn whole_value(value: String);
    }
    fp_export! {}

    #[test]
    #[should_panic(
        expected = "fp-bindgen binding generation failed: import function `whole_value` has unsupported argument `value` type `String`"
    )]
    fn scalar_generator_rejects_a_legacy_whole_value_declaration() {
        // Validation runs before any output is created. This deliberately
        // rejected declaration must never become an alternate guest ABI.
        fp_bindgen!(BindingConfig {
            bindings_type: BindingsType::RustWasmtimeCoreWasm,
            path: "target/rejected-whole-value-interface",
        });
    }
}
