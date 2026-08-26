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
  depend on `kernal-api`, which will privately depend on `running-process` when
  phase 1 lands. Never introduce the reverse dependency.
- Keep backend types private. Public APIs use facade-owned semantic types rather
  than re-exporting `running-process`, Tokio, or another implementation crate.
- Do not add a second allocator, pprof schema, Tokio Console stack, crash
  handler, or OS HAL behind a runtime fallback. This crate is the canonical
  owner.
- Keep the broker implementation in `running-process` during phase 1. Hoist the
  generic broker-daemon pattern only after client migrations stabilize, without
  changing application payloads or adding round trips.
- Treat one supported API and one compilation unit as separate choices. Add
  private crates or release amalgamation only when measured Soldr/Cargo timings
  justify them; do not add speculative traits without multiple real backends.
- First-party clients pin an exact pre-1.0 release. Local path patches are
  migration-only and must not reach a release branch.
- `0.0.0` is an unusable registry reservation and must never be restored as a
  dependency fallback or published from this source branch.
