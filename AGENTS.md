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
- Do not add a second allocator, pprof schema, Tokio Console stack, crash
  handler, or OS HAL behind a runtime fallback. This crate is the canonical
  owner.
- First-party clients pin an exact pre-1.0 release. Local path patches are
  migration-only and must not reach a release branch.
- `0.0.0` is an unusable registry reservation and must never be restored as a
  dependency fallback or published from this source branch.
