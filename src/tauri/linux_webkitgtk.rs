//! Private Linux setup for the WebKitGTK-backed external-webview capability.
//! Process environment is configured by the launcher, never mutated here.

use std::cell::RefCell;

use gtk::gdk::prelude::MonitorExt as _;
use gtk::prelude::GtkSettingsExt as _;
use webkit2gtk::{
    PermissionRequestExt as _, SettingsExt as _, UserContentManagerExt as _, WebViewExt as _,
};

use super::WebviewPermissions;

#[path = "linux_webkitgtk/dpi.rs"]
mod dpi;

thread_local! {
    static DPI: RefCell<Option<(gtk::Settings, dpi::Correction)>> = const { RefCell::new(None) };
}

/// Published Wry installs an IPC script and endpoint even with no application
/// handler. This capability permits neither, nor any initialization scripts.
/// Called on the UI thread before the facade initiates the first navigation.
pub(super) fn remove_host_bridge(
    webview: &webkit2gtk::WebView,
) -> Result<(), super::NativeWebviewError> {
    let manager = webview.user_content_manager().ok_or_else(|| {
        super::NativeWebviewError::HostFailure("webview has no user content manager".into())
    })?;
    manager.remove_all_scripts();
    manager.unregister_script_message_handler("ipc");
    Ok(())
}

/// Correct page DPI on the GTK thread without repeatedly dividing our own
/// previous write. A different observed setting becomes the new desktop input.
/// An external reset exactly equal to our last write is indistinguishable.
pub(super) fn ensure_font_dpi() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let desktop_dpi = settings.gtk_xft_dpi();
    let integer_scale = gtk::gdk::Display::default()
        .and_then(|display| display.primary_monitor().or_else(|| display.monitor(0)))
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1);
    let effective_dpi = DPI.with(|state| {
        let mut state = state.borrow_mut();
        if state
            .as_ref()
            .is_some_and(|(previous, _)| previous != &settings)
        {
            *state = None;
        }
        let (_, correction) =
            state.get_or_insert_with(|| (settings.clone(), dpi::Correction::default()));
        correction.update(desktop_dpi, integer_scale)
    });
    // Release the state borrow before GTK can invoke property callbacks.
    if desktop_dpi != effective_dpi {
        settings.set_gtk_xft_dpi(effective_dpi);
    }
    if let Some(screen) = gtk::gdk::Screen::default() {
        screen.set_resolution(f64::from(effective_dpi) / 1024.0);
    }
}

/// Installs the opt-in user-media policy on a just-created raw WebKit view.
pub(super) fn configure_permissions(
    webview: &webkit2gtk::WebView,
    permissions: WebviewPermissions,
) {
    if !permissions.allow_user_media {
        return;
    }
    if let Some(settings) = webview.settings() {
        settings.set_enable_media_stream(true);
        settings.set_enable_webaudio(true);
    }
    webview.connect_permission_request(|_, request| {
        use gtk::glib::Cast as _;
        if request
            .downcast_ref::<webkit2gtk::UserMediaPermissionRequest>()
            .is_some()
        {
            request.allow();
            true
        } else {
            false
        }
    });
}
