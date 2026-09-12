# Issue 17 implementation status

The blob implementation on this development branch is incomplete and is not
release evidence for issue #17. The generated ABI exposes blob creation,
bounded writes, pull reads, explicit EOF, and closure through the existing operation family, exercised
by the threaded smoke guest. The write import validates the guest-memory range
and checks authority/chunk/pending-byte quotas before copying into host-owned
bytes using atomic byte loads. It retains no guest pointer across suspension.
Read collection validates a caller-supplied destination on each call and
copies only a completed bounded result. Pending calls retain no pointer;
wrong-owner, wrong-kind, short-buffer, and double collection are rejected.
The real guest verifies 64 MiB through 1,024 sequential write/read pairs,
reusing two 64 KiB arrays and checking every returned byte. Generated
`OutputFile::write_blob` dispatches an opaque blob/output token pair through
the same operation protocol to the supplied runtime's blocking lane. Its
wire-level host test verifies exact replacement and rejection of forged or
wrong-owner output grants. The in-process `execute_threaded_root_with_output`
entry point authorizes the destination before root Store creation. The root
guest discovers it through `OutputFile::granted`; child Stores do not receive
an initial grant. The real guest replaces an existing output with the exact
bytes `guest exact output`, with no sibling temporary remaining afterward.
This is Linux in-process output evidence only: output-grant delivery through
the killable worker protocol and its failure cleanup are not implemented.

Guest `seal` publishes EOF without revoking the readable handle. Pending empty
reads then complete with zero bytes; an empty unsealed blob stays pending.
Producers must await preceding writes before sealing, because sealing rejects
pending and subsequent writes.

## Verified host behavior

- Bounded chunk writes and pull reads use the existing scoped resource table.
- Pending writes wait for capacity; reads and resource closure release it.
- Pending reads distinguish data availability, explicit EOF, and cancellation.
- Cancellation discards pending write input. Sketch teardown also discards
  completed, unconsumed read buffers and rejects subsequent result retrieval.
- Exact output grants retain a canonical parent and final filename on the host.
  The current synchronous helper requires a sealed blob, writes a sibling
  temporary file, syncs and closes it, and uses the existing atomic replacement.
- Replacement failure removes the owned temporary file. Failed exclusive
  creation does not remove a file belonging to another creator.

Focused command:

```sh
soldr cargo test --locked --features wasm-sketch-host operations::tests --lib
```

The native hub test and the Cargo-built guest both transfer 64 MiB. Run the
guest proof with `CARGO_TARGET_DIR="$PWD/target/threaded-proof" bash
scripts/build-threaded-smoke.sh` on Unix (PowerShell counterpart on Windows).
On Linux x86-64 on 2026-09-12, the in-process proof passed in 5.52 seconds and
the worker proof in 9.33 seconds, including compilation/admission overhead
inside each test. The transfer uses an explicit finite fuel budget of
1.7 trillion aggregate and 100 billion for each root/child Store slice.
The guest now also fills the 1 MiB per-blob capacity, submits another 64 KiB
write, and proves that write remains pending across a scheduler turn with no
consumption. One bounded pull then completes the waiting write. It drains all
17 chunks before the 64 MiB round trip. The combined proof therefore asserts
a 1 MiB peak blob buffer; the earlier 64 KiB peak was for the sequential-only
proof. This is a controlled consumer pause in one Store, not a concurrent
multi-producer/consumer stress test. The guest additionally
cancels a capacity-blocked write and an empty pending read, observes typed
`Cancelled` results, and verifies that the cancelled write adds no chunk to
the subsequently drained stream. The generated cancellation import is accepted
only with the complete lifecycle import set. Pending write inputs and
retained read results have separate snapshot counters. This does not establish
a bound on total allocated memory, collection capacity, or simultaneous guest
transfers.

## Remaining acceptance work

The shared hub now enforces independent live-blob, pending-read, and
pending-write count limits (128 each by default). Hosts configure these and
chunk/per-blob/per-sketch byte limits through facade-owned `SketchBlobLimits`
and `SketchExecutionLimits::with_blob_limits`. Zero count limits disable the
corresponding admission. Byte limits must be nonzero and ordered
chunk <= blob <= sketch, with chunk lengths representable in the generated
32-bit transfer fields. Blob creation checks include reserved generated creations.
Pending-write rejection occurs before copying input; cancellation frees the
pending-I/O count even before the terminal result is collected. The separate
operation-table limit still accounts for that uncollected result. Focused
tests cover count reuse after cancellation/close and eight simultaneous host
submissions competing for one pending-write slot. This is host concurrency,
not yet the required concurrent guest-thread proof.
All 42 hub tests pass. With the count limits enabled, the existing 64 MiB
Cargo guest passed on Linux x86-64 in-process (5.87 seconds) and inside the
killable worker (9.18 seconds), reusing the admitted artifact and rebuilding
the host.

The worker supervisor sends all seven limits, and the worker validates them
through the same public constructor before compiler construction. Private
worker protocol version 3 rejects older peers rather than silently using
default limits; rebuild the worker executable together with the host. This
changes no guest ABI import signature or guest artifact. Pending inputs and
completed reads still have separate byte budgets. Hosts can additionally set
`SketchBlobLimits::with_maximum_transfer_bytes` for combined hub-owned blob,
pending-input, and completed-read backing capacities. Its default is three
times the sketch-storage budget plus one chunk; the minimum configurable
value is the sketch-storage budget plus two chunks. Writes preserve one chunk
of pull headroom, and reads can use that reserve. Result collection releases
capacity and revisits waiting writers. The bound includes write/read copy
overlap. Exact-sized read allocations avoid an eight-byte minimum growth for
smaller chunks. All 43 hub tests pass, including a tight 16-byte combined
budget that rejects input before copying, remains drainable, and resumes the
writer after result collection.

