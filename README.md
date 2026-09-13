# kernal-api

`kernal-api` is the shared systems facade for Soldr, zccache, and fbuild. Its
target architecture builds on `running-process`, the trusted low-level
native/process substrate, and adds stable application contracts for async
execution, hashing, diagnostics, profiling, symbolization, allocation,
networking, storage, and other common capabilities. The private
`running-process` phase-1 adapter has landed: this crate depends on the exact
published `running-process` 4.10.10 registry release unconditionally, and does
not expose backend types.

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

The base crate contains the async process/host facade. Its bounded process
adapter privately uses `running-process` 4.10.10 without exposing backend
types; that dependency is mandatory, not feature-gated. Optional features keep
consumers from linking tooling they do not use:

- `sqlite` for synchronous, bounded SQLite connection/transaction/query and
  backup mechanics; applications retain schema and SQL. See [SQLite facade](docs/sqlite.md).

- `fs`, `ipc`, `ipc-async`, `session-relay`, `pty`, `conpty-sidecar`
- `fs` also enables `hash::blake3_tree`: content-authoritative fingerprints of
  glob-selected directory trees, with bounded parallel streaming reads and a
  versioned path/content encoding. Include/exclude rules remain caller policy.
  Defaults admit at most one million selected files, 16 GiB per file, and eight
  readers (limited by host parallelism). Digests ignore absolute roots and
  timestamps; symlinks are skipped and non-UTF-8 regular-file paths fail explicitly.
- `fs-watch` for filesystem-change watcher construction and event
  classification (created/modified/removed/renamed plus an explicit
  overflow-or-lost-watch rescan signal); debouncing, ignore-lists, and
  cache-invalidation policy stay with the application
- `snapshot` for cooperative thread capture and deferred unwinding
- `crash` for the one native crash-handler and bounded crash spool
- `profile` for bounded CPU profiles and checked-in pprof encoding
- `allocator` for dormant mimalloc sampling and heap dumps
- `tokio-console` for off-CPU task profiles and runtime diagnostics (the
  backend name is diagnostic metadata, not the application API)
- `symbolize` for the worker wire/client API; `symbolize-worker` builds the
  isolated `kernal-symbolize` parser executable
- `symbolize-split` for post-link debug-symbol splitting: given a linked
  binary it produces a stripped binary plus a complete, matched symbol file,
  reports the mechanism obtained (`gnu-debuglink`, `dsym-bundle`, or the
  `.pdb` the MSVC linker already wrote), and can prove the pair resolves a
  known function through the isolated worker rather than trusting file sizes
- `window-icon` for host-console and child window/stock icons. This is GUI
  hosting: on Linux it decodes PNG and speaks the X11 client protocol. Unlike
  the other optional backends this gates the code and public surface rather
  than the dependency graph: `running-process` declares `png` and `x11rb`
  non-optionally on Linux, so a headless build still resolves them until the
  substrate gates its own copy. **Source break:** `platform::window_icon`,
  `set_window_icon_impl` and `window_icon_support_impl` were available on the
  default feature set before this release and now require `window-icon`
- `wasm-sketch-host` for opt-in core-Wasm sketch admission; the real threaded
  Rust artifact fixture remains source-only under `guests/threaded-smoke`
- `full` for diagnostic executables that need the entire non-daemon surface

The four daemon slices are deliberately outside `full`, because each one
carries a frozen wire that only an application already speaking it should
compile. They are documented on docs.rs but must be enabled by name:

- `daemon-identity` for direct-daemon identity, sidecar, probe, and
  endpoint-mux semantics over an existing endpoint; endpoint naming, payload
  protocols, and daemon lifecycle stay with the application
- `daemon-frame-v1` for the frozen v1 daemon-frame envelope codec alone,
  independent of identity, broker IPC, hashing, and runtime
- `daemon-registration` for the frozen v1 registration records and
  owner-private persistence, excluding endpoint/client policy and identity
- `daemon-registration-v2` for frozen v2 service-definition registration
  alone, so an application dual-writing during the v1-to-v2 rollout pulls no
  broker client, identity, IPC, or runtime policy

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
