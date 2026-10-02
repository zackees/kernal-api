//! macOS half of the external webview's window presentation.
//!
//! A macOS application id is the bundle identifier, fixed by `Info.plist`, so
//! a validated app id is accepted and not applied here. A transparent NSWindow
//! needs Tauri's `macos-private-api` feature, which this crate does not
//! enable; only the WKWebView background is made transparent.

use tauri_runtime::RuntimeInitArgs;
use tauri_runtime_wry::WindowBuilderWrapper;

/// Event-loop arguments. The app id has no macOS effect in this release.
pub(crate) fn runtime_init_args(app_id: Option<&str>) -> RuntimeInitArgs {
    let _ = app_id;
    RuntimeInitArgs::default()
}

/// Window-level transparency is unavailable without the private API.
pub(crate) fn transparent_window(
    builder: WindowBuilderWrapper,
    transparent: bool,
) -> WindowBuilderWrapper {
    let _ = transparent;
    builder
}

/// Acceptance-only check. macOS exposes no further native evidence here;
/// the neutral facade has already compared the portable getters.
#[cfg(feature = "tauri-webview-test-support")]
pub(crate) fn verify_presentation(
    _window: &tauri_runtime_wry::WryWindowDispatcher<()>,
    _webview: Option<&wry::WebView>,
    expected: &crate::tauri::ExpectedPresentation<'_>,
) -> Result<(), String> {
    // No host getter reports these here; the request is still recorded.
    let crate::tauri::ExpectedPresentation {
        app_id,
        decorations,
        transparent,
        skip_taskbar,
    } = expected;
    let _ = (app_id, decorations, transparent, skip_taskbar);
    Ok(())
}
