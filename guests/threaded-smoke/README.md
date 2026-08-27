# Threaded Rust smoke guest

This is a standalone, source-only Rust 1.95 fixture for
`wasm32-wasip1-threads`. It is intentionally not a workspace member and has
no checked-in lockfile, Wasm binary, or build output.

It imports `kernal-api:v1::kernel-yield`, creates and joins one standard
thread using `Arc<AtomicU32>`, and exports `kernal-api-run` with a numeric
result marker. It deliberately performs no output, environment, filesystem,
network, clock, or randomness operations.

Run `scripts/build-threaded-smoke.ps1` on Windows or
`scripts/build-threaded-smoke.sh` on Unix. Both scripts use Soldr, build into a
temporary target directory, and hand the real Wasm artifact to the focused host
characterization test. The current admission policy is expected to reject it;
the test prints a stable semantic manifest that a later GREEN slice can check
in as an expected artifact profile.
