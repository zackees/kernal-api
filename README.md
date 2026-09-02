# kernal-api

`kernal-api` is the shared systems facade for Soldr, zccache, and fbuild. Its
target architecture builds on `running-process`, the trusted low-level
native/process substrate, and adds stable application contracts for async
execution, hashing, diagnostics, profiling, symbolization, allocation,
networking, storage, and other common capabilities. The private
`running-process` phase-1 adapter is implemented on the
`feat/running-process-adapter` migration branch and uses the exact published
`running-process` 4.10.10 registry release without exposing backend types.

In the target architecture, applications use `kernal-api`; they do not use
`running-process` or Tokio directly. The permanent dependency direction and
staged migration are defined in [ARCHITECTURE.md](ARCHITECTURE.md).

The name is intentionally spelled **kernal-api**. The spelling is the stable
package and repository identity on crates.io, PyPI, and GitHub.

## Compatibility

- Rust 1.95.0, edition 2021
- Python 3.10 or newer for the PyPI companion package
- Linux, macOS, and Windows
- x86-64 and ARM64
- `kernal_api::async_engine`, backed by exactly Tokio 1.53.1 in this release
- `kernal_api::hash`, with kernel-owned BLAKE3 byte, reader, and file digest
  operations

The async facade also owns cooperative cancellation plus separate connection
and transfer-progress timeout policies. A connection is bounded by a fixed
deadline; a transfer is bounded by an idle budget that is reset only when the
caller records meaningful progress. Clients therefore do not need to expose
Tokio or choose an unsafe global download timeout.

Consumers must pin the same exact `kernal-api` release while the API is below
1.0. There is no compatibility fallback to the `0.0.0` namespace reservation:
the crates.io copy is yanked and the PyPI copy has the impossible
`Requires-Python: <0` marker.

See [COMPATIBILITY.md](COMPATIBILITY.md) for the client contract and feature
matrix. Client repositories install the [boundary Dylint](DYLINT.md) to reject
direct use of implementation crates owned by this package.

## Rust features

The current base crate contains the async process/host facade. On the phase-1
migration branch, its bounded process adapter privately uses
`running-process` 4.10.10 without exposing backend types. Optional features
keep consumers from linking tooling they do not use:

- `fs`, `ipc`, `ipc-async`, `session-relay`, `pty`, `conpty-sidecar`
- `snapshot` for cooperative thread capture and deferred unwinding
- `crash` for the one native crash-handler and bounded crash spool
- `profile` for bounded CPU profiles and checked-in pprof encoding
- `allocator` for dormant mimalloc sampling and heap dumps
- `tokio-console` for off-CPU task profiles and runtime diagnostics (the
  backend name is diagnostic metadata, not the application API)
- `symbolize` for the worker wire/client API; `symbolize-worker` builds the
  isolated `kernal-symbolize` parser executable
- `wasm-sketch-host` for opt-in core-Wasm sketch admission; the real threaded
  Rust artifact fixture remains source-only under `guests/threaded-smoke`
- `full` for diagnostic executables that need the entire surface

The library never installs a global allocator or subscriber by surprise.
Applications opt in explicitly and can still compile all facilities into one
final executable without allocator, crash-handler, pprof-schema, or Tokio
Console version collisions.

Process-to-process and durable machine-readable contracts use protobuf with
fixed field numbers. JSON is reserved for human/tool export formats such as a
Firefox profile; it is not an IPC control protocol. See
[PROTOCOLS.md](PROTOCOLS.md) for the wire-format rules and the deliberately
signal-safe crash-journal exception.

## Compile-resource ownership

First-party clients compile through Soldr. Its default native-cache route
wraps both Rust compilation and `cc`/`c++` build-script work, so the
facade-owned native sources share zccache and Soldr's oversized-unit resource
gate. Large or known amalgamated C/C++ files and the published zccache Rust
amalgamation receive exclusive compile admission instead of competing with a
full set of ordinary compiler children.

That scheduler protection is separate from this crate's API boundary. The
boundary Dylint prevents clients from adding their own copies of the runtime,
allocator, profiler, symbolizer, and OS-HAL implementation dependencies,
including direct use of `running-process` after the relevant facade is ready.
It does not rename or combine third-party source files.

## License

BSD 3-Clause, matching the platform implementation from running-process.
