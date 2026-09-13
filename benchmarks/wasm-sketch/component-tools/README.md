# Private component encoder probe

This standalone, unpublished workspace encodes the sibling guest's core Wasm
using pinned `wit-component = 0.251.0`, matching wit-bindgen 0.58's metadata
format. By default it does not instantiate a component or grant effects.

The optional `engine-probe` feature additionally compiles the encoded component
with the production runtime's pinned Wasmtime 45.0.0, enabling Component Model
async support. It still does not instantiate or execute it. Add
`--features engine-probe` to the Cargo commands below to require this extra
check. Compilation errors occur before the output is created. Without the
feature, only structural validation is performed; the success output states
explicitly when engine compilation has also passed.

This experimental tool has its own lockfile and does not change the parent
crate's features or dependencies. Wasmtime is optional here and no second
production runtime fallback is introduced.

## Opt-in stream execution

`--features execution-probe` additionally links the exact generated private
world and executes five isolated instances: a 64 MiB stream, a producer trap
after exactly one 64 KiB chunk, host-call cancellation after observing a
pending second read, guest-issued read cancellation followed by a fresh
transfer in that same instance, and a fast-producer/slow-consumer transfer.
It grants one synthetic blob and permits one stream at a time, with no
filesystem/network/WASI host imports. The first four scenarios produce at most
the reader's capacity and 64 KiB per call. The slow-consumer scenario deliberately
offers bounded 128 KiB batches to 64 KiB reads to exercise retained-buffer
backpressure. No whole-payload vector is created.
Normal return requires an empty blob table and matching host/guest byte counts.
All five runs require zero live blob and producer objects after
store teardown. The probe uses the kernel runtime builder and timeout wrapper,
not a direct Tokio dependency or a second executor implementation. Each run has
a 30-second async timeout, 100 million fuel units, and an 8 MiB per-memory limit.
These bounds are not a complete hostile-component admission policy.

Build the guest, then add `--features execution-probe` to the encoder command
below, using a new output path. Run the actual-artifact regression explicitly:

```sh
KERNAL_COMPONENT_PROBE=/absolute/path/to/probe.component.wasm \
  soldr --no-cache cargo test --locked \
  --manifest-path benchmarks/wasm-sketch/component-tools/Cargo.toml \
  --features execution-probe --target-dir benchmarks/wasm-sketch/component-tools/target \
  -j1 -- --include-ignored
```

The artifact test is explicitly ignored without this command, because Cargo
does not rebuild the independent Wasm guest. The local path dependency on the
exact `kernal-api = 0.1.0` source is experimental/migration-only and must be
replaced by a matching published pin before release.

The first instantiation rejected synchronous WIT methods paired with concurrent
host bindings (`[method]blob.read` async type mismatch). Making the private WIT
methods explicitly async and awaiting them in the guest made exact-world
instantiation pass, without weakening linker type checks. The rebuilt component
is 60,841 bytes; the two-scenario output is retained at
`/tmp/kernal-component-execution-probe-2.wasm`, SHA-256
`4d762703cb75c1c5f684443fdcec6bf111964ac45b660c75446995218cedc141`.
This earlier artifact is Linux x86-64 fixture evidence, not a public-facade comparison,
write cancellation proof, slow-consumer test, six-target validation, or
go/no-go selection.

The pending-call test uses the same component bytes. Its producer supplies one
64 KiB chunk and then returns `Poll::Pending` without producing more bytes. The
driver polls the real component call and drops it only after observing both the
producer's pending marker and a pending outer future. It then tears down the
store, verifies that exactly one chunk was produced, and checks that live blob
and producer counts are zero. Cancellation is observation-driven, not timed
with a sleep; the 30-second timeout remains only a failure guard.

The focused test initially failed with `call completed before a pending read`
when the producer still ran to completion. Adding the deliberately pending mode
made the same test pass. This proves cleanup after abandoning a pending host
call and destroying its store, not a guest-issued cancellation handshake or
continued use of the same instance after cancellation.

The additional guest-cancellation fixture exports `cancel-read`. It reads one
64 KiB chunk, polls the second read to Pending, then uses a **test-only** async
checkpoint in the private WIT to wait until the host has also observed Pending.
The checkpoint uses the kernel executor's yield operation and adds no ambient
capability; it is instrumentation, not a proposed public API. The guest calls
the binding's explicit read-cancellation operation and requires a Cancelled
result with an empty returned buffer, then drops the stream and blob.

Before any store destruction, the host checks the pending and cancellation
markers, exactly one produced chunk, an empty resource table, and zero live
blob/producer objects. It then explicitly grants another blob and runs the
64 MiB transfer through the **same** Store and component instance. This proves
guest-issued pending-read cancellation and subsequent instance reuse for this
fixture, but not pending writes, cancellation races, or slow-consumer behavior.

