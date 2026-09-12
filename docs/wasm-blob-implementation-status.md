# Issue 17 implementation status

The blob implementation on this development branch is incomplete and is not
release evidence for issue #17. The generated ABI still exposes only the
synthetic operation family. Blob tests currently call the private operation
hub directly.

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

The 64 MiB test proves repeated native hub transfers, not Wasm transfers. Its
64 KiB peak is the blob buffer only; pending write inputs and retained read
results have separate snapshot counters. It does not establish a bound on
total allocated memory, collection capacity, or simultaneous guest transfers.

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
- Grant output before guest execution and pass only its semantic handle.
- Exercise 64 MiB through the real generated guest ABI, including slow-consumer
  backpressure, cancellation, forged/stale/cross-sketch handles, and teardown.
- Inject write, flush, and rename failures and cancellation at the commit
  boundary; prove preservation of existing output and temporary cleanup.
- Verify the six supported targets and native output behavior on each OS;
  document measured quotas, transfer bytes, peaks, and cleanup counters.
- Review, validate, and merge the completed PR before closing #17.

The wider goal also retains #13, #19–#22, #28, and fp-bindgen#1. Completion of
these native hub tests does not close those deliverables.
