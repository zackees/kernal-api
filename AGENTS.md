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
- Follow the target graph in [ARCHITECTURE.md](ARCHITECTURE.md): applications
  depend on `kernal-api`, which privately depends on `running-process`. Phase 1
  has landed; the dependency is mandatory and asserted by
  `tests/facade_policy.rs`, so do not add it a second time. Never introduce the
  reverse dependency.
- Keep backend types private. Public APIs use facade-owned semantic types rather
  than re-exporting `running-process`, Tokio, or another implementation crate.
  Approved canonical exceptions are documented and policy-tested narrowly:
  issue #189's independent-spawn contract, `foreground`, the opt-in `broker`
  contract, `daemon_frame_v1` and its registration macro, and the renamed
  `daemon_registration` / `daemon_registration_v2` namespaces (including
  `daemon_registration_v2::canonical`). The selected `async_process`,
  `process`, `containment`, liveness/priority, and native process primitives
  are likewise enumerated in `tests/facade_policy.rs`.
  Re-export only their reviewed
  `running-process` symbols with `pub use` or `pub use ... as ...`. Preserve
  Rust type identity; do not add matching facade enums, wrapper types,
  conversion tables, a whole-crate re-export, or an implicit fallback from
  `Independent` to `Inherited`.
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
- Read [docs/platform-boundary.md](docs/platform-boundary.md) before changing
  a platform capability. Its target structure and enforcement plan are tracked
  by #152. Until that issue lands, the repository's platform Dylint deliberately
  exempts `kernal-api` itself, so passing Dylint alone is not evidence that new
  facade-owner source follows this boundary.
