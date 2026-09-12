# External webview isolation

The `tauri-webview` feature is a native desktop capability. It is excluded
from `default` and `full`, and its dependencies are target-scoped so a Wasm
guest target cannot resolve a platform webview runtime.

The private backend deliberately constructs `tauri_runtime::PendingWindow` and
`PendingWebview` on `tauri_runtime_wry::Wry`. It does not use Tauri's
`WebviewWindowBuilder`: that high-level application path installs IPC support
and initialization scripts, which would create a host bridge for untrusted
external content. The raw pending webview starts with an empty IPC handler,
no custom URI scheme handlers, and no initialization scripts. It also uses an
incognito data store, disables clipboard, extensions, autofill, devtools, and
link preview, refuses downloads, and denies every popup/new-window request.

Only absolute HTTP(S) URLs with a host are accepted. This includes loopback
HTTP for native tests. `file:`, `data:`, custom schemes, credentials, malformed
URLs, and prohibited redirects are rejected before a window is created. A
`PageLoadEvent::Finished` only resolves the completion for its matching
requested top-level URL. A user close, failed close, popup, or rejected
navigation completes the owned callback with a typed private reason.

Wry owns the platform event loop and must be initialized and driven on its UI
thread. Creation uses Wry's supported handle routing from the facade-owned
async runtime's blocking lane; it never blocks the event loop and avoids the
Windows callback-creation deadlock. The backend does not create a Tokio runtime
or another OS runtime. Issue #16's generated operation table will own the
generation-safe semantic resource handle, timeout, cancellation, and public
error mapping, then attach its operation completion to this backend callback.
