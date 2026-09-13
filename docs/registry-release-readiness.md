# Registry release readiness

The first usable version remains unpublished: on 2026-09-13 the crates.io
sparse index contained only yanked `0.0.0`, and PyPI reported `0.0.0`.

At HTTP-server merge `033468a`, these local checks passed:

- Release process-substrate guard.
- `soldr cargo package --locked`, including default-feature verification.
- Extracted-package check with FastLED's current features:
  `fs,fs-watch,hash-sha256,archive,http-client,http-server,event-stream`.

However, checking the extracted package with `tauri-webview` failed with 13
Wry compilation errors, including missing `InnerWebView` and `webkit2gtk`.
The registry graph selected Wry 0.55.1 through Tauri runtime and the direct
Wry 0.57.0 dependency. The Git-patched checkout's green CI does not verify this
graph. In addition, registry Wry 0.57.0 unconditionally injects the Linux IPC
script; the checkout's Wry patch makes that injection conditional on a native
handler. Compilation alone is not proof of external-page isolation.

The follow-up implementation removes all Git patches and explicitly enables
`os-webview` on direct Wry 0.57.0. Published Tauri runtime retains private Wry
0.55.1 for its integration; only direct Wry creates the isolated external views.
An initial attempt to unify on 0.55.1 was rejected in review because it injects
IPC unconditionally on Windows and macOS. Wry 0.57.0 makes those injections
conditional on a configured handler. Two private versions remain; no graph
reduction or measured build-speed gain is claimed for this release fix.
The existing native Linux isolation smoke first failed on this registry graph:
the page reported `ipc=1&tauri=0&platform=1`. Construction now precedes
navigation, and the Linux adapter removes initialization scripts and unregisters
the IPC endpoint before loading the external URL. The same native smoke then
passed with all three bridge probes absent on the initial 0.55.1 Linux draft.
The corrected 0.57.0 graph subsequently built successfully and passed all six
native Linux scenarios: isolation/close, prohibited redirect, timeout,
cancellation, window close and user-clicked popup denial. A native Windows CI
step now runs the isolation proof after a separate build; its probes cover
`window.ipc`, Tauri internals and the WebKit IPC endpoint, not every
platform-provided messaging object. Cross-platform execution and final
extracted-package validation remain required; these local results are not a
release.

The release workflow now verifies the extracted package with all features,
and installs the same Linux webview development prerequisites used by native
CI. Its focused configuration regression was RED before the workflow change
and GREEN afterward. This gate detects the problem; it does not fix the
underlying registry viewer graph. Resolve the patched dependencies and rerun
package verification plus native isolation checks before publishing.

Publication credentials also need configuration. Repository Actions secrets
and environments were empty when inspected; no token was present in the
current environment or default Cargo credentials file. The existing workflow
expects `CARGO_REGISTRY_TOKEN` and `PYPI_API_TOKEN`. Never put tokens in source,
issue comments, logs, or chat.
