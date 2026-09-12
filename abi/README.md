# Generated Core-Wasm contract

This directory contains the private v1 scalar control protocol. The Rust
declarations in `kernal-api-v1.rs` feed the owned fp-bindgen Wasmtime backend;
`generated/` contains its guest source, private linker, manifest, and admission
descriptor. The descriptor is projected from the emitted manifest, not a
second function list. Generated support remains private beneath guest semantic
code. Ordinary Cargo builds neither run nor depend on the generator tool.

## Explicit commands

Run from the repository root with Rust 1.95.0 and Soldr installed:

```sh
bash scripts/regenerate-wasm-abi.sh
bash scripts/regenerate-wasm-abi.sh --check
soldr cargo test --locked --manifest-path tools/wasm-abi-gen/Cargo.toml
soldr cargo test --locked --features wasm-sketch-host --test generated_abi_admission
CARGO_TARGET_DIR="$PWD/target" bash scripts/build-generated-core-smoke.sh
soldr cargo test --locked --no-default-features
```

The guest build produces a real `wasm32-wasip1-threads` command artifact,
stamps its required metadata, and executes generated calls in both root and
child Stores. The artifact test is explicitly ignored without this command;
absence of an artifact is not a passing runtime proof. Git Bash can run these
scripts on Windows. Native execution evidence must name the actual host;
cross-compilation is only compilation evidence.

Metadata stamping validates the core module header and section boundaries,
is idempotent for an exact existing declaration, and rejects conflicting or
duplicate declarations. Admission independently validates it before engine
compilation. Only the exact generated import set is admitted alongside the
existing enumerated, restricted thread/P1 compatibility surface.

## Historical RED characterization

At kernal-api commit `2aacba6`, `src/wasm/mod.rs` defines `kernel-yield`
by hand with `Linker::func_wrap`; the closed threaded admission list repeats
that handwritten import. Neither an ABI declaration source nor generated
guest/linker/manifest artifacts exists in that revision. Its yield callback
cannot submit, poll, cancel, or release a native operation and exposes no
generated resource lifecycle. This is source-level historical evidence, not
a claim that a failing test was run against the historical checkout.

The fp-bindgen source pin is
`4e44d9e5408653e3c428ee3f855cc194d53f60b0`. Its
[historical protocol specification](https://github.com/zackees/fp-bindgen/blob/4e44d9e5408653e3c428ee3f855cc194d53f60b0/docs/SPEC.md)
reserves only 24 bits for a single encoded value's length. Thus its documented
maximum is `(1 << 24) - 1 = 16,777,215`; the next byte count sets a reserved
bit and is outside that documented contract.

There is an important discrepancy: the actual
[legacy memory helpers](https://github.com/zackees/fp-bindgen/blob/4e44d9e5408653e3c428ee3f855cc194d53f60b0/fp-bindgen-support/src/common/mem.rs)
at that revision mask 32 length bits, not 24. They therefore do **not** enforce
the documented 16 MiB boundary. It would be false to report a runtime rejection
at 16,777,216 bytes. Neither the documented 24-bit limit nor the implementation's
wider whole-value representation supplies bounded streaming or lifecycle
control. The selected generator rejects non-scalar declarations; no FatPtr,
serialization, pointer/length workaround, or legacy runtime is used here.

## Compatibility and ownership

The vendored generator is an isolated development tool, not a root dependency
or shipped runtime backend. Its exact provenance, licensing, and source patch
are recorded in `vendor/fp-bindgen/PROVENANCE.md`. Generated output is always
recreated from source, including the fix for the upstream helper-scope defect.

V1 reserves only fixed scalar request and completion fields. Host context
selects scope. Numeric values never grant capability rights. No operation
accepts paths, platform objects, arbitrary bytes, or serialized bulk values.
Issue 37 supplies the sole asynchronous terminal lifecycle; issue 38 supplies
generation-safe resource registries; issue 17 later adds bounded transfer.
