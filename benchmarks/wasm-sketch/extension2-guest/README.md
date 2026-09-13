# Extension2 guest contract — RED, not runtime acceptance

This source-only experiment makes the missing #13 guest capability concrete.
The `guest-proof` binary currently **does not compile**: the public facade has
no `EncryptedArchive`. The optional binary is an explicit unfinished acceptance
contract, not a shipped example or a passing Wasm proof. Do not replace its
kernel calls with host-side orchestration or cite library tests as guest GREEN.

```sh
soldr cargo check --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml \
  --features guest-proof --bin kernal-extension2-guest-proof \
  --target wasm32-unknown-unknown --target-dir target/extension2-guest-proof
```

The policy library independently checks bounded original `TWPV1AES` prefix and
JSON header bytes, synthetic schema/algorithm/version/commit/key identity,
96-bit nonce, duplicate names, strict relative paths, and count/size bounds.
The fixture also caps each entry name at 4 KiB; host inventory records must
enforce that bound before transferring or allocating a guest-visible name.
It contains a manually adapted subset of decisions inspected in extension2's
`tw-orange-preview-source/src/implementation.rs` at
`f1e1173b3136a46d1ca778840a7b775411581096`. It does **not** execute the unchanged
upstream policy crate. No private key was copied; the host fixture must use a
synthetic key. The 32 MiB per-entry ceiling is a conservative fixture policy
borrowed from extension2's nested-XPI limit, not a claim that its outer preview
decoder imposes that same ceiling.

```sh
soldr cargo test --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml --lib \
  --target-dir target/extension2-guest-proof
soldr cargo clippy --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml --lib \
  --target-dir target/extension2-guest-proof -- -D warnings
```

Recorded on Linux x86-64: both policy tests pass (0.03 s), strict library
Clippy passes, and a `cargo check --lib` using the same Wasm target and lock
passes (5.31 s). The guest binary instead fails on the missing facade import
with E0432. These are compile/policy controls, not artifact execution evidence.

The executable contract requires a host-granted encrypted input, a bounded
header read, asynchronous authentication, bounded inventory, and entry-to-Blob
streaming through `kernal_api::guest`. The guest accepts exactly one `payload`
entry and verifies every byte of its 17 MiB body with a 64 KiB buffer. The host
must keep the original header as AAD and expose no archive or entry resource
before the final tag succeeds. The method names are a proposed semantic
contract; generated bindings and host dispatch are still absent.

To turn this RED into runtime evidence, add the generated operations and
host-granted input, construct and encrypt the large synthetic ZIP, build and
admit the actual Wasm artifact, execute success and corrupted-tag variants,
and assert transfer/storage peaks and zero resources at teardown. In particular,
the host must verify that ciphertext itself exceeds 16 MiB; this source only
asserts the decompressed payload length. Feed encrypted-size/SHA-256 agreement,
full release/XPI policy, cancellation, worker containment, six-native-target
execution, and exact published dependency pins remain acceptance gaps.
