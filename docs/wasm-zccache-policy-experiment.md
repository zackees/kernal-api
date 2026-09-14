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

The revision-8 archive guest was freshly rebuilt and passes its 29 authenticated
staging tests in 1.46 seconds plus all eight streaming cases in 42.94 seconds
on Linux x86-64. The screenshot guest was also rebuilt for revision 8 in
normal, trap, and blocked forms; all 16 Linux x86-64 native regressions pass in
173.78 seconds. These are local compatibility results, not six-target evidence
for this revision.

Fresh local revalidation at `3dbafa2` ran the checked-in
`ci/run_extension2_guest.py` harness against a newly compiled archive guest and
newly embedded ABI metadata. Its native host staging group passed 29 tests (4
artifact-dependent tests ignored) in 0.45 seconds; the selected actual guest
streaming/authentication proof then passed once in 31.91 seconds. This is a
current Linux x86-64 execution of the one exact ignored proof, not a rerun of
the prior eight-case group.

Hosted CI run `34783064265` at `983d3ee` then ran that checked-in staging plus
actual-streaming harness successfully on all six native targets: Linux,
macOS, and Windows on x86-64 and ARM64. This is six-target execution evidence
for this exact sealed archive guest proof; it does not turn the earlier local
timing into a cross-host performance comparison or satisfy the separate
Component/facade selection gates below.

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
prove native Windows path equivalence. The later revision-8 compiler grant adds
the actual bounded request-key decision and controlled process miss described
below; it still is not a complete artifact cache. The source pin is
migration-only, not a published front-door dependency.

The remaining work must compare native and Wasm policy outputs on every host
and keep daemon, watcher, IPC, and artifact movement native.

### Controlled compiler-miss design constraints

Source inspection identifies `SpawnSpec::spawn_session` and `ProcessSession`
as the existing native facade seam. `ProcessSessionOptions` bounds queued
chunks and each chunk's bytes; `next_output` preserves stdout/stderr identity
and backpressures child pipes. Lifecycle wait/kill remain independent of a
pending output receive. Reuse this seam rather than adding another process
backend or executor. The revision-8 integration now exercises this exact seam;
the remaining artifact-cache and six-host work is called out below.

The host grants one exact command description, working directory, explicit
environment/stream policy, and one opaque cache identity. The guest derives
the real `zccache-hash` request-fingerprint bytes through the public bounded
`Blake3Hasher`, then supplies its fixed 32-byte digest to the grant's semantic
`cache_status` method. The current private fixture supplies a precomputed
expected digest and outcome; a production host would derive that identity from
its metadata/content facts before instantiation. It exposes neither paths,
artifact bytes, nor cache enumeration. A mismatch or foreign store is rejected
without spawning or consuming the grant.

On `Hit`, the shared policy returns before submitting a process operation. The
actual Component hit test supplies an intentionally nonexistent absolute
compiler path: success therefore proves that no compiler was launched. On
`Miss`, it uses the existing exact host command and bounded tagged output
facade, drains and hashes 2 MiB from each stream, observes exit, and awaits
close. The same cache-aware shared policy passes freshly rebuilt revision-8
Core and Component guest artifacts on Linux x86-64. Unit coverage also checks
exact key matching, store ownership, miss/hit values, and zero spawn attempts.

The Core private threaded-root fixture now uses the real
`zccache-artifact::KvStore`, pinned at
`e473e4cd8945f8e7e3bf3d93b2bb3c5b469e72ae`, in the private
`kernal-compiler-v1` namespace. On a miss, after the guest has completed its
normal process/output lifecycle and the exact output job has drained, the
host reads that embedding-selected output and atomically retains it under the
same 32-byte request key. On a hit, the host reads and verifies the zccache
value, restores it through the existing exact-output atomic replacement path
before instantiating the Store, and withholds output authority from the guest.
The guest sees only `Hit` and returns before spawn; cache root, namespace, and
artifact bytes remain private host state.

