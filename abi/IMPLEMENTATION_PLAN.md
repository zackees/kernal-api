# Issue 16 implementation sequence

Work is performed sequentially in the existing checkout, without worktrees.
Each implementation is assigned to Terra after two main-agent investigation
rounds. Tests follow implementation; failures are fixed before handoff. The
main agent and Sol review the completed change before its PR is pushed and
admin-merged on local evidence. GitHub Actions is not a merge gate for this run.

## 36: Generated versioned contract (active)

Investigation round 1 inspected issues 13, 15, 16, 36, 37, and 38, the closed
synthetic and threaded admission profiles, thread Store ownership, worker
containment, and feature-isolation rules at baseline `2aacba6`. The existing
`kernel-yield` import is handwritten; no generated operation protocol exists.

Investigation round 2 inspected the owned fp-bindgen generator and macro tests
at `4e44d9e5408653e3c428ee3f855cc194d53f60b0`, its emitted artifacts, existing
threaded fixture build scripts, and CI target coverage. Its explicit
Wasmtime/Core-Wasm backend generates scalar synchronous control calls. It
rejects async declarations and bulk values; asynchronous lifecycle semantics
must therefore use generated scalar submit/poll/yield/cancel controls.

Implementation scope: one Rust declaration source, isolated pinned generator,
checked-in private guest support and host adapter, exact generated admission
descriptors and metadata, stable bounded control/result/error reservations,
deterministic regeneration/drift checks, real guest build and import mutation
fixtures, compatibility and historical RED evidence. No asynchronous execution
or resource registry implementation belongs to this change.

Review corrections identified during implementation:

- Completion values must carry full scope/slot/generation without lossy packing.
- Scope comes from host context; guest-supplied integers never grant authority.
- Terminal requests need an explicit acknowledgement/release contract because
  completion words may be read separately.
- Admission signatures must derive from generated artifacts, not duplicate
  handwritten tables. The generated linker must participate in real linking.
- Metadata stamping belongs to explicit artifact tooling and must derive from
  the same contract.
- The pinned generator omits parent helper imports in guest output. Correct
  its vendored source with provenance and regenerate, rather than editing
  generated output or introducing another backend.

Local validation must cover deterministic regeneration, guest compilation,
exact admission and metadata mutation rejection before compilation/effects,
generated linkage, ordinary feature isolation, and relevant existing tests.
Report platform compilation separately from native execution.

## 37: Generated asynchronous lifecycle (queued)

Only start after 36 is merged. Reinvestigate the final generated contract and
the current threaded runtime in two rounds before writing Terra's concrete
prompt. Implement one synthetic yield operation, host-owned request identity,
bounded pending quota, explicit caller RuntimeHandle, suspension/resumption,
one terminal winner, cancellation/timeout/exit/trap/teardown cleanup, and
instrumentation. No resource registry or alternate scheduler/ABI.

## 38: Scoped generation-safe resources (queued)

Only start after 37 is merged. Reinvestigate the landed lifecycle and actual
guest-thread sharing in two rounds before writing Terra's concrete prompt.
Implement generated synthetic create/use/close, logical-sketch ownership,
scope/slot/generation/kind/rights validation, quotas and idempotent cleanup.
Prove real same-sketch cross-thread sharing and foreign-sketch rejection.
No byte transfer, paths, files, streams, Tauri, or ambient host capabilities.

## Parent completion audit

After all three changes merge, check every acceptance criterion of issue 16
against current code, checked-in fixtures, actual local results, and merged
PRs. Missing platform or lifecycle evidence remains outstanding; child merges
alone do not establish parent completion.
