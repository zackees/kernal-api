# Wasm-driven viewport screenshot (in progress)

This private example is the application requested by #20. Its business logic
is in `guest/src/main.rs`, compiled as one Rust 1.95.0
`wasm32-wasip1-threads` module. The guest receives opaque URL and exact-output
grants, then performs open → matching load completion → five-second kernel
sleep → native viewport capture → opaque output write → close. It neither
receives a URL/path string nor reads PNG bytes into guest memory.

## Current acceptance stage

The application source deliberately exposes the missing generated webview
contract. `WebviewUrl`, the generated asynchronous wait/driver glue, and the
native CLI integration are not implemented yet. This is not a working
screenshot runner and is not enabled as a passing CI gate. The real-runner
RED → GREEN proof remains incomplete until the native CLI loads this artifact
and the complete guest sequence executes against the offline native fixture.

Build the actual guest on a Unix host with the pinned target installed:

```sh
soldr --no-cache rustup target add wasm32-wasip1-threads
CARGO_TARGET_DIR="$PWD/target/screenshot-proof" bash examples/wasm-tauri-screenshot/build-guest.sh
```

The build currently fails at the missing generated API, before producing an
admitted module. Keep that failure distinct from native execution evidence.
The build script never runs a native replacement for the guest sequence.

The intended native executable remains:

```text
kernal-api-wasm-tauri --url <http-or-https-url> --output <screenshot.png>
```

Its remaining work includes pre-instantiation grants, import validation,
shared logical-root webview ownership, structured ABI/event tracing, typed
failure reporting, and deterministic window/blob/output teardown. Native
Windows/macOS execution and a PowerShell build entry point remain required.
