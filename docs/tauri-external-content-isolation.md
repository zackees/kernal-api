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

`WebviewUrlGrant::new` validates a URL before effects and bounds both its input
and canonical form to 16 KiB. `open_granted_webview` binds that value to the
client's instance in the same operation/resource hub. Its open borrows the URL
grant; revocation cancels an unfinished open and reclaims the unpublished view.
Cancellation before resource attachment also rolls back both reservations.
`open_webview(&str)` uses this same path, with a temporary grant released on
success, error, or future drop. An already completed view owns its separate
view authority, so releasing the temporary URL grant does not close it.

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
generated screenshot guest now uses URL-grant/open/load/capture/close
operations through a root-scoped native dispatcher. URL/output grants precede
instantiation and every operation uses the root's original hub and runtime.
See `examples/wasm-tauri-screenshot` for the actual CLI/guest proof and its
remaining tracing, containment, and platform acceptance work. Native-only
smoke tests remain distinct from that end-to-end Wasm proof.

On Linux, renderer environment must be configured by the application launcher,
before starting the process. The safe library constructor does not mutate
process environment: an existing runtime or application thread may already
be reading it. Where needed, launch with `env JSC_useSharedArrayBuffer=1 app`;
on NVIDIA, also supply `__NV_DISABLE_EXPLICIT_SYNC=1`, and for an X11 backend
`WEBKIT_DISABLE_DMABUF_RENDERER=1`. These are launch-time choices, not defaults
that override explicitly supplied user values.

GTK's unknown/fractional font DPI is corrected before each view. Correction
remembers the original desktop setting per GTK settings object on the UI
thread, so repeated views do not divide an already corrected value. Observed
desktop-setting and monitor-scale changes are recomputed from the original;
an external reset exactly equal to the last applied value is indistinguishable.
The repeated-view, scale-change, desktop-change, and unknown-DPI regression
tests failed with the original cumulative division and pass with this state.

`WebviewPermissions` is deny-by-default. Call
`open_webview_with_permissions(url, WebviewPermissions::deny_all().allow_user_media())`
only for pages that should receive microphone/camera access. On Linux this
enables WebKitGTK media streams and accepts only user-media permission
requests; all other WebKit permission kinds remain denied. GStreamer core,
base, good, and PipeWire plugins must be available to WebKitGTK for devices to
enumerate (notably, an unwrapped Nix shell may need to expose them).

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
It separately waits for native creation admission to drain after releasing
the paused UI. This does not establish every native destruction race on
every platform. Creation admission, like capture admission, is held through
the native callback even after semantic cancellation consumes the operation.
The live image test separately decodes PNG pixels and verifies
viewport dimensions and fixture colors. These Linux gates do not establish
the still-required Windows/macOS runtime proofs or the generated Wasm sketch.
See [adapter ownership and remaining evidence](viewport-capture-adapters.md).
