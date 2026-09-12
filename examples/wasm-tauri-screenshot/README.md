# Wasm-driven viewport screenshot (in progress)

This private example is the application requested by #20. Its business logic
is in `guest/src/main.rs`, compiled as one Rust 1.95.0
`wasm32-wasip1-threads` module. The guest receives opaque URL and exact-output
grants, then performs open → matching load completion → five-second kernel
sleep → native viewport capture → opaque output write → close. It neither
receives a URL/path string nor reads PNG bytes into guest memory.

## Current acceptance stage

Commit `52d4a3b` preserves the initial real Rust build failure: the generated
`WebviewUrl`, `run`, and `OperationFuture::wait` interfaces were absent. The
generated guest contract now supplies those interfaces and the application
compiles. The native host's webview opcode dispatch and CLI integration are
not implemented yet. This is not a working screenshot runner and is not
enabled as a passing CI gate. The real-runner RED → GREEN proof remains
incomplete until the native CLI loads this artifact and the complete guest
sequence executes against the offline native fixture.

Build the actual guest on a Unix host with the pinned target installed:

```sh
soldr --no-cache rustup target add wasm32-wasip1-threads
CARGO_TARGET_DIR="$PWD/target/screenshot-proof" bash examples/wasm-tauri-screenshot/build-guest.sh
```

The build produces one module with generated ABI metadata; metadata embedding
alone does not establish admission or successful execution. The build script
never runs a native replacement for the guest sequence. The focused artifact
test must be selected explicitly:

```sh
KERNAL_API_SCREENSHOT_ARTIFACT_WASM="$PWD/target/screenshot-proof/kernal-api-wasm-tauri-guest/wasm32-wasip1-threads/release/kernal-api-wasm-tauri-guest.admitted.wasm" \
  soldr cargo test --locked --features wasm-sketch-host --test wasm_tauri_screenshot -- --ignored
```

That test checks closed-profile admission and rejection without webview grants,
not successful native capture. It also mutates the real artifact's import
namespace and requires pre-compilation rejection. The original admission
check required every compatibility import used by the threaded smoke guest;
this smaller program exposed that mismatch. Admission now accepts a subset
of the same exact name/signature allowlist, while requiring a complete
generated lifecycle (or the legacy kernel-yield boundary). No new import or
ambient authority was allowed by this change.

The generated command driver handles only
generated kernel futures: Wasmtime suspends the guest stack at its async host
import, so no guest runtime is created. Foreign futures returning `Pending`
are rejected rather than busy-polled by an alternate scheduler.

The intended native executable remains:

```text
kernal-api-wasm-tauri --url <http-or-https-url> --output <screenshot.png>
```

Its remaining work includes pre-instantiation grants, import validation,
shared logical-root webview ownership, structured ABI/event tracing, typed
failure reporting, and deterministic window/blob/output teardown. Native
Windows/macOS execution and a PowerShell build entry point remain required.
