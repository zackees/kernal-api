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
