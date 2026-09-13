# zccache policy sketch experiment — issue #13

## Reproduced native dependency barrier

Inspected zccache revision `a7c84de53105ce41b2060bd9dd7730ef226e78a1`
(workspace version 1.13.22) in the sister checkout
`../kernal-api-extern/zccache`. No upstream source changes were made during
this initial reproduction; coordinated prerequisites are recorded below.
On the Linux x86-64 reference host, using its pinned Rust 1.95.0 toolchain:

```sh
soldr --no-cache rustup target add wasm32-unknown-unknown
soldr cargo tree --locked -p zccache-compiler --target wasm32-unknown-unknown -i mio
soldr cargo check --locked -p zccache-compiler --target wasm32-unknown-unknown -j 1
```

The dependency query succeeds and reports:

```text
mio v1.1.1
└── tokio v1.50.0
    └── zccache-platform v1.13.22
        ├── zccache-compiler v1.13.22
        └── zccache-core v1.13.22
            └── zccache-compiler v1.13.22
```

The check exits 101 in Mio, with the primary diagnostic:

```text
This wasm target is unsupported by mio. If using Tokio, disable the net feature.
```

There are 49 Mio compilation errors; the primary diagnostic is at
`mio-1.1.1/src/lib.rs:44`. The local Soldr build record is
`20260913T024540Z-home-niteris-dev-kernal-api-extern-zccache.xml` under
`/home/niteris/.soldr/logs/builds/`. This is the required native-policy-path
RED evidence, not a successful guest build or proof that removing one
dependency makes the compiler crate portable.

An earlier attempt failed with E0463 because the Wasm target was absent.
That attempt is excluded from the portability result. Installing the target
and rerunning reached Mio's explicit unsupported-target diagnostic.

## Source constraints for the extraction

The actual source, rather than the architecture summary, establishes these
requirements for native/Wasm fixture equivalence:

- `crates/zccache-compiler/src/detect.rs` performs string-based compiler-family
  classification, including Windows-style paths, clang-cl, Emscripten, and
  Dylint drivers. Reuse these cases rather than inventing a toy classifier.
- `crates/zccache-compiler/src/parse.rs` uses `NormalizedPath` and consults
  `platform::host::is_windows()` for output policy. A guest's own compilation
  target is not the host fact that this policy needs.
- `crates/zccache-core/src/path.rs` has host-dependent normalization and key
  semantics. Replacing it with the guest standard library's path behavior
  would not prove native/Wasm equality on Windows and macOS.
- `crates/zccache-hash/src/cache_key.rs` uses the domain
  `zccache-cache-key-v2\0`, preserves argument order, and sorts environment
  and dependency maps. The checkout's architecture guidance still describes
  sorted arguments and a v1 tag; that prose must not define the experiment's
  expected keys. Kernel hashing must preserve the actual byte-update sequence.

## Explicit Rustc host-policy prerequisite

