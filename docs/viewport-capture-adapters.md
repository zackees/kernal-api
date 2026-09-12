# Native viewport capture ownership (#19)

Status: implementation prerequisite review; the three adapters and snapshot
operation are not implemented. This note does not establish native capture
acceptance or close #19.

## Exact dependency boundary

The current manifest pins Tauri 2.11.5, tauri-runtime 2.11.3,
tauri-runtime-wry 2.11.4, tauri-utils 2.9.3, and Wry 0.57.0. Tauri packages
also resolve through the existing immutable Git patch at
`4039372c9f75fc3e1c9f0fc98858c7c100880f8f`; that patch must be included when
reporting the implementation version, not just the registry version numbers.

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
