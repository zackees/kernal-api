# Agent instructions

- Route Rust toolchain commands through `soldr`, for example
  `soldr cargo test --all-features` and `soldr cargo fmt --all`.
- Route Python tools through `uv run --no-project`; never invoke `python` or
  `python3` directly. Build packages with `uv build --clear` so a prior
  namespace-reservation artifact cannot enter a release directory.
- Rust 1.95.0 is the MSRV and pinned toolchain. Python support starts at 3.10.
- Support Linux, macOS, and Windows on x86-64 and ARM64.
- Keep heavyweight facilities feature-gated. `default = []` must remain a
  useful async process/host HAL without profiling dependencies.
- Add an integration test as a module of an existing category in `tests/`, not
  as a new top-level file. Every top-level `tests/*.rs` is its own linked
  binary that statically links this crate's whole graph -- Wasmtime, Cranelift,
  Tauri and all -- so one per file costs about 180 MB and a full link each.
  Sixty of them made a 1.7 GB test archive; the nine categories
  (`tests/<category>/main.rs` declaring each file as a module) make it 619 MB
  and eleven links. Test IDs are `<category>::<module>::<test>`, so a
  `--exact` filter or a test that re-execs itself by name carries the module
  prefix. A file keeps its own `#![cfg(feature = "...")]`, which is what lets a
  category compile under any feature subset; do not reach for
  `required-features` on the category target. A new top-level test file needs a
  stated reason. Today: `wasm_tauri_screenshot` and `wasm_worker_containment`,
  which `ci/native_proof.py` ships to native hosts by target name, and
  `allocator_heap_profile`, which starts the process-global heap profiler while
  another test asserts it is dormant. Process-global, one-way state is the
  standing reason a test earns its own binary -- sharing one made those two
  order-dependent, and nextest hid it by running every test in its own process
  while `cargo test` failed. This mirrors soldr#2934.
- Follow the target graph in [ARCHITECTURE.md](ARCHITECTURE.md): applications
  depend on `kernal-api`, which privately depends on `running-process`. Phase 1
  has landed; the dependency is mandatory and asserted by
  `tests/facade_policy.rs`, so do not add it a second time. Never introduce the
  reverse dependency.
- Keep backend types private. Treat `running-process` exactly like Tokio or any
  other implementation crate: public APIs use facade-owned semantic types and
  never re-export, alias, or name a backend type in a public position. There
  are no exceptions. Renaming with `pub use ... as ...` changes only the
  spelling, not the backend type a client receives. `tests/facade_policy.rs`
  and the `kernal_api_boundary` Dylint enforce this.
- Do not add a second allocator, pprof schema, Tokio Console stack, crash
  handler, or OS HAL behind a runtime fallback. This crate is the canonical
  owner.
- Keep the broker implementation in `running-process`. Hoist the generic
  broker-daemon pattern only after client migrations stabilize, without
  changing application payloads or adding round trips.
- Treat one supported API and one compilation unit as separate choices. Add
  private crates or release amalgamation only when measured Soldr/Cargo timings
  justify them; do not add speculative traits without multiple real backends.
- First-party clients pin an exact pre-1.0 release. Local path patches are
  migration-only and must not reach a release branch.
- `0.0.0` is an unusable registry reservation and must never be restored as a
  dependency fallback or published from this source branch.
- Keep host-platform selection structured: the one `std::cfg_select!` root in
  `src/lib.rs` selects a private concrete platform tree. Ordinary modules use
  the neutral crate-root facade and must not add `cfg(target_os = ...)`, name
  `platform_imp`/a concrete platform tree, or reach into `std::os`/native OS
  APIs. This does not prohibit host-neutral feature or test configuration;
  the rule is about selecting a host implementation. Guest/WASM target
  selection is a separate concern.
- Prefer duplication over a selector in the neutral facade. Every `cfg`
  selector the platform Dylint names -- `target_os`, `target_arch`, `unix`,
  `windows`, `target_env`, and the rest -- belongs in a concrete tree, even
  when the three copies would be byte-identical and the selection is not about
  the OS at all. `platform::host::cpu_compatibility_features` is the worked
  example: an x86 `cfg` pair in `src/platform/host.rs` would have been one
  copy instead of three, and it still moved into `platform_linux`,
  `platform_macos` and `platform_win`, with the neutral leaf re-exporting the
  bridged name. Three identical copies are cheaper to own than one exception
  to the boundary.
- Read [docs/platform-boundary.md](docs/platform-boundary.md) before changing
  a platform capability. Its target structure and enforcement plan are tracked
  by #152. Until that issue lands, the repository's platform Dylint deliberately
  exempts `kernal-api` itself, so passing Dylint alone is not evidence that new
  facade-owner source follows this boundary.
