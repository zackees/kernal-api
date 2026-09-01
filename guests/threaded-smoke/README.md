# Threaded Rust smoke guest

This is a standalone, source-only Rust 1.95 fixture for
`wasm32-wasip1-threads`. It is intentionally not a workspace member and has a
checked-in lockfile, but no Wasm binary or build output.

It imports `kernal-api:v1::kernel-yield`, creates and joins two ordinary Rust
child threads, and exercises `Arc<AtomicU32>`, `Mutex`, `mpsc`, and a
deterministic `DashMap` result. Its public manifest is deliberately exact: no
validation metadata or report export expands the `threaded-rust-v1` admission
surface. The root instance enters `_start` once; each child instance enters
`wasi_thread_start(tid, arg)` once. It deliberately performs no output,
environment, filesystem, network, clock, or randomness operations.

Run `scripts/build-threaded-smoke.ps1` on Windows or
`scripts/build-threaded-smoke.sh` on Unix. Both scripts use Soldr's `rustup`
front end to provision `wasm32-wasip1-threads`, build into a temporary or
caller-managed target directory, and run the real artifact through the public
admission profile. Soldr 0.9.11 is the first locally verified release for this
path. The Bash entry point requires a writable `CARGO_TARGET_DIR` and uses its
`kernal-api-threaded-smoke` subdirectory, which keeps it compatible with
Bosn's read-only source mount. The temporary guest build deliberately runs
with Soldr's cache disabled: it characterizes admission, not cross-target
cache materialization.

The guest's target configuration pins the bundled Wasm linker and explicitly
exports `kernal-api-run`; the build scripts select Soldr's default linker only
for that guest compilation so the ambient host-linker choice cannot alter the
closed artifact ABI.

The cache-disabled build avoids the cached cross-target materialization failure
tracked by [soldr#2919](https://github.com/zackees/soldr/issues/2919). Supplying
a direct artifact remains a diagnostic lane: it distinguishes source build
failure from closed-profile admission failure.
