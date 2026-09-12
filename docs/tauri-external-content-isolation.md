# External webview isolation

The `tauri-webview` feature is a native desktop capability. It is excluded
from `default` and `full`, and its dependencies are target-scoped so a Wasm
guest target cannot resolve a platform webview runtime.

The private backend creates a plain `tauri_runtime::PendingWindow` on the
canonical `tauri_runtime_wry::Wry` event loop, then attaches a direct
`wry::WebViewBuilder` without calling `with_ipc_handler`, adding scripts, or
adding custom schemes. It does not use Tauri's `WebviewWindowBuilder`, which
installs an IPC bridge. The exact pinned Wry fork makes bridge installation
conditional on that omitted handler. A loopback page-level proof asserts that
`window.ipc`, Tauri internals, and the platform IPC handler are all absent.
The view is incognito, disables clipboard, autofill, and devtools, refuses
downloads, and denies every popup/new-window request.

Only absolute HTTP(S) URLs with a host are accepted. This includes loopback
HTTP for native tests. `file:`, `data:`, custom schemes, credentials, malformed
URLs are rejected before a window is created; prohibited later navigations are
rejected by the native policy. A
`PageLoadEvent::Finished` only resolves the completion for its matching
requested top-level URL. A user close, failed close, popup, or rejected
navigation completes the owned callback with a typed private reason.

Wry owns the platform event loop and must be initialized and driven on its UI
thread. Creation uses Wry's supported handle routing from the facade-owned
async runtime's blocking lane; it never blocks the event loop and avoids the
Windows callback-creation deadlock. The backend does not create a Tokio runtime
or another OS runtime. The public `webview` facade owns semantic
`open_webview`, `wait_until_loaded`, terminal observation, and `close`
operations. They share the generated kernel operation/resource hub without
exposing a Tauri/Wry type or inventing a second registry. Timeout,
cancellation, user close, and rejected navigation revoke the resource
generation and wake pending operations with typed facade errors. The current
scalar guest ABI intentionally remains synthetic: it has no bounded URL
request transport, so native-only acceptance tests use this same hub path
until a generated URL capability is added.

For a repeatable Linux proof outside CI, use the Nix development shell below.
The `LD_LIBRARY_PATH` derivation is necessary when launching a Soldr-built
binary from the ephemeral shell rather than from a Nix-wrapped derivation:

```sh
nix-shell -p pkg-config gtk3 webkitgtk_4_1 xorg-server xauth xvfb-run --run 'LD_LIBRARY_PATH=$(printf "%s" "$NIX_LDFLAGS" | tr " " "\n" | sed -n "s/^-L//p" | paste -sd:); export LD_LIBRARY_PATH; GDK_BACKEND=x11 xvfb-run -a soldr --jobs 2 cargo run --locked --features tauri-webview-test-support --bin kernal-tauri-smoke -j 1 -- close'
```

Replace `close` with `popup`, `redirect`, `timeout`, `cancel`,
`window-close`, `capture`, `capture-cancel`, or `open-cancel` to exercise isolation and cleanup paths. The test-support
feature is accepted only for this executable proof and exposes aggregate
semantic counts, never backend types.

The existing `tauri-webview-smoke` CI job runs both capture scenarios and the
ignored live WebKitGTK image test explicitly. `capture` verifies the opaque
snapshot service, bounded reads, typed pixel/byte failures, and final cleanup.
`capture-cancel` pauses native UI dispatch, abandons four requests, verifies
the fifth is rejected while native admission remains held, and then drains
the queue. `open-cancel` abandons an open while UI dispatch is paused and
requires the reserved operation and resource counts to return immediately
to their pre-open baseline. Before the reservation guard, this real Linux
runner failed with two resources and two pending operations instead of one
of each; the guard now releases that semantic authority on future drop.
This counter check does not claim that native creation has already drained.
The live image test separately decodes PNG pixels and verifies
viewport dimensions and fixture colors. These Linux gates do not establish
the still-required Windows/macOS runtime proofs or the generated Wasm sketch.
See [adapter ownership and remaining evidence](viewport-capture-adapters.md).
