# Compatibility and version policy

`kernal-api` owns the systems dependency versions that must coexist in one
process. Its direct dependency requirements are exact and release tests keep
them aligned with the checked-in lockfile. Clients do not select parallel
implementations of these facilities.

| Contract | Supported baseline |
|---|---|
| Rust | exactly the project's 1.95.0 MSRV/toolchain floor or newer |
| Python | 3.10+ |
| Operating systems | Linux, macOS, Windows |
| Architectures | x86-64, ARM64 |
| Async engine | `kernal_api::async_engine`, backed by exactly Tokio 1.53.1 |
| Heap allocator/profiler | exactly `mimalloc-pprof` 0.9.4 |
| Native crash interception | exactly `crash-handler` 0.7.0 |
| Async task protocol | exactly `console-api` 0.9.0 / `console-subscriber` 0.5.0 |
| CPU/async export | one checked-in `perftools.profiles` schema |

## Wasm fuel and epoch characterization (#44)

The feature-gated `wasm-sketch-host` resolves **Wasmtime 45.0.0** from the
checked-in Cargo lockfile. The facade keeps its backend types private while
characterizing these semantic boundaries:

- a real Wasmtime `OutOfFuel` trap maps to `SketchExecutionError::OutOfFuel`
  for both root and ordered child execution, even if an epoch terminal winner
  was also observed;
- epoch deadline observation for ordinary compute is finite under a configured
  tick/deadline but wall-clock delivery is scheduling-dependent; same-tick
  cancellation deterministically wins deadline selection;
- root failure takes precedence over ordered child failure, followed by private
  validation reporting; cooperative terminal paths release epoch, Store,
  instance, and root accounting;
- an `atomic.wait` or host-blocked call is not an in-process cancellation
  promise. It is classified as `ContainmentRequired`, characterized only in a
  killable subprocess, and worker-process reaping remains issue #28.

The default feature set remains empty: Wasmtime and this characterization are
not selected by ordinary async/process users.

| Target | Native evidence for this boundary |
|---|---|
| Linux x86-64 | Managed Bosn 0.1.3 (`uname -m` = `x86_64`): current-tip fmt, warnings-denied all-feature lint, and locked all-feature suite passed (jobs `j1-2957e646`, `j2-20e8e2d9`, `j3-c11d96c9`); both exact containment subprocess tests also passed (`j6-c8de8973`, `j7-943bab55`). |
| Windows x86-64 | Native Soldr current-tip CI-equivalent all-feature suite passed: 369 library tests, all integration/doc tests, with only the two established PDB line-resolution quarantines skipped. |
| macOS x86-64 / ARM64 | pending/not run |
| Linux ARM64 | pending/not run |
| Windows ARM64 | pending/not run |

This is an evidence matrix, not a statement of test execution. Supported
targets remain those in the package policy above; unlisted native runs must be
recorded before being claimed as evidence.

## Wasm worker containment (#28)

`wasm-sketch-worker` places one request in a private contained process
boundary. The facade reports typed worker diagnostics and maintains its parent
lease/gauge accounting through normal completion, cooperative cancellation,
deadline handling, unexpected exit, and bounded forced cleanup; it does not
expose child handles, Job Objects, pidfds, or backend worker types. Windows
uses kill-on-close Job containment and Linux uses parent-death signaling, so a
supervisor's abrupt death also contains the worker. The optional
`wasm-sketch-worker-test-support` hook is test-only and not a public API: its
external proof marker records a PID together with an opaque native creation
key, never a PID alone. The broader universal identity API remains follow-up
#51.

`default = []` remains isolated from the Wasm host, worker, and test-support
feature. The process boundary is therefore opt-in rather than a new default
process/runtime dependency.

| Target | Focused #28 evidence |
|---|---|
| Windows x86-64 | Native local Soldr passthrough with `ZCCACHE_DISABLE=1`: the four core D4 containment tests plus crash and parent-death proofs passed (6 passed); the two exact, ignored inner helpers were intentionally not part of the normal run. Prior current-tip checks, lint, and regression evidence also passed. |
| Linux x86-64 | Core/full evidence predates this focused record, but D4 crash and parent-death native proof is pending: the foreign managed Bosn lease is protected and was not disturbed. |
| Windows ARM64 | Compile-only; native focused evidence pending. |
| macOS x86-64 / ARM64 | Compile-only; native focused evidence pending. |
| Linux ARM64 | Pending. |

This matrix records platform evidence rather than expanding the supported API
surface. Native crash/parent-death claims require exact PID-plus-creation-key
observation and bounded disappearance/exit evidence on the listed target.

## Client rule

Until 1.0, the four first-party clients use an exact Cargo requirement:

```toml
kernal-api = { version = "=0.1.0", features = ["..."] }

[profile.dev.package.kernal-api]
codegen-units = 1

[profile.test.package.kernal-api]
codegen-units = 1
```

The Python companion is likewise pinned with `kernal-api==0.1.0` when used by
first-party Python tooling. A source checkout may temporarily use a path patch
only on an explicit migration branch; release branches must resolve the exact
registry version. There is no `optional = true` legacy implementation behind
the same client operation and no runtime fallback to a second HAL.

## Ownership boundary

- `platform`: process, filesystem, IPC, PTY, terminal, host identity, and
  resource operations.
- `snapshot`: cooperative sibling-thread capture and deferred unwind.
- `crash`: the single native crash handler and bounded pre-crash spool.
- `profile`: bounded sampling, CPU/off-CPU aggregation, and pprof/Firefox/
  collapsed export.
- `symbolize`: protobuf-tagged ASLR-independent capture schema and isolated
  PDB/DWARF/Mach-O parser worker.
- `allocator`: the facade-owned allocator plus sampled heap lifecycle/dumps.
- `async_engine`: the facade-owned runtime/task surface and task diagnostics.
  It also owns cancellation tokens, connection deadlines, and progress/idle
  timeout policy; clients must not substitute a raw runtime or global transfer
  timeout for these contracts.

Client CI installs the two Dylints in [DYLINT.md](DYLINT.md). They deny direct
implementation-crate use and host `cfg` selection outside this HAL, including
platform branches elided on the CI host.

First-party builds use Soldr's cache-enabled Cargo front door. That preserves
native `CC`/`CXX` caching and Soldr's exclusive resource gate for oversized C,
C++, and published-Rust amalgamations. This scheduling contract complements
the facade boundary: Soldr protects the compilation unit, while the Dylint
prevents a client from introducing a second owner for it. Client workspace
profiles keep the centralized facade at one rustc codegen unit because
dependency-local profile settings do not control a consuming workspace.

Applications retain product policy: which environment variable enables a
facility, where dumps are stored, and which endpoints are exposed.

## Dependency direction

In the target architecture, `running-process` is the lower native/process
implementation layer and `kernal-api` is its higher semantic facade. That
private dependency has not landed in the current release. The reverse
dependency is forbidden. First-party applications migrate to a single direct
systems dependency on `kernal-api` and may not expose backend types in their
public APIs.

Migration branches may temporarily carry both dependencies while a capability
is moved. The corresponding strict Dylint rule is enabled as soon as the facade
has parity, and release branches must not retain the legacy direct dependency.
Exact backend versions are selected by `kernal-api`, not by its clients.
