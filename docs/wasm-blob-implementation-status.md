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
multi-producer/consumer stress test. Pending write inputs and
retained read results have separate snapshot counters. This does not establish
a bound on total allocated memory, collection capacity, or simultaneous guest
transfers.

## Remaining acceptance work

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
