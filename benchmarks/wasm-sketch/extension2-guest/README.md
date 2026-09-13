# Extension2 guest contract — RED, not runtime acceptance

This source-only experiment makes the missing #13 guest capability concrete.
The `guest-proof` binary currently **does not compile**: the public facade has
no `ArchiveEntry::open`. The optional binary is an explicit unfinished acceptance
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
passes (5.31 s). The original guest binary failed on the missing facade import
with E0432; inventory is now implemented, but entry streaming remains absent.
These library checks are not artifact execution evidence.

## Header-only actual guest control

The separate `header-proof` feature executes the public grant and bounded
original-header read in a Cargo-built Rust Wasm guest. The host retains the
file and synthetic key; its native grant is test-only. No plaintext is exposed.
The Linux x86-64 execution test passes valid identity, wrong identity, and
missing grant cases, with zero live resources and pending operations after
each fresh admitted execution. A sparse 17 MiB tail is deliberately **not a
valid encrypted ZIP**: this proves header policy execution, not authentication.

```sh
SOLDR_LINKER=default soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml \
  --features header-proof --bin kernal-extension2-guest-proof \
  --target wasm32-wasip1-threads --release --target-dir target/extension2-header-proof -j1
cp target/extension2-header-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.wasm \
  target/extension2-header-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm
soldr cargo build --locked --manifest-path tools/wasm-abi-generator/Cargo.toml
tools/wasm-abi-generator/target/debug/kernal-api-wasm-abi-generator \
  --embed-threaded-metadata target/extension2-header-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm
KERNAL_EXTENSION2_HEADER_WASM="$PWD/target/extension2-header-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm" \
  soldr --no-cache cargo test --locked --features wasm-sketch-host,archive-auth-test-support \
  --lib authenticated_input_actual_guest_validates_header_and_rejects_missing_or_wrong_identity \
  -j1 -- --ignored
```

Operation protocol revision 4 adds inventory/metadata/entry-drop to revision
3's authentication submissions and revision 2's grant/header/abandon submissions. Rebuild guest code before
embedding its metadata; never relabel an older binary.
The header control also awaits a host timer to exercise the complete admitted
async lifecycle; the bounded header copy itself finishes synchronously.

## Authentication guest control

`auth-proof` now authenticates a real ZIP containing a 17 MiB payload through
the public guest facade and closes the resulting opaque archive. Guest policy
decodes the explicit nonce; the host retains the key and original AAD, performs
bounded decryption on its tracked blocking lane, and returns archive authority
only after the final tag succeeds. The input is consumed once. Dropping the
authentication future abandons pending work or its uncollected result without
allocating another operation slot; dropping the archive revokes its storage.

```sh
SOLDR_LINKER=default soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml \
  --features auth-proof --bin kernal-extension2-guest-proof \
  --target wasm32-wasip1-threads --release --target-dir target/extension2-auth-proof -j1
cp target/extension2-auth-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.wasm \
  target/extension2-auth-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm
soldr cargo build --locked --manifest-path tools/wasm-abi-generator/Cargo.toml
tools/wasm-abi-generator/target/debug/kernal-api-wasm-abi-generator \
  --embed-threaded-metadata target/extension2-auth-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm
KERNAL_EXTENSION2_AUTH_WASM="$PWD/target/extension2-auth-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm" \
  soldr --no-cache cargo test --locked --features wasm-sketch-host,archive-auth-test-support \
  --lib authenticated_input_actual_guest_authenticates_large_zip_and_rejects_bad_tag_or_nonce \
  -j1 -- --ignored
```

Linux x86-64 execution passes valid authentication, corrupted tag, and wrong
nonce cases in 11.27 s. Each ends with zero staging bytes, authentication jobs,
resources, and operations. Native tests separately cover pending-future
abandonment and abandonment after successful authentication before collection.
This authentication-only control does **not** enumerate or read entries.

## Inventory guest control

`inventory-proof` additionally enumerates the authenticated ZIP through the
public facade, applies bounded name/size/duplicate policy, and requires exactly
one `payload` record of 17 MiB. It does not read the entry body. Protocol 4
returns independently owned entry handles: closing enumeration retains staging
while an entry remains live; dropping the last entry releases that storage.
ZIP metadata I/O runs on the existing tracked blocking lane, never an import.

Build using the authentication commands above, substituting `inventory-proof`
for `auth-proof` in feature and target directory, and set
`KERNAL_EXTENSION2_INVENTORY_WASM` when running the ignored test
`authenticated_input_actual_guest_enumerates_large_zip_and_rejects_bad_tag_or_nonce`.
The freshly built revision-4 guest passes valid ZIP, bad-tag, and wrong-nonce
cases on Linux x86-64 in 13.60 s, with all four teardown counters zero.
Native tests cover foreign/stale entry rejection, short metadata buffers,
entry lifetime after parent close, resource-quota cursor rollback, and dropping
an inventory future after completion but before collecting its entry.
The full contract remains RED solely at `ArchiveEntry::open` (E0599, Soldr
log `20260913T075448Z-home-niteris-dev-kernal-api.xml`).

The executable contract requires a host-granted encrypted input, a bounded
header read, asynchronous authentication, bounded inventory, and entry-to-Blob
streaming through `kernal_api::guest`. The guest accepts exactly one `payload`
entry and verifies every byte of its 17 MiB body with a 64 KiB buffer. The host
must keep the original header as AAD and expose no archive or entry resource
before the final tag succeeds. The method names are a proposed semantic
contract; entry streaming dispatch is still absent.

To turn this RED into runtime evidence, add the generated operations and
host-granted input, construct and encrypt the large synthetic ZIP, build and
admit the actual Wasm artifact, execute success and corrupted-tag variants,
and assert transfer/storage peaks and zero resources at teardown. In particular,
the host must verify that ciphertext itself exceeds 16 MiB; this source only
asserts the decompressed payload length. Feed encrypted-size/SHA-256 agreement,
full release/XPI policy, cancellation, worker containment, six-native-target
execution, and exact published dependency pins remain acceptance gaps.