Native pulls now return a private read-only chunk whose backing capacity stays
charged until drop, including chunks held by the output writer during blocking
I/O. Drop frees the buffer before releasing its credits and driving waiting
operations. Teardown revokes resources but does not erase a still-live native
chunk's charge. Plain-`Vec` pull/collection helpers exist only in unit-test
builds; the production generated collector copies synchronously into guest
memory and native output uses the charged chunk. Allocator-internal
reallocation scratch is still outside this capacity ledger, which is not a
total process-memory limit. Root finalization now revokes authority, drains
guest children, and joins retained output jobs before recording final cleanup.
Output jobs have a separate lifetime count and are capped by the operation
limit even after guest cancellation/result consumption. Job completion releases
that count, not cancellation. A join error has the facade-owned
`output-cleanup-failed` result when no earlier execution failure takes priority.
Native blocking syscalls cannot be interrupted in-process: joining a wedged
filesystem operation can wait indefinitely. The required worker-owned output
grant and parent cleanup protocol remain necessary for bounded containment.
Validation now includes 46 hub tests and 25 root-observation tests. Controlled
blocking work proves that joining remains pending while a native chunk is
held, that the job cap rejects additional output admission, and that job and
memory counters reach zero after release. An injected unwind remains visible
to cleanup even after completed task handles are pruned. The reused Cargo
guest passed in-process (6.19 seconds, including exact output) and in the worker
(10.44 seconds including test-side compilation/admission) on Linux x86-64.
Both 64 MiB Cargo guest paths also pass with an explicit 2,228,224-byte
combined hub limit; the in-process proof retains its exact 1,179,648-byte peak
assertion. The version-3 protocol tests, reconstruction test, and host-policy
validation test pass with the additional transfer-budget field.
The non-default policy (64 KiB chunks, 1 MiB per blob, 2 MiB sketch storage,
one live blob/read/write) passed the existing 64 MiB Cargo guest in-process
(6.98 seconds) and in the rebuilt worker (11.07 seconds including test-side
compilation/admission), on Linux x86-64. Protocol round-trip/version rejection
and worker reconstruction of non-default/invalid limits also passed.

The capacity audit now measures `retained_transfer_capacity` separately from
payload lengths. A regression demonstrates why aggregate enforcement is still
required: after writing 1,024 bytes and reading 512 without collection, the hub
retains at least 1,536 bytes of backing capacity (1,024 in the partially drained
blob plus 512 in the result), although payload-length counters total 1,024.
Blob storage growth now checks the sum of retained blob backing capacities
against the sketch byte limit and reserves exactly the requested growth instead
of using geometric growth. Partial drains do not release that capacity budget;
emptying or closing the blob does. A focused regression verifies that another
blob's synchronous write is rejected and its asynchronous write stays pending
until the first blob releases its allocation. All 38 native hub tests pass.
This is a bound on blob storage, not yet one aggregate transfer-memory budget:
pending inputs and completed read results remain separately accounted.
The existing Cargo-built 64 MiB guest also passed with this accounting change
on Linux x86-64: 5.95 seconds in-process and 9.47 seconds through the killable
worker. These runs reused the admitted guest artifact and rebuilt the host.

`peak_retained_transfer_capacity` now retains a high-water mark across teardown.
It samples input admission and storage growth before pending inputs are dropped,
and includes the overlap while a pull allocates its result before releasing
an empty blob's backing buffer. A regression observes a 2,048-byte peak for a
1,024-byte asynchronous write/read even though snapshots after each operation
retain only 1,024 bytes. All 39 focused hub tests pass.
The in-process Cargo guest proof additionally asserts an exact 1,179,648-byte
peak (1 MiB blob storage plus two 64 KiB chunks) during its pressure phase and
zero retained capacity after teardown. That proof passed in 6.03 seconds on
Linux x86-64 with the existing admitted artifact and rebuilt host.

These counters now include native chunk capacity until drop, exposed separately
as `native_transfer_capacity`. All 44 hub tests pass, including retained-native
chunk pressure, drop-triggered writer progress, and truthful accounting across
teardown. Allocator-internal reallocation scratch is excluded; the high-water
mark is not a process-memory peak.
The existing Cargo guest remains green with native chunk charging: 6.24 seconds
in-process (including exact-output commit) and 9.61 seconds through the worker
on Linux x86-64. The admitted artifact was reused and the host rebuilt.

- Extend the sole generated ABI with bounded guest-memory transfers and
  semantic blob/output APIs; retain no guest pointer across suspension.
- Configure and account total bytes, live blobs, pending reads/writes, and all
  in-flight allocations, including retained capacity and completed results.
- Complete ordering and progress-timeout behavior under multiple producers and
  consumers; exercise scheduler races through real guest threads.
- Complete output commit cancellation coverage. Dispatch now uses the caller's
  blocking lane, reserves exclusive blob consumption, and checks the operation
  terminal winner at final replacement under the hub lock. A stalled filesystem
  replacement holds the hub lock; worker containment must bound this case.
- Deliver the exact output grant through the worker boundary before guest
  execution, retaining only the semantic handle in guest-visible state.
- Extend the 64 MiB generated guest proof with slow-consumer backpressure,
  cancellation, forged/stale/cross-sketch handles, and teardown.
- Inject write, flush, and rename failures and cancellation at the commit
  boundary; prove preservation of existing output and temporary cleanup.
- Verify the six supported targets and native output behavior on each OS;
  document measured quotas, transfer bytes, peaks, and cleanup counters.
- Review, validate, and merge the completed PR before closing #17.

The wider goal also retains #13, #19–#22, #28, and fp-bindgen#1. Completion of
these native hub tests does not close those deliverables.
