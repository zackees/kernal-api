# Private Core-Wasm ABI generator

`soldr cargo run --locked --manifest-path tools/wasm-abi-generator/Cargo.toml`
regenerates the checked-in scalar v1 guest crate, private Wasmtime linker, and
ABI manifest in `src/wasm/generated/v1` from the declaration in `src/main.rs`.
The guest package lives in its `guest/` subdirectory so Cargo includes the
private host files when packaging the facade.

`ci/check_wasm_abi_generation.sh` regenerates into a temporary directory and
compares every generated file. It is the required drift check and leaves the
working tree untouched.

The threaded build scripts copy Cargo's Wasm output and embed the generated
`kernal-api.abi` custom section in that copy. Its contents bind the complete
manifest and the declared capability set. Admission checks these bytes before
compilation. Embedding parses real section boundaries, leaves an already
matching artifact unchanged, and rejects mismatched or duplicate sections.
Changes to the ABI therefore require no manual edits to the guest artifact.

Run `soldr cargo test --locked --manifest-path tools/wasm-abi-generator/Cargo.toml`
for malformed-manifest/metadata, idempotency, and historical transport checks.

The tool pins `zackees/fp-bindgen` at `4e44d9e5408653e3c428ee3f855cc194d53f60b0`.
It is a development tool outside the published package, so ordinary
`kernal-api` builds do not resolve the generator or its dependency graph.

This initial generated contract has one scalar, authority-free import:
`kernal-api:v1::kernel_yield`. Future submit/poll/yield/cancel and resource
operations must extend this declaration and regenerate all outputs; they may
not add a handwritten guest import path.
