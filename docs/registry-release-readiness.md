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

## Autonomous releases

`.github/workflows/auto-release.yml` follows the Soldr/zccache version-bump
pattern. A push to `main` that changes the Cargo package version starts the
existing package and six-target symbolizer verification pipeline. Unchanged
versions and existing GitHub releases do not automatically publish again.
Manual dispatch on `main` defaults to `dry_run: true`: verification and Actions
artifacts only, with no tag, GitHub release, or registry writes. Set it to false
to release the current version without another version bump. Existing tags must
resolve to the exact workflow commit; tags and release assets are never replaced.
GitHub release creation happens directly in the same workflow, not through a
second workflow triggered by a bot-created release event.

Registry publishing is deferred by owner request. GitHub packages, sidecars and
workers can release without registry credentials. To enable crates.io later,
configure `CARGO_REGISTRY_TOKEN` and set the repository Actions variable `PUBLISH_CRATES_IO`
to `true`. PyPI independently uses `PYPI_API_TOKEN` and `PUBLISH_PYPI=true`.
The publishing jobs use the `release` environment; secrets can be configured
there or at repository scope. Opt-in variables must be repository-level because
job conditions are evaluated before entering the environment. Neither variable
is set by this change.
Skipping an opted-out registry is not evidence that its package is published.
Manual non-dry-run dispatch can recover registry publication for an existing
GitHub release only from the same tagged commit and with identical rebuilt
GitHub assets. A mismatch fails rather than overwriting an immutable release.
After `main` advances, dispatch on the existing tag instead:
`gh workflow run auto-release.yml --ref v0.1.0 -f dry_run=false`.
Registry jobs wait for successful GitHub asset comparison before publishing.
Never put tokens in source, issue comments, logs, or chat.
