# fp-bindgen vendor provenance

This directory is a minimal source vendor of `zackees/fp-bindgen` commit
`4e44d9e5408653e3c428ee3f855cc194d53f60b0`, obtained from its GitHub source
archive. It includes only `fp-bindgen`, `macros`, the workspace manifest, and
the upstream Apache-2.0/MIT license texts.

Local deltas in `fp-bindgen/src/generators/wasmtime_core_wasm.rs`:

- Import the generated crate's conversion helpers into `pub mod imports` with
  `use super::*`. Upstream emitted calls such as `u32_from_i32(raw)` without
  importing them, making every nontrivial generated guest fail to compile.
- Normalize generated text to one final newline in `write_file`. The upstream
  templates can leave an extra blank final line, which fails kernal-api's
  checked-in drift gate.

Neither delta changes an ABI, transport, or generator backend behavior.

The patched generator file has SHA-256
`46347f10092c57ee343852c2e3f16a2eece76574e3235c3439e174c0ef44b1c3`.
A comparison against the immutable GitHub source showed exactly that one
template-line change. The workspace manifest is retained for inherited package
metadata; this deliberately partial vendor tree is consumed through the
isolated `tools/wasm-abi-gen` workspace, not built as the upstream workspace.
