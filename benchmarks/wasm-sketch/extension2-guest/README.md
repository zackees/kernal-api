# Extension2 guest streaming proof — not full runtime acceptance

This source-only experiment exercises the #13 public guest archive capability.
The `guest-proof` binary now compiles with authentication, inventory, and
entry-to-Blob streaming. It remains a synthetic policy fixture, not the full
upstream extension2 application or final runtime acceptance. Do not replace
its kernel calls with host-side orchestration or cite library tests as guest evidence.

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
with E0432; inventory and entry streaming now compile through the public facade.
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

Operation protocol revision 5 adds entry-to-Blob open to revision 4's
inventory/metadata/entry-drop and revision
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
The prior full contract was RED at `ArchiveEntry::open` (E0599, Soldr log
`20260913T075448Z-home-niteris-dev-kernal-api.xml`); protocol 5 implements it.

## Streaming guest proof

The independent `wasm-archive-native` six-host CI matrix invokes the same
build-and-execute runner:

```sh
uv run --no-project -m unittest ci.test_run_extension2_guest ci.test_native_proof_jobs
uv run --no-project ci/run_extension2_guest.py \
  --native-target x86_64-unknown-linux-gnu --target-dir "$PWD/target"
```

Install `wasm32-wasip1-threads` in the pinned toolchain first. Substitute the
actual native Rust host triple on other hosts. The runner rejects a cross-target,
selects one Cargo-reported executable per build, copies fresh guest code before
embedding metadata, and rejects a missing or zero-test proof. Adding this gate
is not evidence that all six hosts have passed; inspect the matrix results.

Archive staging and actual guest execution have a separate 30-minute job
deadline from the six screenshot jobs. The previous combined macOS x86-64
job passed the archive guest, then timed out building the screenshot harness
(run `34748066733`). Separating the jobs retains both native matrices without
making their cold builds compete for one deadline. Workflow regression tests
check both host sets and the archive job's independent guest setup.

At `7a23fdb`, run `34749620965` passed all six screenshot jobs and five
archive jobs. Intel macOS archive job `103703608908` hit the 30-minute deadline:
native staging tests finished at 10:00:20 UTC, but the helper's second native
harness build took another 8m39s. Actual guest execution began at 10:12:29 and
was cancelled at 10:13:07 without a result. This is incomplete execution, not
a passing archive proof or an observed guest assertion failure.

The helper now owns the single native harness build and runs both the existing
`authenticated_` staging tests and the exact ignored guest test from it. The
staging gate rejects zero executed tests, and the actual guest gate still
requires exactly one passing execution. No native target, test gate, or timeout
was removed. Runner regression tests first failed on the missing staging call
and duplicate workflow build, then passed after this change.
The combined helper passed locally on Linux x86-64 at `eb612e2`, with freshly
built revision-6 guest metadata: 29 staging tests passed (four existing ignored)
in 0.57 s, followed by the one actual guest test in 37.41 s. The native harness
was built once (416.25 s). This validates the revised runner locally; it does
not establish completion of the Intel macOS job or six-host revision-6 parity.
The archive job uses the screenshot builder's `-PrepareTargetOnly` mode to
reuse verified core/std target repair without building any screenshot artifact.

Build with `--features guest-proof` and target directory
`target/extension2-stream-proof`, copy and embed metadata as above, then run:

```sh
KERNAL_EXTENSION2_STREAM_WASM="$PWD/target/extension2-stream-proof/wasm32-wasip1-threads/release/kernal-extension2-guest-proof.admitted.wasm" \
  soldr --no-cache cargo test --locked --features wasm-sketch-host,archive-auth-test-support \
  --lib authenticated_input_actual_guest_streams_large_zip_and_rejects_bad_tag_or_nonce \
  -j1 -- --ignored
```

The fixture uses 64 KiB chunks, a 128 KiB per-Blob ceiling, a 256 KiB aggregate
Blob ceiling, and a fixed total transfer-capacity budget. It checks both peak
counters against the configured limits and requires zero retained capacity,
archive jobs, staging bytes, resources, and operations at teardown. This is
transfer accounting, not aggregate RSS measurement. The root has a finite
500-million-instruction workload budget; the unchanged 100k compatibility
default correctly exhausted during the first full-payload execution.
The freshly built revision-5 Wasm passes valid ZIP, corrupted-tag, and
wrong-nonce cases on Linux x86-64 in 12.73 s, including these peak and teardown
assertions. This does not replace six-target execution or upstream policy proof.
The same full-guest test now additionally rejects an authenticated unexpected
name, traversal path, oversized entry (32 MiB + 1), and incorrect payload bytes.
The seven-case Linux run passes in 31.31 s. Policy/size/path failures expose no
Blob payload; the incorrect-content case reaches guest byte validation, then
reclaims its producer and storage on nonzero guest exit. Native path/size
rejection and guest product-name/content rejection are distinct evidence.

An eighth case supplies a valid 17 MiB first payload followed by 16,384
uniquely named empty entries. The resulting 16,385-entry archive must fail
native inventory admission before any Blob payload is exposed. Requiring a
zero Blob peak distinguishes count rejection from the guest merely rejecting
the second entry's product name after streaming the first entry. This is a
native count-limit proof, not execution of the guest's own count-policy limit.
The ZIP writer retains fixture metadata, while payload generation and
encryption continue using fixed 64 KiB buffers.
The eight-case Linux x86-64 run passes in 37.11 s. A temporary mutation raising
the native ceiling to 16,385 fails the zero-peak assertion with 131,072 bytes
exposed (39.58 s); the source ceiling was restored to 16,384 afterward. This
demonstrates that the new case detects a relaxed native count limit rather
than passing solely because the guest eventually rejects an extra name.

The reader is sequential: drain or drop the open Blob before advancing
inventory or opening another entry. Entry handles independently retain storage,
but do not promise concurrent stream progress. Closing the parent archive does
not revoke an already-open stream; dropping that Blob stops its producer.

The executable contract requires a host-granted encrypted input, a bounded
header read, asynchronous authentication, bounded inventory, and entry-to-Blob
streaming through `kernal_api::guest`. The guest accepts exactly one `payload`
entry and verifies every byte of its 17 MiB body with a 64 KiB buffer. The host
must keep the original header as AAD and expose no archive or entry resource
before the final tag succeeds. The method names are a proposed semantic
contract now implemented for this synthetic source-only fixture.

The host verifies the encrypted ZIP length exceeds 16 MiB. Feed encrypted-size/SHA-256 agreement,
full release/XPI policy, cancellation, worker containment, six-native-target
execution, and exact published dependency pins remain acceptance gaps.