The ignored actual Core round-trip test first runs the controlled miss, then
runs the same request with a deliberately nonexistent compiler executable.
It verifies the second embedding-selected output equals the cached first
artifact, so success demonstrates both retention/retrieval and no compiler
spawn. The focused KV test independently verifies persistence through the
real zccache format. This remains a bounded private experiment, not a public
cache API, a cache enumeration capability, or an artifact-cache selection
decision. Reserve operation/resource capacity before the controlled miss
spawn; revocation between reservation and attachment must reclaim a spawned
session. Output delivery retains its aggregate in-flight byte budget in
addition to native queue limits.

The `wasm-compiler-native` CI matrix runs that exact compiler-cache proof on
each required native host without first fetching the private extension2 policy
source. `ci/run_compiler_guest.py` builds the separate compiler-only guest,
embeds fresh ABI metadata, builds one native host harness, and runs the
miss-to-hit test.
The extension2 archive matrix remains a separate credential-dependent proof.
Merely scheduling this matrix is not platform evidence; each native result
must succeed before it is counted toward #13.

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

### Internal compiler-grant lifecycle foundation

`src/wasm/process_resource.rs` now adds the native grant/spawn lifecycle to
the existing operation hub, behind `wasm-sketch-host`. Grants bind an absolute
executable and cwd, explicit environment, null stdin, and piped output. They
require spawner ownership: an omitted binding is normalized to the spawner;
a conflicting explicit owner is rejected. Native owner-death enforcement
retains its platform limitations (including Linux's documented SIGTERM),
and is not a portable descendant-tree guarantee.

Operation, resource, and tracked-job capacity are reserved before consuming
the one-shot command. At most four native jobs are retained concurrently;
each session uses a one-chunk queue with 64-KiB chunks. Resource revocation
signals a tracked supervisor, which explicitly kills and waits for the
direct child outside the authority mutex. Root finalization joins these jobs.
Join futures retain task handles in the hub, serialize their polling, and
latch cleanup errors rather than treating a failed join as successful reaping.
The deadline begins at admission and expired queued commands do not spawn.

Linux x86-64 validation passes all 93 operation tests, including 14 process
tests/helper cases. Two focused regressions were observed RED before their
fixes: simultaneous cleanup joiners lost a wakeup, and an already-expired
queued command still attempted native spawn. The tests also cover cancelled
join retry, cancellation after spawn but before publication, uncollected
spawn abandonment after actual output readiness, deadline reaping without
draining output, grant/operation/resource/job admission, and owner binding.

This foundation is deliberately not guest-reachable yet: no process ABI,
root grant provisioning, public guest process API, or Component adapter is
added by it. Complete output accounting and ABI integration
remain required before exposure. These tests use controlled child fixtures,
not rustc or the cache workflow. They do not prove abrupt worker-death cleanup,
uninterruptible spawn containment, aggregate RSS, or six-host execution.

### Scoped compiler-output collection

The internal `process_output.rs` path reserves one operation and 64 KiB of
the existing shared transfer budget before receiving a native output event.
Only one pending or uncollected event is allowed per process. Its payload is
dropped before its reservation is released, even if the process is revoked
while the event is held. Successful collection validates authority and runs
the bounded delivery callback under the same lock. Dropping a pending or
uncollected event revokes the process rather than permitting continuation
from an uncertain byte position. Callback unwind releases the lock before
revocation, without poisoning the authority. Receive and collection preserve
distinct cancellation, deadline, trap, owner-exit, and closure terminal reasons.

Stdout/stderr tags and native completion events are retained. A fixed 64-MiB
combined output ceiling rejects the overflow event before exposing it to a
consumer. The transfer-capacity snapshot now distinguishes the reserved
process-event allowance from native blob-buffer capacity; it is charged
capacity, not an RSS observation. Linux validation passes 101 operation tests,
including 22 process tests/helper cases and a 4-MiB dual-stream fixture. A
cancellation-after-receive regression was RED before collection and delivery
were made atomic. Pending-read Drop, uncollected-result Drop, callback unwind,
owner/budget rejection, and cumulative counting are covered.

This initial event reservation did **not** account for all native process buffers.
Source inspection of `running-process` 4.10.10 `process_runtime.rs` finds one
shared queue, two pump scratch buffers, and two blocked-send payloads. Tokio
1.53.1's Windows process pipes additionally use two blocking-read buffers.
Thus a conservative payload allowance is eight 64-KiB chunks including one
facade event, or nine with a separate host copy; allocator overhead, task
metadata, error strings, kernel pipes, child memory, and guest memory are
separate. These are source-derived allowances, not measured peak allocation.

`ProcessSession::wait` proves direct-child reaping, not buffer release. Normal
channel exhaustion is stronger for pump/queue storage, but Windows blocking
reads can outlive post-exit abandonment. The substrate needs an explicit,
tracked output-shutdown acknowledgement (including outstanding platform I/O)
before native reservations can be safely recycled. That initial internal
output step did not supply the acknowledgement or native ledger. The following
prerequisites add them; guest ABI and cache hit/miss integration remain absent.

This experiment does not replace the Component Model comparison, ten-edit
latency measurements, sealed extension2 archive proof, or six native target
acceptance required by #13.

## Native output-shutdown facade prerequisite

The experimental branch now pins the private substrate to
`1943831d13bf0f4c82eb694bc1f9cf5940d4718a` from
[running-process PR #1204](https://github.com/zackees/running-process/pull/1204).
Its existing package version is 4.10.11; no package was published or relabelled.
This Git source is migration-only. The unchanged TOML-aware release guard
rejects it, and an exact published registry release containing the implementation
must replace it before release packaging. This is not release-ready evidence.

`ProcessSession::shutdown_output(&self)` requests shutdown before acquiring the
private output-lane mutex, then awaits the substrate's reader/pump cleanup and
discards queued events. Returned events remain caller-owned. It neither kills
the child nor proves descendant cleanup. Concurrent callers and cancelled
observers can retry the same shutdown; an error is not permission to recycle
native capacity.

The facade regression deliberately parks a receive holding that mutex. Moving
the shutdown request behind mutex acquisition produces a five-second deadline
failure (Soldr log `20260913T124130Z`); requesting first passes. A separate case
drops a pending shutdown observer and retries with two callers. These are native
facade tests, not proof of guest exposure, native-buffer admission accounting,
or the complete compiler/cache workflow.

## Native compiler payload admission and cleanup

The supervisor now reserves seven 64-KiB chunks (448 KiB) under the hub lock
before consuming a compiler grant or scheduling native creation. This covers
the pinned substrate's queue, scratch, pending-send and Windows blocking-read
payloads. The existing facade-event lease adds a separate 64 KiB when a receive
is admitted. Neither figure measures allocator overhead, metadata, diagnostic
strings, OS pipe buffers, child memory, guest memory, or additional host copies.
The native allowance participates in transfer-capacity admission and snapshots.

The tracked supervisor owns the reservation; resource revocation, operation
collection, and direct-child exit cannot refund it. It concurrently attempts
acknowledged output shutdown and direct kill/reap, including wait after a kill
error. Refund requires successful cleanup. Pre-spawn cancellation/deadline
returns the unused reservation. A failed spawn, cleanup error, panic, or dropped
supervisor does not refund uncertain native storage; failed jobs poison further
compiler admission. This deliberately favors retained accounting over pretending
that an unknown failure reclaimed native resources. Scope destruction is not a
claim that surviving native I/O was reclaimed.

Linux operation tests cover rejection without consuming/scheduling, charging
before first supervisor poll, a caller-held event outliving native cleanup,
and pre-spawn refunds. A private per-hub checkpoint pauses before output cleanup:
even after direct reaping and spawn-result collection, the allowance cannot be
reused until acknowledgement. Injected errors and panics occur after real output
cleanup, testing conservative accounting without leaking actual child pipes.
The checkpoint releases on sender Drop during assertion unwind. An unconditional
refund mutation fails the error-retention test (Soldr log `20260913T124854Z`).
These parent tests prove ledger ordering; actual platform blocking-read cleanup
requires the upstream native tests. Six-host proof and guest compiler/cache
integration remain required and are not inferred from these Linux results.

Local validation: 104 operation tests pass, strict Clippy passes with
`--no-default-features --features wasm-sketch-host --lib --tests`, and the
actual ABI-revision-6 Core-Wasm parser/hash guest still passes its 64-MiB public
facade proof against the updated host. That reused guest artifact tests ABI
compatibility; no compiler guest operation was added or exercised by it.

## Per-process close completion

The internal compiler authority now returns an observation-only completion
when closing a process. Authority is revoked synchronously; the completion
resolves only after the tracked supervisor finishes native cleanup and its
payload refund. Closing one process does not close the logical hub. Concurrent,
cancelled/retried, and late observers see the same retained result through the
existing facade sticky notification primitive, without sharing a single task
join waker or allocating a tombstone map.

The supervisor owns a completion guard before its first poll. Panic or runtime
teardown reports failure rather than leaving observers pending or treating task
destruction as resource reclamation. Moving guard construction inside the future
is RED (`20260913T130548Z`): destroying an unpolled runtime leaves the observer
pending. Restoring external ownership is GREEN. Tests also hold cleanup after
reaping, cancel an observer, wake concurrent and late observers, reject a foreign
owner, and preserve the failed result and native charge after cleanup error/panic.
These are private host prerequisites; the guest compiler operations and their
Core/Component adapters have not yet been added.

Upstream shutdown validation now passes all six native OS/architecture pairs in
[run 34758566275](https://github.com/zackees/running-process/actions/runs/34758566275),
including the queued-before-system-call race on Windows x86-64 and ARM64.
The CI commit `8e8c942` changes no Rust implementation relative to the parent pin
`1943831`. This proves the dependency shutdown primitive on those hosts, not the
parent ledger, complete guest workflows, or full #13 six-host acceptance.

The private exit observer validates the process capability before waiting and
again before returning a status. It neither consumes output nor revokes the
process when its future is dropped; process revocation wakes a pending observer.
The test checks a registered counting waker immediately after revocation, before
any repoll or runtime scheduling, rather than relying only on sticky state.
The dual-stream test drains output before waiting and checks repeatable exit
observation. Removing admission-time ownership validation is RED (Soldr log
`20260913T131658Z`): a foreign owner waits on the native process instead of being
rejected immediately. This remains a host primitive, not a guest ABI claim.

## Revision-8 Core compiler guest proof

The Core candidate now exposes `guest::CompilerGrant`, `CompilerProcess`, tagged
`CompilerOutputEvent`, and semantic `CompilerExit`. The host owns the exact
executable, arguments, cwd, cleared environment, and deadline. Grant retrieval
is one-shot; neither a path nor a guest-authored command crosses the ABI. The
host injection is currently the private root-grant fixture, not a general
external host API. The operation protocol is revision 8 (35–48), so revision-7
modules must be rebuilt, not relabeled.

A completed output operation owns the native event directly, without retaining
an `Arc<OperationHub>` inside the operation table or allocating another payload
copy. Publication atomically transfers its 64-KiB credit from the native read
lease to the ready result; even EOF/exhaustion retain that full credit until
collection or abandonment. Collection validates the owner and a complete
64-KiB destination before consuming anything, then copies directly into shared
atomic memory. Generic lifecycle polling cannot consume an output/exit result.
Cancellation and resource revocation still invalidate ready, uncollected output
and preserve their terminal reason. Abandoning a read revokes an uncertain
stream; abandoning an exit observer does not revoke the process. `close().await`
waits for the existing native cleanup acknowledgement, not merely child exit.
All producer handles are bounded independently of consumed operation slots and
joined during root cleanup on the caller-supplied runtime.

The fresh Cargo-built `kernal-compiler-guest-proof` executes the shared
`benchmarks/wasm-sketch/shared/compiler_policy.rs` through these public methods.
Before obtaining the one-shot grant, that policy runs the shared explicit-host
Rustc parser fixture; the same actual guest therefore covers parser policy,
request-key derivation, cache decision, and controlled miss in one path. It
then checks one-shot grant retrieval and rejection of an undersized read, drains
2 MiB from each native stream in at most 64-KiB chunks, hashes fixture marker
runs without a second guest payload buffer, observes both EOFs and exhaustion,
checks repeatable successful exit, and awaits close. On a cache miss, the
Core-only fixture seals its verified marker runs into a bounded 4-MiB `Blob`
and publishes that blob through the one exact output destination granted by the
embedding host. The host asserts that the published file contains exactly 2 MiB
of each marker; text emitted by the native self-executing test harness never
enters either hash or artifact. A cache hit receives no output grant and must
not publish a file. The Component candidate deliberately has no corresponding
WIT capability. The Linux x86-64 host observes zero live resources, pending
operations, retained process jobs, and transfer bytes afterward. This is
bounded exact-output transfer evidence, not an artifact-cache workflow or a
measurement of total RSS.

Focused regressions cover invalid/foreign collection followed by successful
retry, actual out-of-range shared-memory destinations, ready-result revocation
for all terminal reasons, pending/ready teardown, exhausted-result abandonment,
retained producer quota after observer cancellation, and delayed close
acknowledgement. Review found an interaction where producer Drop overwrote a
successful exit with `Closed`; the focused wire-wait test reproduced it
(`20260913T134311Z`) and passes with an atomic pending-only Drop transition.

Linux build/execute commands (native builds additionally need the system's
OpenSSL/pkg-config development environment):

```sh
SOLDR_LINKER=default soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/extension2-guest/Cargo.toml \
  --features guest-proof --bin kernal-compiler-guest-proof \
  --target wasm32-wasip1-threads --release --target-dir target/extension2-stream -j1
soldr --no-cache cargo run --locked \
  --manifest-path tools/wasm-abi-generator/Cargo.toml \
  --target-dir target/extension2-abi -j1
cp target/extension2-stream/wasm32-wasip1-threads/release/kernal-compiler-guest-proof.wasm \
  target/extension2-stream/compiler.admitted.wasm
target/extension2-abi/debug/kernal-api-wasm-abi-generator \
  --embed-threaded-metadata target/extension2-stream/compiler.admitted.wasm
KERNAL_COMPILER_GUEST_WASM="$PWD/target/extension2-stream/compiler.admitted.wasm" \
  soldr --no-cache cargo test --locked --no-default-features \
  --features wasm-sketch-host --lib -j1 \
  wasm::compiler_dispatch::tests::compiler_actual_guest_spawns_drains_hashes_persists_waits_and_closes \
  -- --exact --ignored --nocapture
```

The fixture lock now follows the same explicit migration-only process-substrate
revision as the parent; it is not a published dependency acceptance claim.
The Component compiler adaptation and one exact zccache request-key hit/miss
decision now exist. The Core fixture also transfers one verified compiler
artifact through the existing exact-output authority; it does not yet retain,
address, or retrieve that artifact from a zccache backend. Matched candidate
measurements and the remaining #13 acceptance evidence remain unfinished. The
Core-Wasm/Wasmtime threaded substrate is the recorded v1 decision; this does
not turn the Component candidate into a runtime fallback or close #13 before
the outstanding evidence exists.

The revision-8 archive and screenshot artifacts now have fresh six-native-host
evidence from [Actions run 34790621949](https://github.com/zackees/kernal-api/actions/runs/34790621949): Linux, macOS, and Windows on x86-64 and ARM64 all completed
the actual streaming-archive and offline screenshot/admission lanes. The same
run's native Windows test job failed only in the independently tracked
compiler-session job-object restriction
([running-process#1207](https://github.com/zackees/running-process/issues/1207));
it did not fail an archive, screenshot, or containment proof.
This is six-host evidence for those artifacts, not a claim that the separate
compiler-policy guest execution, artifact-cache workflow, or full #13
acceptance is complete.

## Component compiler candidate

The opt-in `wasm-component-compiler-experiment` now implements the same public
`CompilerGrant` / `CompilerProcess` guest API and runs the identical shared
compiler policy. Host spawn, output, hashing, exit observation, cancellation,
and close use the existing operation hub and process supervisor. The WIT
interface passes typed capabilities and bounded output, never command strings.
This is compile-time candidate selection, not a runtime fallback. Selecting the
candidate alone does not enable the native Wasmtime dependency; the host proof
also selects `wasm-sketch-host`.

Component output needs an extra copy that the Core adapter does not make.
Before submitting a native read, the adapter admits a typed-resource slot and
an additional 64-KiB lowering allowance from the same aggregate byte budget.
The ready output resource owns the native operation. Its synchronous `copy`
method consumes that event once into a bounded host `Vec`; repeated copies
reject without allocation. The resource keeps its extra allowance until its
destructor, after canonical lowering returns. The guest briefly holds the
canonical list and its public API destination, unlike Core's direct copy.
Uncopied resource destruction abandons the read and revokes an uncertain stream.
Pending async returns consume resource slots too.

Teardown first closes adapter admission, then closes the hub, destroys the whole
Wasmtime Store, joins native producers, and only then refunds deferred lowering
credits. Resource-field destruction alone does not prove canonical lowering has
released its return value. The budget refuses finalization with live resources.
Focused tests cover one-shot copy, slot reuse, and this deferred refund. An
actual encoded guest with its `cabi_realloc` body replaced by `unreachable`
traps after the first host output copy; the test checks that the lowering credit
remains charged after Store destruction until explicit finalization. Fault
encoding preserves all other core-module bytes and is opt-in only.

A second actual-guest test cancels the outer execution while a real async host
read holds an admitted operation and lowering permit but has not published its
output resource. The test verifies zero output copies and full cleanup after
Store destruction; it does not claim guest-issued cancellation/reuse coverage.
Disabling the budget's deferred-refund branch is RED (`20260913T142420Z`):
resource destruction prematurely reduces the expected 64-KiB charge to zero.
Restoring the branch makes the focused test GREEN.

Linux x86-64 execution passes the cache hit, controlled 2-MiB-per-stream miss,
allocator-trap, and admitted-read cancellation cases. The success path observes
zero resources, operations, tracked process jobs, and transfer bytes after
cleanup. To reproduce, use a fresh output directory because the encoder
intentionally refuses to overwrite files:

```sh
SOLDR_LINKER=default soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/component-guest/Cargo.toml \
  --features compiler-proof --target wasm32-unknown-unknown --release \
  --target-dir benchmarks/wasm-sketch/component-guest/target -j1
SOLDR_LINKER=default soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/component-guest/Cargo.toml \
  --features lowering-trap-proof --target wasm32-unknown-unknown --release \
  --target-dir benchmarks/wasm-sketch/component-guest/target-lowering -j1
soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/component-tools/Cargo.toml -j1
component_proof_dir=$(mktemp -d /tmp/kernal-component-compiler-XXXXXX)
benchmarks/wasm-sketch/component-tools/target/debug/kernal-component-tools \
  benchmarks/wasm-sketch/component-guest/target/wasm32-unknown-unknown/release/kernal_component_probe.wasm \
  "$component_proof_dir/compiler.wasm"
benchmarks/wasm-sketch/component-tools/target/debug/kernal-component-tools \
  benchmarks/wasm-sketch/component-guest/target-lowering/wasm32-unknown-unknown/release/kernal_component_probe.wasm \
  "$component_proof_dir/compiler-trap.wasm" --trap-realloc
KERNAL_COMPONENT_COMPILER_WASM="$component_proof_dir/compiler.wasm" \
KERNAL_COMPONENT_COMPILER_TRAP_WASM="$component_proof_dir/compiler-trap.wasm" \
  soldr --no-cache cargo test --locked --no-default-features \
  --features wasm-sketch-host,wasm-component-compiler-experiment --lib -j1 \
  wasm::component_compiler::tests -- --include-ignored --nocapture
```

The candidate remains incomplete for #13: hostile incoming hash lists are still
canonically allocated before the host length check; the fixed 32-byte compiler
cache key crosses the Component ABI as four scalar words. Cancellation coverage
and public blob parity are not complete. The request-key decision is
not a complete artifact cache: it does not persist or move artifacts. Nor is it
six-host parent acceptance, a total-RSS bound, or the matched measurements needed to
choose the final runtime. Fixture source pins remain migration-only, not
published-dependency acceptance.

Local regression gates also pass: 116 operation tests, four native semantic
guest-adapter tests, the revision-8 Core compiler and 64-MiB hash artifacts,
and the legacy Component execution probe (hash, bounded transfer, traps,
pending-call teardown, cancellation/reuse, and backpressure). Strict Clippy
passes for the native Component host with tests, the default native library,
the Component compiler guest, and the encoder's default targets. The default
native graph and native candidate-only graph contain neither Wasmtime nor
wit-bindgen. The sole local pre-push review found no remaining findings.
