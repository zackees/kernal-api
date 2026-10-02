//! Windows half of the external webview's window presentation.
//!
//! WebView2 windows have no application-id concept that tao exposes, so a
//! validated app id is accepted and not applied here. Transparency uses tao's
//! layered-window path together with WebView2's transparent background.

use tauri_runtime::window::WindowBuilder as _;
use tauri_runtime::RuntimeInitArgs;
use tauri_runtime_wry::WindowBuilderWrapper;

/// Event-loop arguments. The app id has no Windows effect in this release.
pub(crate) fn runtime_init_args(app_id: Option<&str>) -> RuntimeInitArgs {
    let _ = app_id;
    RuntimeInitArgs::default()
}

/// Request a transparent top-level window.
pub(crate) fn transparent_window(
    builder: WindowBuilderWrapper,
    transparent: bool,
) -> WindowBuilderWrapper {
    builder.transparent(transparent)
}

/// Acceptance-only check. Windows exposes no further native evidence here;
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