The coordinated sister branch `feat/rustc-explicit-host-policy` contains
[zccache commit 4d37833](https://github.com/zackees/zccache/commit/4d378335db604ea340e204fa50d513197415009d).
It adds `RustcHost` and `parse_rustc_invocation_with_host` to the existing
compiler crate, without copying its parser into this benchmark. The native
entry point still resolves the existing host/configuration facts and delegates
to that implementation. Host-side proc-macro/Dylint names are independent of
the requested target; an explicit target still controls executable naming.

RED: the three new tests fail with unresolved imports before the seam exists
(Soldr record `20260913T085516Z-home-niteris-dev-kernal-api-extern-zccache.xml`).
GREEN on Linux x86-64: all three explicit-host tests pass, exercising all three
host families, Dylint/test-cache admission, and original argv preservation.
The complete compiler suite passes 383 tests with no failures or ignored tests;
strict package Clippy and an independent focused Astra review also pass:

```sh
soldr --no-cache cargo test --locked -p zccache-compiler -j1
soldr --no-cache cargo clippy --locked -p zccache-compiler --all-targets -j1 -- --deny warnings
```

This prerequisite is tracked in [zccache #1580](https://github.com/zackees/zccache/pull/1580)
and merged as `348e175aa4be339ebc666bb2be4dc3dd7993af60`. After integrating
the shared encoder prerequisite, the combined compiler suite again passed
383 tests and strict all-target compiler Clippy passed. It does not remove the native
dependency graph or change `NormalizedPath`/lexical path semantics. These
results are native parser evidence, not an actual Wasm parser/key/miss proof.

## Shared request-key encoder prerequisite

[zccache #1581](https://github.com/zackees/zccache/pull/1581), commit
`c54cf8aecd9a508cd090fa32ef9e1f5569eeaa05`, extracts the actual daemon
`zccache-request-v2` byte encoding into the existing hash crate. The native
daemon now consumes that same fallible emitter with lazy argument normalization.
It preserves ordered argv, detached remap handling, raw depfile salts, and
selected sorted environment entries without collecting a whole-key buffer.
This request-cache fingerprint is not the complete artifact key described above.

Literal-byte compatibility fixtures passed against the original daemon encoder.
The new emitter tests first failed with E0432, then passed after implementation.
Linux validation: 23 hash tests, 29 daemon fingerprint/path-policy tests, and
the full daemon suite (837 passed, 28 existing ignored integration tests,
139.73 seconds). Strict all-target Clippy for the changed crates passed with
`--no-deps`; broader dependency linting stopped on three existing
`double_must_use` diagnostics in untouched protocol code. Focused formatting
and independent Astra code/documentation review passed.

The PR merged as `e1dc9f27e931300f7ba17b76ee9ffa2073abe72d`; an authoritative
PR read confirmed the merge after the command returned a transient API error.
The sink boundary is ready for a kernel hash adapter, but neither
that adapter nor portable path normalization or the Wasm dependency graph is
implemented by this prerequisite. Neither PR changes a release version.

## Bounded public guest hash control

Operation protocol revision 6 adds a facade-owned `guest::Blake3Hasher`, backed
by the kernel's existing BLAKE3 implementation. Updates are bounded to 64 KiB,
resources are store-scoped, and operation capacity is reserved before mutation.
Dropping an uncollected create reclaims its resource; abandoning an update
revokes uncertain state without replay or rollback. Finalize consumes the hash
only after validating its complete fixed-size destination.

The `kernal-hash-guest-proof` binary in the extension2 benchmark workspace is
a separate capability control, not extension2 policy. On Linux x86-64, a freshly
compiled revision-6 guest checked the empty-input digest and hashed 64 MiB
of `0x5a` twice, in 65,536-byte and
4,093-byte chunks, through the public facade. Both digests matched the native
kernel and independent `b3sum` literal. The actual guest test passed in 4.57 s,
with zero live resources and pending operations after execution and execution
limits returned to their defaults after root closure. This is correctness
evidence, not a benchmark or aggregate-memory measurement.

The operation regression suite passed 79 tests; the independent focused hash
review run passed 12 tests. Strict host Clippy, generated-ABI drift checking,
and Astra lifecycle review passed. The hash proof is opt-in: build
`kernal-hash-guest-proof` with `guest-proof` for `wasm32-wasip1-threads`, copy
the freshly built module and embed metadata using the ABI generator, then set
`KERNAL_HASH_GUEST_WASM` to that admitted copy when running the ignored
`hash_actual_guest_streams_64_mib_through_public_facade` host test.

The initial control did not connect the shared zccache request encoder to the
guest. A private Component adapter now
executes the exact shared hash policy through the same public facade on Linux
(3.52 s), including failed-export cleanup and direct canonical oversized-input
rejection. This establishes hash correctness only: canonical list lifting is
not yet bounded before allocation, Blob semantics remain a separate private
probe, and no matched runtime-performance selection has been made.

The shared guest policy now also executes the real zccache request encoder,
pinned to source commit `2543136ea8b648b295d2f7115a19656ff0854531` with default
features disabled. This encoder-only graph has no normal dependencies: hashing
continues through the public kernel capability, not guest BLAKE3. Its resumable
cursor feeds a fixed 64-KiB buffer with chunk limits of 1, 7, and 65536 bytes.
The fixture includes order-sensitive arguments, an empty argument, raw depfile
stdout salt, and sorted environment entries. Its expected digest was checked
independently with `b3sum` over the literal existing v2 byte protocol.

Fresh actual guest executions passed on Linux x86-64: Core 4.59 s and Component
3.88 s, including the existing 64-MiB hash and cleanup checks. The Component
artifact is 87242 bytes with exactly two kernel import instances. These are
correctness observations, not matched performance measurements. The source pin
is migration-only and does not satisfy published-front-door acceptance.

The freshly rebuilt revision-6 archive guest also passes all eight cases on
Linux (42.45 s). All 16 screenshot regressions also pass on Linux against
fresh revision-6 normal, trap, and blocked guests (165.78 s). These are local
compatibility results, not six-target evidence for this revision.

## Remaining proof

The actual Rustc parser is now exercised by the shared `rustc_policy.rs`
fixture, pinned to source commit `c6ddfa974a4920a127eac81773db6a5c56cd30a7`
from [zccache #1583](https://github.com/zackees/zccache/pull/1583). Native
execution and freshly built Core/Component guests pass the same source.
Coverage includes compiler detection, proc-macro host naming despite a Wasm
target, independent Unix/Windows lexical syntax, test-cache opt-in, complete
metadata output plans with unknown flags, nested Dylint cdylib policy, and
malformed nested-driver rejection. The expanded actual Core run passed in
8.35 s and Component in 7.56 s on Linux x86-64. These concurrent correctness
runs are not comparative performance evidence. The Component artifact is
148700 bytes and retains exactly two kernel import instances.

This uses the upstream policy implementation without native compiler/runtime
dependencies; only its lexical `typed-path` dependency is present. It does not
prove native Windows path equivalence, actual process execution, or a complete
cache hit/miss workflow. The source pin is migration-only, not a published
front-door dependency.

No complete compiler-policy or cache-workflow GREEN result is claimed yet. The next
implementation must reuse representative parser/key fixtures, supply host
facts explicitly, and route hashing and the controlled compiler miss through
public kernel capabilities with bounded stdout/stderr. Native and Wasm runs
must compare the same policy outputs. Daemon, watcher, IPC, and artifact
movement stay native.

### Controlled compiler-miss design constraints

Source inspection identifies `SpawnSpec::spawn_session` and `ProcessSession`
as the existing native facade seam. `ProcessSessionOptions` bounds queued
chunks and each chunk's bytes; `next_output` preserves stdout/stderr identity
and backpressures child pipes. Lifecycle wait/kill remain independent of a
pending output receive. Reuse this seam rather than adding another process
backend or executor. This is a proposed integration, not an executed guest
process proof.

The host must grant one exact command description and working directory,
with explicit environment and stream policy. A guest should receive only a
store-scoped opaque grant, not executable paths, arbitrary argv, environment
mutation, or shell access. Reserve operation/resource capacity before spawn;
revocation between reservation and attachment must reclaim a spawned session.
Output delivery needs an aggregate in-flight byte budget in addition to the
native queue limits. Cache-hit fixtures must prove no spawn occurred, while
miss fixtures must compare native and guest status and both output streams.

The facade documents that session kill/drop terminates and reaps only the
direct child. Post-exit drain grace reports abandoned descendant-held pipes;
it is not process-tree containment. Tests must distinguish direct-child
cleanup, output completion, and descendant containment, and must not claim
the last from `kill_on_drop`. Cancellation, trap, grant revocation, and
uncollected completion each need zero-resource/pending-operation checks in
both candidate adapters before this workflow can count toward #13.

The host-neutral `tests/process_session_streaming.rs` exercises the native
seam with a direct child test executable, not a shell: 32 MiB per tagged
stdout/stderr stream, a one-chunk queue, 4096-byte chunks, exact payload
counts without whole-output collection, and one EOF per stream. A separate
test stops receiving output and verifies explicit kill/reap completes without
draining the queue. Both checks pass on Linux x86-64 alongside all 12 existing
process-session tests. They do not execute a compiler, measure aggregate RSS,
prove descendant containment, or provide Windows/macOS execution evidence.

This experiment does not replace the Component Model comparison, ten-edit
latency measurements, sealed extension2 archive proof, or six native target
acceptance required by #13.
