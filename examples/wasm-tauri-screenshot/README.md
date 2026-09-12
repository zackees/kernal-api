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
generated contract, native dispatch, and CLI now run the actual sequence.
On Linux x86-64, the checked-in offline CLI proof produced an 800×600,
2,809-byte PNG in about 7.5 seconds, preserved an unrelated file, and left no
temporary output. The portable Rust decoder verifies red and blue regions
at six interior sample points. An initial native run exposed a
redundant guest close after output commit; commit consumes the snapshot and
output grants, so the guest now proceeds directly to closing the view.

This is still an in-progress #20/#21 implementation, not completion of the
full acceptance matrix. Broader failure-path trace coverage,
queued/cancelled native-creation destruction evidence, the complete
failure matrix, killable native-webview worker integration, and native
Windows/macOS execution remain outstanding. Do not treat this in-process
diagnostic CLI as containment for arbitrary untrusted Wasm.

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

PowerShell 7 uses the equivalent checked-in entry point and absolute storage:

```powershell
soldr --no-cache rustup target add wasm32-wasip1-threads
$env:CARGO_TARGET_DIR = Join-Path (Get-Location).Path 'target/screenshot-proof'
./examples/wasm-tauri-screenshot/build-guest.ps1
$env:KERNAL_API_SCREENSHOT_ARTIFACT_WASM = Join-Path $env:CARGO_TARGET_DIR 'kernal-api-wasm-tauri-guest/wasm32-wasip1-threads/release/kernal-api-wasm-tauri-guest.admitted.wasm'
soldr cargo test --locked --features wasm-sketch-host --test wasm_tauri_screenshot -- --ignored
```

Both scripts require the pinned Wasm target to be installed, preserve the
caller's build storage, and fail on build/metadata errors. The PowerShell
entry restores the caller's working directory and `SOLDR_LINKER` setting.
Building a guest this way does not prove native Windows webview execution.
The existing Windows threaded-artifact CI lane invokes this script and the
host-only admission test without enabling GUI dependencies. The script's
actual build and caller-state restoration have been checked with PowerShell
on Linux; native Windows validation remains separate.

That test checks closed-profile admission and rejection without webview grants,
not successful native capture. It also mutates the real artifact's import
namespace and requires pre-compilation rejection. The original admission
check required every compatibility import used by the threaded smoke guest;
this smaller program exposed that mismatch. Admission now accepts a subset
of the same exact name/signature allowlist, while requiring a complete
generated lifecycle (or the legacy kernel-yield boundary). No new import or
ambient authority was allowed by this change.

To run the real offline native CLI regression, supply the same artifact and
enable both capabilities on a host with a working native display:

```sh
KERNAL_API_SCREENSHOT_ARTIFACT_WASM="$PWD/target/screenshot-proof/kernal-api-wasm-tauri-guest/wasm32-wasip1-threads/release/kernal-api-wasm-tauri-guest.admitted.wasm" \
  soldr cargo test --locked --features wasm-sketch-host,tauri-webview-test-support --test wasm_tauri_screenshot -- --ignored --nocapture --test-threads=1
```

