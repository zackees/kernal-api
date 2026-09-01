# Compilation-boundary measurement protocol

This is the reproducible measurement gate for issue #3 and the phase-0 input
to #5. It measures a physical package choice without changing the public
`kernal-api` facade: a clean build, an `src/lib.rs` incremental edit, Cargo's
critical path (the measured Cargo wall clock; the `cargo-timings` HTML is
retained with each target directory), peak compiler-process RSS, Soldr cache
snapshots, debug output size, and the packaged `.crate` size.

## Candidates and decision rule

The initial and selected candidate is the cohesive single `kernal-api` package.
The actual pre-adapter baseline is commit
`f745dcff4e2874170b779c510c698258aee90055`, the parent of `2cb0bb2`
(`feat(process): use running-process substrate`). It is not a synthetic
reconstruction. It is a substrate-migration observation, not a packaging A/B:
the intervening history also changed facade and platform code. Do not use it to
claim a physical-boundary win. A future private-crate candidate must instead
measure two otherwise identical revisions with the same public facade and
feature set.

No private implementation crate is selected or benchmarked yet: adding one
solely to create a comparison would violate the issue's decision against
speculative boundaries. A proposed split must first identify a cohesive module
with an implementation-only dependency graph. It then runs this exact protocol
against an otherwise identical facade and feature set, declares `publish =
false`, and adds a facade-policy test proving no backend type escapes. A split
is adopted only for a repeatable material win in clean/incremental critical
path, peak memory, cache reuse, or output size that outweighs its maintenance
cost.

Release amalgamation is not selected. Before it may be adopted, the candidate
must produce byte-identical archives from two clean source trees with fixed
`SOURCE_DATE_EPOCH`, then run `soldr cargo package --locked` and test the
unpacked archive. Its archive size and build results belong in the same result
set; otherwise this repository remains normally packaged by Cargo.

## Commands

Use an empty or disposable output directory. The script deletes only its own
`OUTPUT/targets/<label>-N` directories and invokes `soldr cargo package` with
`--allow-dirty --no-verify` to record the source archive.

```bash
git worktree add ../kernal-api-baseline f745dcff4e2874170b779c510c698258aee90055
uv run --no-project benchmarks/compilation-boundaries/measure.py \
  --source ../kernal-api-baseline --label pre-adapter --repeat 3 \
  --output benchmarks/compilation-boundaries/results/raw
uv run --no-project benchmarks/compilation-boundaries/measure.py \
  --source . --label adapter --repeat 3 \
  --output benchmarks/compilation-boundaries/results/raw
uv run --no-project ci/check_compilation_boundary_dependencies.py
```

Use `--features wasm-sketch-host` for the heavyweight representative workload
and record it separately. The script uses the host `time -f '%e %M'` command on Linux;
on other hosts it records `null` peak RSS rather than guessing. Run the native
platform lane to collect that platform's peak-RSS number. A Soldr daemon that
does not answer `soldr cache --json` is reported as unavailable; do not infer
cache hits from elapsed time.

The checked-in result was collected on the host, toolchain, and command lines
stored in its JSON. Raw Cargo timing HTML, complete stdout/stderr, and Soldr
cache snapshots are collection artifacts and intentionally stay outside Git;
the JSON contains their summarised evidence and exact rerun recipe.

## Dependency RED -> GREEN check

`ci/check_compilation_boundary_dependencies.py` uses `soldr cargo tree`, not
`Cargo.lock`: the lockfile includes optional packages and cannot prove they are
disabled. For each pair it first proves the default graph excludes the package
(RED condition) and then proves enabling the owner feature resolves it (GREEN):
`wasm-sketch-host`/`wasmtime`, `ipc`/`interprocess`,
`tokio-console`/`console-subscriber`, and `allocator`/`mimalloc-pprof`.
