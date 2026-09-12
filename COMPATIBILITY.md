# Compatibility and version policy

`kernal-api` owns the systems dependency versions that must coexist in one
process. Its direct dependency requirements are exact and release tests keep
them aligned with the checked-in lockfile. Clients do not select parallel
implementations of these facilities.

## Generated Core-Wasm ABI v1

`abi/kernal-api-v1.rs` is the sole declaration source for the private
`kernal-api:v1` scalar Core-Wasm control ABI. `bash scripts/regenerate-wasm-abi.sh`
regenerates its checked-in guest crate, Wasmtime 45 linker glue, and manifest;
`bash scripts/regenerate-wasm-abi.sh --check` fails on drift. The generator is
vendored from fp-bindgen revision
`4e44d9e5408653e3c428ee3f855cc194d53f60b0` with a documented generator-source
helper-scope correction. It runs only when explicitly called and is outside
the facade's normal dependency graph. See `abi/README.md` and the vendor
provenance record for reproduction commands and the exact local delta.

Version 1 carries only bounded scalar controls. A host assigns opaque request
identities; completion, error/status, operation, scope, and resource identity
fields are specified beside the declarations. It conveys neither bulk values
nor authority. Lifecycle and resource execution remain reserved for #37 and
#38. The generated guest compiles for `wasm32-wasip1-threads`; CI runs that
compile on Linux and Windows. macOS compilation is covered by Soldr's
cross-target lane; macOS execution remains gated by `ENABLE_MACOS_RUNNERS`.

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
| Linux x86-64 | Managed Bosn 0.1.3 native evidence: `doctor` healthy (+0.130 s), `CARGO_BUILD_JOBS=1`; fmt, test-support check/clippy, parser/identity, and containment passed (6 outer; 2 intended ignored inner helpers, including pidfd crash and `PR_SET_PDEATHSIG` parent-death). All-feature check/clippy/full passed: 323 library + 54 symbolize + 14 worker tests, integrations/docs, and containment 6/2. No-default check/test passed: 112 library tests plus integrations; its tree excludes Wasmtime, wasmparser, and wat. Persisted jobs `j15-cbae57c3`–`j20-805af44d`; cleanup `j22-ae169c23` left zero sessions/artifacts. |
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
- `platform::window_icon` (feature `window-icon`): window and stock icon
  mechanics for the host console or a child, including the ICO/PNG decode and
  the X11 property write. GUI hosting is opt-in, so a headless client does not
  compile it. This gates the code and public surface, not yet the dependency
  graph: `running-process` declares `png` and `x11rb` non-optionally on Linux,
  so they still resolve in a default build until the substrate gates its own
  copy. `platform::window_icon`, `set_window_icon_impl` and
  `window_icon_support_impl` were available on the default feature set before
  this release; enabling `window-icon` is required as of it.
- `daemon_identity`, `daemon_frame_v1`, `daemon_registration`,
  `daemon_registration_v2` (features of the same names, each opt-in and
  outside `full`): the frozen v1/v2 daemon wires -- identity/sidecar/probe/mux
  semantics, the frame envelope codec, and the two registration record sets
  with their owner-private persistence. Endpoint naming, payload protocols,
  broker negotiation, and daemon lifecycle remain application policy. A client
  migrating off a direct substrate dependency takes these from here rather
  than reimplementing the records.

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

`running-process` is the lower native/process implementation layer and
`kernal-api` is its higher semantic facade. That private dependency has landed:
it is a mandatory, non-optional dependency of the current release, so the
client-side ban is live rather than pending. The reverse dependency is
forbidden. First-party applications migrate to a single direct systems
dependency on `kernal-api` and may not expose backend types in their public
APIs.

Migration branches may temporarily carry both dependencies while a capability
is moved. The corresponding strict Dylint rule is enabled as soon as the facade
has parity, and release branches must not retain the legacy direct dependency.
Exact backend versions are selected by `kernal-api`, not by its clients.
