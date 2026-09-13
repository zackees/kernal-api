# External webview isolation

The `tauri-webview` feature is a native desktop capability. It is excluded
from `default` and `full`, and its dependencies are target-scoped so a Wasm
guest target cannot resolve a platform webview runtime.

The private backend creates a plain `tauri_runtime::PendingWindow` on the
canonical `tauri_runtime_wry::Wry` event loop, then attaches a direct
`wry::WebViewBuilder` without calling `with_ipc_handler` or adding custom
schemes. Default opens add no caller scripts. It does not use Tauri's
`WebviewWindowBuilder`, which installs an IPC bridge. The exact published Wry
0.57.0 pin makes bridge installation conditional on that omitted handler on
Windows and macOS. On Linux the adapter removes backend initialization scripts
and unregisters the IPC endpoint before navigation. A loopback proof asserts that
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

On Linux, before Wry initializes WebKitGTK, the facade enables
`JSC_useSharedArrayBuffer=1` unless the user supplied a value. When NVIDIA is
loaded it likewise sets `__NV_DISABLE_EXPLICIT_SYNC=1`, and disables the
DMA-BUF renderer only for an X11 backend. It corrects GTK's unknown/fractional
font DPI immediately before each view is built. These settings are private
host mechanics; explicitly supplied environment values always win.

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

Replace `close` with `popup`, `redirect`, `timeout`, `cancel`, or
`window-close` to exercise isolation and cleanup paths. The test-support
feature is accepted only for this executable proof and exposes aggregate
semantic counts, never backend types.

## Explicit page bootstrap

Native callers may opt into `open_webview_with_bootstrap` using a validated
`WebviewPageBootstrap`. Its trusted source is limited to 64 KiB of UTF-8 and
cannot contain NUL. The limit bounds stored source, not execution time or page
allocations. JavaScript runs at document start in the ordinary page world;
syntax errors use normal page error reporting rather than becoming open errors.
It is not a native IPC capability and must not contain secrets or untrusted
remote input interpolated as code.

Bootstrap views restrict navigation to the initial HTTP(S) origin. Both the
native policy and a main-frame/origin JavaScript guard apply; the latter is
necessary because WebView2 can inject initialization scripts into subframes.
The script runs again on same-origin reloads. Default script-free opens retain
their existing navigation policy.

Run the `bootstrap` smoke scenario to verify execution before the first page
script, re-execution on reload, absence in a same-origin child frame, absence
of the tested IPC bridges, and rejection of cross-origin navigation. The HTTP
fixture permits automatic favicon requests without counting them as an expected
page request; it still rejects other unexpected paths. The fixture regression
tests that distinction independently of native browser scheduling.