The test first failed when `cancel-read` simply performed a full transfer.
The actual cancellation implementation passed the same focused test. Its
updated component is 68,382 bytes, retained at
`/tmp/kernal-component-guest-cancel-green.wasm`, SHA-256
`1725402199c4a68725e6626d7b114203499b3c24af58acd413cca1f26d41f6f3`.
Use this rebuilt world for current artifact tests; earlier components lack the
new export and cannot satisfy the generated host's exact-world checks.

The `slow-consumer` export reads through the same reusable 64 KiB buffer but
calls a test-only pause checkpoint after consuming its first 64 KiB. In this
scenario the host offers a 128 KiB batch, leaving 64 KiB retained by the runtime.
The checkpoint verifies exactly one batch was produced, awaits a 20 ms sleep
through the kernel executor, and verifies both byte and batch counters stayed
unchanged. Once the guest resumes reading, it must finish exactly 64 MiB in
512 batches, with a recorded maximum batch of 128 KiB and normal cleanup.
This checks stop/resume behavior with a nonempty retained buffer; it is not a
measurement of aggregate process memory or all concurrent-stream schedules.

The focused test failed with `consumer never paused at buffered capacity` when
the export used the unpaused consumer. Enabling its checkpoint made the same
test pass. The updated component is 71,378 bytes, retained at
`/tmp/kernal-component-backpressure-green.wasm`, SHA-256
`bc0adc14fa00d7131613a92aa52ca734afb1f540ca14b947fa86a87e4a21a9f8`.
Current artifact tests require this latest private world. Earlier artifacts
lack `slow-consumer`; their historical evidence above remains scoped to the
operations they actually contained.

## Encoding-only commands and earlier evidence

From the repository root:

```sh
soldr --no-cache cargo run --locked \
  --manifest-path benchmarks/wasm-sketch/component-tools/Cargo.toml \
  --target-dir benchmarks/wasm-sketch/component-tools/target -j1 -- \
  benchmarks/wasm-sketch/component-guest/target/wasm32-unknown-unknown/release/kernal_component_probe.wasm \
  /absolute/new/probe.component.wasm
soldr --no-cache cargo test --locked \
  --manifest-path benchmarks/wasm-sketch/component-tools/Cargo.toml \
  --target-dir benchmarks/wasm-sketch/component-tools/target -j1
```

The input read is bounded at 32 MiB; the output path must not exist. Encoding
and structural validation happen before creating the output. A write failure
can leave a partial output file; an unsuccessful command is never admission
evidence. The validator requires a component with exactly one top-level imported
instance named `kernal:probe/blobs@0.1.0` and rejects ambient/other interfaces.
Tests also reject a core module and an empty component.

This is not production admission: validation enables all parser features,
does not constrain internal memory or instructions, and does not verify the
imported instance's complete semantic type or the exported entry point. The
future host must link against the exact generated world and enforce the same
quotas and lifecycle rules as the core candidate before comparison. Structural
validity does not establish Wasmtime 45 compatibility or guest correctness.

The first Linux x86-64 toolchain run encoded and validated a 52,326-byte
component, with no WASI adapter and one top-level kernel interface import.
The guest's initial missing-trait implementation failed compilation (E0277);
adding the async stream consumer made the non-ambient Rust 1.95.0 build pass.
The initial encoder compilation caught the new `ComponentExternName.name`
API shape; the corrected encoder completed successfully. No timings here are
reference-host edit samples and no execution or selection is claimed.

The retained local component is `/tmp/kernal-component-probe-1.wasm`, SHA-256
`1cfd6d2a1cbf26d6ec939b15c5661d2336bb4c0e9fa9ee24e1aa619e0e1d3748`.
Its input core module SHA-256 is
`fbd170175ce093fbf4b67a0c09e479cc50872a1d68c6164181bc0dda0bba8009`.
All three validator tests, strict tools/guest Clippy, formatting, and the parent
dependency-isolation check pass locally. Six-target native execution remains
unproven for this new candidate.

The subsequent `engine-probe` run successfully compiled that same 52,326-byte
component with Wasmtime 45.0.0 on Linux x86-64. Its output is retained at
`/tmp/kernal-component-engine-probe-1.wasm`. No instance was created and no
guest function was called. This proves compiler acceptance of the actual async
component encoding, not import linking, stream progress, cancellation, or
resource cleanup. The initial probe build failed on a Rust error-conversion
mismatch; preserving `wasmtime::Result` inside the compiler helper and converting
its diagnostic at the CLI boundary fixed that build error without changing
component bytes or weakening validation.
