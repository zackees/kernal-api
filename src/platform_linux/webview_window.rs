//! Linux (GTK/tao) half of the external webview's window presentation.
//!
//! The neutral facade validates every option before this module runs. The
//! application id becomes both the GTK application id that tao creates its
//! event loop with and GLib's program name. GTK 3 publishes the program name,
//! not the application id, as the Wayland `xdg_toplevel` app id, and derives
//! the X11 `WM_CLASS` from it at initialization, so both must be set before
//! the event loop initializes GTK. A window is made transparent through tao's
//! RGBA-visual path.

use tauri_runtime::window::WindowBuilder as _;
use tauri_runtime::RuntimeInitArgs;
use tauri_runtime_wry::WindowBuilderWrapper;

/// Event-loop arguments carrying the validated application id, if any.
///
/// Call once, on the UI thread, immediately before the event loop is created.
/// With an id, this sets GLib's process-wide program name to it; the host owns
/// the process's only GTK event loop, so no other toolkit user is renamed.
pub(crate) fn runtime_init_args(app_id: Option<&str>) -> RuntimeInitArgs {
    if let Some(app_id) = app_id {
        gtk::glib::set_prgname(Some(app_id));
    }
    RuntimeInitArgs {
        app_id: app_id.map(str::to_owned),
    }
}

/// Request an RGBA visual and an app-paintable GTK window.
pub(crate) fn transparent_window(
    builder: WindowBuilderWrapper,
    transparent: bool,
) -> WindowBuilderWrapper {
    builder.transparent(transparent)
}

/// Acceptance-only check, run on the UI thread, that each requested option
/// reached the GTK window and WebKit view. Returns a description on mismatch.
#[cfg(feature = "tauri-webview-test-support")]
pub(crate) fn verify_presentation(
    window: &tauri_runtime_wry::WryWindowDispatcher<()>,
    webview: Option<&wry::WebView>,
    expected: &crate::tauri::ExpectedPresentation<'_>,
) -> Result<(), String> {
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use tauri_runtime::WindowDispatch as _;
    use webkit2gtk::WebViewExt as _;
    use wry::WebViewExtUnix as _;

    let gtk_window = window.gtk_window().map_err(|error| error.to_string())?;
    let app_id = gtk_window
        .application()
        .and_then(|application| gio::prelude::ApplicationExt::application_id(&application))
        .map(|id| id.to_string());
    if app_id.as_deref() != expected.app_id {
        return Err(format!(
            "GTK application id {app_id:?}, expected {:?}",
            expected.app_id
        ));
    }
    if let Some(expected_id) = expected.app_id {
        // GTK 3 sends this, not the application id, as the Wayland app id.
        let program = gtk::glib::prgname();
        if program.as_deref() != Some(expected_id) {
            return Err(format!(
                "GLib program name {program:?}, expected {expected_id:?}"
            ));
        }
    }
    if gtk_window.is_decorated() != expected.decorations {
        return Err(format!(
            "GTK decorated={}, expected {}",
            gtk_window.is_decorated(),
            expected.decorations
        ));
    }
    if gtk_window.skips_taskbar_hint() != expected.skip_taskbar {
        return Err(format!(
            "GTK skip-taskbar hint={}, expected {}",
            gtk_window.skips_taskbar_hint(),
            expected.skip_taskbar
        ));
    }
    if expected.transparent {
        if !gtk_window.is_app_paintable() {
            return Err("transparent GTK window is not app-paintable".into());
        }
        let rgba = gtk_window
            .visual()
            .is_some_and(|visual| visual.depth() == 32);
        let screen_has_rgba = GtkWindowExt::screen(&gtk_window)
            .and_then(|screen| screen.rgba_visual())
            .is_some();
        if screen_has_rgba && !rgba {
            return Err("transparent GTK window did not receive the RGBA visual".into());
        }
        let webview = webview.ok_or("transparent window has no WebKit view")?;
        let background = webview.webview().background_color();
        if background.alpha() != 0.0 {
            return Err(format!(
                "transparent WebKit view has background alpha {}",
                background.alpha()
            ));
        }
    }
    Ok(())
}
