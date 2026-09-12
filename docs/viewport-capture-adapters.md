# Native viewport capture ownership (#19)

Status: the Linux and Windows callback adapters are implemented, but are not yet
wired to the webview operation service. The macOS adapter and generated
snapshot operation remain unimplemented. This note does not establish native
capture acceptance or close #19.
The adapter lives under `src/platform_linux/viewport_capture.rs`, selected by
the existing root platform selector and gated only by `tauri-webview` in the
concrete Linux tree. The broader cfg-boundary/Dylint/docs migration is tracked
in [#152](https://github.com/zackees/kernal-api/issues/152).

The shared hub now has a private `NativeBlobEncoder` implementing `io::Write`.
It copies encoder writes in configured chunks directly into quota-accounted
blob storage, without an intermediate full encoded-image vector. Its resource
stays reserved and unreadable until successful finish, which publishes a
sealed read-only blob. Encoded-byte overflow is rejected before that write is
copied, failures remain terminal, and drop/teardown reclaim the reservation.
Tests cover hidden partial output, read-only publication, cross-store rejection,
size overflow, and teardown. This sink does not
bound native images or encoder-internal scratch allocations. The private Linux adapter now calls
WebKitGTK's visible-region snapshot API, validates scaled requested dimensions
and returned image dimensions, and encodes the Cairo image into this sink.
It accepts native cancellation and returns only an opaque token to its private
completion callback. Successful publication and operation completion now share
one hub lock: cancellation or view closure that wins first prevents publication
and releases the reserved blob. A regression covers all three terminal outcomes
and verifies zero retained storage after teardown. The callback uses this
operation-bound publication path, but service dispatch, native cancellation
handle ownership in the service, and an end-to-end generated sketch proof are
still required.
Capture has a distinct hub permission and submission method; a load or close
operation cannot publish a snapshot. The checked-in load-as-capture regression
failed before this separation and passes with it. Additional tests reject
unactivated, foreign-store, and revoked views, and a foreign encoder cannot
complete another store's capture.
The Linux adapter attaches its reserved blob to the capture operation before
requesting the native snapshot. Cancellation and view closure therefore reclaim
partial hub-owned encoding immediately, even while the callback still holds its
encoder. A RED-to-GREEN regression checks both revocation paths before dropping
the encoder, including buffered-byte accounting and rejection of later writes.
Native browser image ownership still lasts until the callback releases it.
The same sink now supports bounded seeking and overwriting for the Windows
`IStream` adapter: seeks allocate nothing, overwrites reuse reserved storage,
and holes are filled through chunked quota admission only when written.
Tests cover header rewrites, zero-filled holes, invalid seeks, and cleanup.
The private Windows adapter uses this storage through its COM `IStream`.

Linux validation: `soldr cargo check --features tauri-webview --lib`, the
five `viewport_capture::tests` unit tests, and strict Clippy for the native
feature's library/tests passed inside the repo's Nix GTK/WebKitGTK shell.
The tests decode an encoded Cairo image and verify pixel-limit/cancellation
cleanup. The encoded-limit regression uses Cairo's real PNG writer with an
eight-byte admission limit and verifies a typed blob-limit error and zero
remaining storage. A late-image regression closes the hub before encoding and
verifies cancellation without publication.

The separately ignored `live_webkit_viewport_png_completes_capture_operation`
test requests an actual visible-region WebKitGTK snapshot under Xvfb. Its
in-memory offline HTML fixture has red and blue halves; the test decodes the
quota-accounted PNG, checks physical viewport dimensions and interior colors,
and verifies the hub's completed operation references exactly that blob. It
also cancels a second native snapshot and waits for its callback before
asserting no blob publication or retained storage. This is an adapter-level
proof, not the external-content service or generated Wasm screenshot sketch.
Windows native execution and the macOS adapter remain outstanding.

```sh
nix-shell -p pkg-config gtk3 webkitgtk_4_1 xorg-server xauth xvfb-run --run 'LD_LIBRARY_PATH=$(printf "%s" "$NIX_LDFLAGS" | tr " " "\n" | sed -n "s/^-L//p" | paste -sd:); export LD_LIBRARY_PATH; GDK_BACKEND=x11 xvfb-run -a soldr cargo test --locked --features tauri-webview --lib live_webkit_viewport_png_completes_capture_operation -j 1 -- --ignored --test-threads=1'
```

## Exact dependency boundary

The current manifest pins Tauri 2.11.5, tauri-runtime 2.11.3,
tauri-runtime-wry 2.11.4, tauri-utils 2.9.3, and Wry 0.57.0. Tauri packages
also resolve through the existing immutable Git patch at
`4039372c9f75fc3e1c9f0fc98858c7c100880f8f`; that patch must be included when
reporting the implementation version, not just the registry version numbers.
Wry also uses the existing immutable Git patch
`29a2228b594d45c0ae9710866e46a6f898120151`. The Linux adapter's direct bindings
are optional exact pins: webkit2gtk 2.0.2, gtk 0.18.2, gio 0.18.4, and
cairo-rs 0.18.5 with its PNG writer enabled. No new package versions were
introduced by these direct edges.

The lockfile resolves Windows Wry bindings to `webview2-com` 0.39.1 and
`windows`/`windows-core` 0.62.2. Any direct COM adapter edges must match those
versions. Microsoft's [CapturePreview contract](https://learn.microsoft.com/en-us/microsoft-edge/webview2/reference/win32/icorewebview2#capturepreview)
writes image data to an `IStream` and signals completion afterward. The adapter
implements its stream over the shared quota-accounted sink, not an
unbounded memory stream followed by a size check. Its optional exact-pinned
direct dependencies match those lockfile versions; the lockfile adds only
three direct dependency edges.

## Windows adapter status

`src/platform_win/viewport_capture.rs` is selected within the existing Windows
tree and calls WebView2 `CapturePreview` with PNG format. It checks controller
bounds before submission and PNG IHDR dimensions before publication, retains
only a fixed 24-byte format header outside the shared sink, and uses the same
operation-bound publication as Linux. Revocation prevents further encoded
writes; the eventual callback drops the COM-owned encoder. There is no native
CapturePreview cancellation handle.

The stream implements write, bounded seek, stat, and commit. Unsupported read,
resize, clone, and transaction operations return `E_NOTIMPL`. Compatibility
with the actual WebView2 PNG writer must still be demonstrated on Windows;
successful cross-compilation alone does not prove this stream contract is
sufficient. No memory-stream fallback is allowed if native testing exposes
another required stream method.

Windows x86-64 library compilation and strict Clippy for library/tests passed.
Windows ARM64 library/test compilation also passed in an isolated target
directory with `RUSTFLAGS='-C debuginfo=1'`. The initial shared-target attempt
failed in `windows-strings`; an isolated attempt was terminated by SIGTERM
inside the `windows` dependency, and its completed retry passed. These earlier
failures are not evidence of native Windows runtime behavior.
The COM write/seek/quota and PNG-header tests are checked in and type-checked,
but have not executed on Windows. A live Windows viewport PNG and cancellation
proof remain required; the Linux Xvfb proof does not cover this adapter.

Inspection of Wry 0.57.0's `src/lib.rs` found platform `webview()` escape
hatches returning WebView2, WebKitGTK, and retained Cocoa webviews, but no
portable snapshot/capture method. kernal-api therefore owns the three private
adapters required by #19: `CapturePreview` on Windows, `takeSnapshot` on macOS,
and `webkit_web_view_get_snapshot` on Linux. No desktop capture, injected
JavaScript, or automation fallback is permitted.

The [WebKitGTK 4.1 snapshot contract](https://webkitgtk.org/reference/webkit2gtk/stable/method.WebView.get_snapshot.html)
is asynchronous, accepts a cancellable and region, and requires completion
through `webkit_web_view_get_snapshot_finish`. The currently resolved Rust
webkit2gtk 2.0.2 binding exposes `WebViewExt::snapshot`, returning a Cairo
surface in its callback and requiring ownership of the GLib main context.
The resolved cairo-rs 0.18.5 PNG writer accepts an `io::Write` sink, allowing
encoded-byte admission to happen before each write instead of after an
unbounded PNG vector has already been created. Direct bindings added for
capture must be optional, exact-pinned, and compatible with this graph.

## Integration requirements

`src/tauri.rs` retains Wry webviews in the UI-thread `UI_WEBVIEWS` map. Native
capture must dispatch there and keep native image objects on their permitted
thread. A completion must enter the existing `OperationHub`; it must not
introduce another executor or permit concurrent Wasm Store entry.

Before requesting capture, validate the logical viewport, native device scale,
checked physical dimensions, and configured pixel budget. Validate the actual
returned dimensions again: a resize or scale change can race the request.
Pixel limits alone do not prove a bound on a native browser's internal memory.

PNG encoding must use a bounded sink backed by the existing blob quota and
transfer accounting. A full PNG `Vec<u8>` returned from a platform callback is
not the guest API, and checking its size only after allocation does not meet
the encoded-byte admission requirement. Successful completion publishes one
opaque blob in the same logical-sketch resource context. The existing native
webview client and Wasm entry point currently create separate service contexts;
the screenshot sketch must not bridge them by copying numeric handles.

Cancellation, close, and teardown must revoke publication authority immediately.
Callbacks that arrive afterward still release their native image and encoding
state, but cannot install a blob or complete an operation a second time. A
native callback retaining storage must retain its accounting until that storage
is actually released, even if the guest already consumed cancellation.

## Required evidence still missing

- A checked-in RED regression for the absent generated capture operation.
- Three native adapters, private bindings, and operation/resource integration.
- Decodable viewport PNGs at native scale without window chrome.
- Pixel/encoded-byte admission, stale and cross-instance rejection, cancellation,
  late callbacks, and teardown tests with truthful final resource counters.
- Native Linux WebKitGTK 4.1/Xvfb, macOS, and Windows execution, plus both
  architecture compile checks for each supported operating system.

The Linux callback and bounded encoder are the first implementation step;
their completion must consume the shared blob protocol from #17. This does not
reduce the requirement to deliver all three native adapters.
