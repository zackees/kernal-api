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

Replace `close` with `widget`, `bootstrap`, `popup`, `redirect`, `timeout`, `cancel`,
`window-close`, `capture`, `capture-cancel`, or `open-cancel` to exercise
isolation and cleanup paths. The test-support feature is accepted only for
this executable proof and exposes aggregate semantic counts, never backend
types.

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

## Widget window controls (#384, first slice)

`WebviewWindowOptions` records four more presentation requests. They are
plain values and have no native effect until an open; the existing title and
size bounds still apply. None of them adds script, navigation, or IPC
authority, and the loopback no-IPC proof runs with all of them enabled.

```rust
let options = WebviewWindowOptions::new("bosn", 360, 240)?
    .decorations(false)
    .transparent(true)
    .always_on_top(true)
    .skip_taskbar(true);
let host = ExternalWebviewHost::with_app_id(runtime.handle(), "dev.bosn.widget")?;
```

`ExternalWebviewHost::with_app_id` validates the id before any native effect:
1 to 255 bytes of `[A-Za-z0-9._-]`, at least two dot-separated elements, none
empty and none starting with a digit. The narrower element rule is GLib's: GTK
refuses any other id, and tao would panic on it, so the facade rejects it on
every host with `WindowOptionsError::InvalidAppId`.

After an open, `WebviewHandle::set_size`, `show`, `hide`, and `focus` each
reserve a hub operation under a dedicated window right, dispatch through the
Wry window dispatcher, and complete once the native event loop has processed
the request. A revoked or closed handle returns `WebviewError::WindowClosed`,
like every other operation on a stale generation. `hide` keeps the page alive
and the handle valid. `set_size` uses the same 1-16384 logical-pixel bound as
the options and rejects other values before native effects.

What each request does depends on the host. "Toolkit" means the request
reaches the native toolkit; the compositor or window manager can still
refuse it.

| Request | Linux Wayland (GTK 3) | Linux X11 | Windows | macOS |
| --- | --- | --- | --- | --- |
| app id | `xdg_toplevel` app id | `WM_CLASS` from GLib program name | ignored | ignored (bundle id comes from `Info.plist`) |
| `decorations(false)` | applied (GTK draws no CSD) | applied | applied | applied |
| `transparent(true)` | RGBA window and WebKit background | same, needs a compositing WM | layered window and WebView2 background | WKWebView background only; the NSWindow stays opaque (needs Tauri's `macos-private-api`) |
| `always_on_top(true)` | ignored by the compositor; use a KWin window rule keyed on the app id | honoured by an EWMH window manager | `HWND_TOPMOST` | floating window level |
| `skip_taskbar(true)` | ignored (no protocol); use a window rule | `_NET_WM_STATE_SKIP_TASKBAR` | removed from the taskbar | ignored |
| `set_size` | applied | applied | applied | applied |
| `show` / `hide` | applied (hide unmaps the toplevel) | applied | applied | applied |
| `focus` | the compositor may refuse | WM focus-stealing policy applies | foreground rules apply | applied |

On Linux the app id is used twice before GTK initializes: as the GTK
application id, which GTK registers on the D-Bus session bus, and as GLib's
process-wide program name, which is what GTK 3 actually sends as the Wayland
app id and uses for the X11 `WM_CLASS`. Run one process per app id. The host
owns the process's GTK event loop, so renaming the program affects only it.

The `widget` smoke scenario opens an undecorated, transparent, keep-above,
skip-taskbar window under an app id. The loopback server requires the no-IPC
report from the page, and `verify_window_options_for_test` reads back the GTK
application id, program name, decoration, skip-taskbar hint, RGBA visual and
app-paintable state, and the WebKit background alpha. The scenario then
rejects an out-of-range resize, resizes, hides, shows, focuses, closes, and
requires that no semantic or native state remains. Keep-above is required
only on hosts without an X11 or Wayland display: a bare Xvfb has no window
manager to grant it, and Wayland compositors ignore it. The stale-handle
checks in `timeout`, `cancel`, and `window-close` also require all four
operations to return `WindowClosed`.

Not in this slice: `set_position`, `initial_position`, a typed `Unsupported`
result for keep-above on Wayland, layer-shell anchoring, `navigate`, and real
Windows/macOS app-id and macOS window transparency.