The native test launches the CLI as a separate process, serves the checked-in
fixture over loopback, verifies successful guest completion, decodes the PNG
with portable Rust tooling, checks bounded dimensions and tolerant red/blue
regions at six interior points, and checks exact-output replacement and
temporary-file cleanup. A bounded host trace records generated submit/poll/yield
crossings, preserves the matching native load callback's monotonic timestamp,
and requires capture at least five seconds later. After joined cleanup and
dropping the admitted sketch, the proof requires zero native admission,
backings, hub resources/operations/transfer bytes, clock/output jobs, Wasm
roots/threads/stores/instances, epoch registrations, and memory reservations.
An additional non-ignored CLI test rejects malformed URLs, credentials, and
file/data/javascript/tauri schemes before module loading. It needs no display
and verifies unchanged output/sentinel files, with a valid-URL negative control
that reaches the separate missing-module error.
The recorder holds at most 512 events; any omitted event fails the proof.
The actual-guest redirect proof supplies an HTTP redirect to a prohibited
scheme, requires `screenshot-load-rejected`, preserves the original output and
unrelated file, and applies the same zero-resource teardown assertions.
The timeout proof holds the real HTTP request open for the production
30-second load deadline, requires `screenshot-load-timed-out`, forbids capture,
and applies the same output-preservation and teardown checks.
The write-failure proof relocates the fixture-owned output directory only
after the guest's HTTP request, so grants and native execution have already
started. The guest completes real capture before its output job encounters
the now-missing parent. The proof requires `screenshot-write-rejected`, no
recreated parent or sibling temporary file, unchanged preserved files, and
zero captured blobs/output jobs/native/Wasm resources. This is a missing-parent
I/O failure, not yet a portable permissions or disk-exhaustion proof.
These observations do not establish the remaining failure matrix.
Run under Xvfb on headless
Linux, using the environment in `docs/tauri-external-content-isolation.md`.

The dedicated `wasm-tauri-screenshot-linux` CI job builds this actual guest
and runs both proofs with WebKitGTK 4.1 under Xvfb. Set
`KERNAL_API_SCREENSHOT_PROOF_DIR` to retain a unique diagnostic directory
containing runner stdout/stderr, process outcome/timing JSON, and the exact
output directory even if an assertion fails. CI uploads these artifacts on
success or failure. `runner.stderr.log` includes structured
`kernal-webview-trace` records; `process.json` separately summarizes the child
process. Hard termination can prevent the bounded trace from being emitted,
so failure-path/containment diagnostics remain incomplete. This Linux x86-64 lane does not
establish native execution on the other five supported host targets.

Focused formatting and lint checks:

```sh
soldr cargo fmt --all -- --check
soldr cargo clippy --locked --features wasm-sketch-host,tauri-webview-test-support --lib --tests --bins -- -D warnings
soldr cargo test --locked --features wasm-sketch-host,tauri-webview-test-support --test wasm_tauri_screenshot
```

The generated command driver handles only
generated kernel futures: Wasmtime suspends the guest stack at its async host
import, so no guest runtime is created. Foreign futures returning `Pending`
are rejected rather than busy-polled by an alternate scheduler.

Run the native executable from this source checkout:

```sh
soldr cargo run --locked --features wasm-sketch-host,tauri-webview --bin kernal-api-wasm-tauri -- --url http://127.0.0.1:8000/ --output screenshot.png
```

Without `--module`, the CLI builds the checked-in guest through Soldr and
embeds its generated metadata. `--module <artifact>` selects an already-built
diagnostic artifact; either path validates imports before instantiation.
The default build-and-run path has also completed against the offline fixture
on Linux x86-64, producing the same 800×600 red/blue viewport.
URL and exact-output grants are installed before module start. The UI service
gets the root's existing hub and the same runtime, never copied tokens from a
different client hub. Native jobs are bounded and joined; cancelled native
creation and capture retain separate four-request admission until callbacks
release them. Failed root execution reports a semantic host error; full guest
status includes its failing step and semantic operation error. The shared
`status.rs` encodes eight steps and five causes in the already-admitted
`proc_exit` scalar; it introduces no import, path, payload, or logging grant.
For example, missing URL authority is exit 17, a rejected load is exit 65,
and a timed-out load is exit 69.
The CLI uses a finite 120-second whole-root epoch budget, leaving room for
the production 30-second operation deadlines and five-second wait. The
generic compiler default remains 30 seconds; using it unchanged here masked
the load timeout with a whole-root `deadline-exceeded` error.
Actual traps remain `trapped`, distinct from these reported failures. The
generated poll decoder preserves rejected status 7 rather than collapsing it
to a generic failure; status 3 is preserved as `TimedOut`. The rest of the
negative matrix and native Windows/macOS execution remain required.
