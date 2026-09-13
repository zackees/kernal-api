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
world and executes three isolated instances: a 64 MiB stream, a producer trap
after exactly one 64 KiB chunk, and host-call cancellation after observing a
pending second read. It grants one synthetic blob and permits one
stream, with no filesystem/network/WASI host imports. The host produces at most
the reader's capacity and 64 KiB per call; no whole-payload vector is created.
Normal return requires an empty blob table and matching host/guest byte counts.
All three runs require zero live blob and producer objects after
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
This is Linux x86-64 fixture evidence, not a public-facade comparison, guest-side
read/write cancellation proof, slow-consumer test, six-target validation, or
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
