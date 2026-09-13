//! Private Linux setup for the WebKitGTK-backed external-webview capability.
//!
//! WebKitGTK snapshots some renderer environment at process launch and reads
//! GTK's screen DPI while creating a view. Keeping those host facts and raw
//! bindings here prevents them from leaking through the semantic facade.

use std::collections::BTreeMap;
use std::path::Path;

use gtk::gdk::prelude::MonitorExt as _;
use gtk::prelude::GtkSettingsExt as _;
use webkit2gtk::{PermissionRequestExt as _, SettingsExt as _, WebViewExt as _};

use super::WebviewPermissions;

const FALLBACK_FONT_DPI: i32 = 96;

#[derive(Debug, Clone, PartialEq, Eq)]
struct HostFacts {
    nvidia: bool,
    x11_backend: bool,
    already_set: Vec<String>,
}

fn environment_overrides(facts: &HostFacts) -> BTreeMap<&'static str, &'static str> {
    let mut result = BTreeMap::new();
    let mut set = |key, value| {
        if !facts.already_set.iter().any(|present| present == key) {
            result.insert(key, value);
        }
    };
    set("JSC_useSharedArrayBuffer", "1");
    if facts.nvidia {
        set("__NV_DISABLE_EXPLICIT_SYNC", "1");
        if facts.x11_backend {
            set("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
        }
    }
    result
}

fn detect_host() -> HostFacts {
    let gdk_backend = std::env::var("GDK_BACKEND").unwrap_or_default();
    let x11_backend = if gdk_backend.is_empty() {
        std::env::var_os("WAYLAND_DISPLAY").is_none()
    } else {
        gdk_backend
            .split(',')
            .next()
            .is_some_and(|backend| backend.trim().eq_ignore_ascii_case("x11"))
    };
    let already_set = [
        "JSC_useSharedArrayBuffer",
        "__NV_DISABLE_EXPLICIT_SYNC",
        "WEBKIT_DISABLE_DMABUF_RENDERER",
    ]
    .into_iter()
    .filter(|key| std::env::var_os(key).is_some())
    .map(str::to_owned)
    .collect();
    HostFacts {
        nvidia: Path::new("/proc/driver/nvidia/version").exists()
            || Path::new("/sys/module/nvidia").exists(),
        x11_backend,
        already_set,
    }
}

/// Must run before Wry initializes WebKitGTK. Existing user values win.
pub(super) fn prepare_renderer_environment() {
    for (key, value) in environment_overrides(&detect_host()) {
        // SAFETY: this application-level setup runs before the facade creates
        // Wry/WebKitGTK or starts the UI event loop, so no facade thread can
        // concurrently read or modify process environment state.
        unsafe { std::env::set_var(key, value) };
    }
}

fn effective_font_dpi(desktop_gtk_xft_dpi: i32, integer_scale: i32) -> i32 {
    if desktop_gtk_xft_dpi <= 0 {
        return FALLBACK_FONT_DPI * 1024;
    }
    (desktop_gtk_xft_dpi / integer_scale.max(1)).max(1)
}

/// Correct GTK's effective page DPI after GTK initialization but before the
/// next WebKitGTK view. This avoids WebKit's negative zoom for unknown DPI and
/// preserves KDE/GNOME fractional desktop scale under GTK3 integer scaling.
pub(super) fn ensure_font_dpi() {
    let Some(settings) = gtk::Settings::default() else {
        return;
    };
    let desktop_dpi = settings.gtk_xft_dpi();
    let integer_scale = gtk::gdk::Display::default()
        .and_then(|display| display.primary_monitor().or_else(|| display.monitor(0)))
        .map(|monitor| monitor.scale_factor())
        .unwrap_or(1);
    let effective_dpi = effective_font_dpi(desktop_dpi, integer_scale);
    settings.set_gtk_xft_dpi(effective_dpi);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(nvidia: bool, x11_backend: bool, already_set: &[&str]) -> HostFacts {
        HostFacts {
            nvidia,
            x11_backend,
            already_set: already_set
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
        }
    }

    #[test]
    fn environment_decision_table_preserves_user_values() {
        assert_eq!(
            environment_overrides(&facts(false, false, &[])),
            BTreeMap::from([("JSC_useSharedArrayBuffer", "1")])
        );
        let x11_nvidia = environment_overrides(&facts(true, true, &[]));
        assert_eq!(x11_nvidia.get("__NV_DISABLE_EXPLICIT_SYNC"), Some(&"1"));
        assert_eq!(x11_nvidia.get("WEBKIT_DISABLE_DMABUF_RENDERER"), Some(&"1"));
        assert!(!environment_overrides(&facts(true, false, &[]))
            .contains_key("WEBKIT_DISABLE_DMABUF_RENDERER"));
        assert!(!environment_overrides(&facts(
            true,
            true,
            &["JSC_useSharedArrayBuffer", "WEBKIT_DISABLE_DMABUF_RENDERER"],
        ))
        .contains_key("JSC_useSharedArrayBuffer"));
    }

    #[test]
    fn effective_font_dpi_handles_fractional_and_unknown_desktops() {
        assert_eq!(effective_font_dpi(172_032, 2), 86_016);
        assert_eq!(effective_font_dpi(172_032, 1), 172_032);
        assert_eq!(effective_font_dpi(-1, 2), 98_304);
        assert_eq!(effective_font_dpi(0, 1), 98_304);
    }
}
