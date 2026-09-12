# Historical FatPtr evidence for #36

`legacy_mem.rs` copies the executable helper from
[the pinned fork revision](https://github.com/zackees/fp-bindgen/blob/4e44d9e5408653e3c428ee3f855cc194d53f60b0/fp-bindgen-support/src/common/mem.rs).
The adjacent license is the upstream MIT license. It is test-only source and
does not introduce the legacy support crate or its runtime dependencies.

The same revision's [historical specification](https://github.com/zackees/fp-bindgen/blob/4e44d9e5408653e3c428ee3f855cc194d53f60b0/docs/SPEC.md#fat-pointers)
describes a 24-bit length and a 16,777,215-byte ceiling. The executable helper
instead preserves all 32 length bits. The tests exercise both sides of the
documented boundary and an allocated 17 MiB value; reporting a reproduced
16 MiB truncation would therefore be incorrect. This checks the packing helper,
not the full historical MessagePack/Wasmer RPC pipeline.

The incompatibility remains: this encoding represents a whole memory value,
without scoped resource identity, chunk credits, or cancellation. A negative
generation test proves that the selected scalar backend rejects a whole-value
`String` declaration. Future bulk transfers must use the kernel's bounded
resource protocol, rather than retain this legacy transport.

Run with `soldr cargo test --locked --manifest-path tools/wasm-abi-generator/Cargo.toml --test historical_fat_ptr`.
