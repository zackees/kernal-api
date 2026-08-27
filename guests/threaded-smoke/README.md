# Threaded Rust smoke guest

This is a standalone, source-only Rust 1.95 fixture for
`wasm32-wasip1-threads`. It is intentionally not a workspace member and has a
checked-in lockfile, but no Wasm binary or build output.

It imports `kernal-api:v1::kernel-yield`, creates and joins two ordinary Rust
child threads, and exercises `Arc<AtomicU32>`, `Mutex`, `mpsc`, and a
deterministic `DashMap` result. The validation fixture carries the validation
profile metadata and bounded report export. The root instance enters `_start`
once; each child instance enters `wasi_thread_start(tid, arg)` once. It
deliberately performs no output, environment, filesystem, network, clock, or
randomness operations.

Run `scripts/build-threaded-smoke.ps1` on Windows or
`scripts/build-threaded-smoke.sh` on Unix. Both scripts use Soldr, build into a
temporary or caller-managed target directory, and run the private validation
lane against the real artifact. The Bash entry point requires a writable
`CARGO_TARGET_DIR` and uses its `kernal-api-threaded-smoke` subdirectory,
which keeps it compatible with Bosn's read-only source mount.

The normal Cargo artifact build is currently expected to be blocked only by
[soldr#2919](https://github.com/zackees/soldr/issues/2919). Supplying a direct
artifact remains a diagnostic lane: it distinguishes Soldr materialization
failure from admission, root/child lifecycle, or validation-report failures.
