//! Private Linux setup for the WebKitGTK-backed external-webview capability.
//!
//! WebKitGTK snapshots some renderer environment at process launch and reads
//! GTK's screen DPI while creating a view. Keeping those host facts and raw
//! bindings here prevents them from leaking through the semantic facade.

use std::collections::BTreeMap;
use std::ffi::{c_char, c_int, c_void, CStr};
use std::path::Path;

use gtk::gdk::prelude::MonitorExt as _;
use gtk::glib::object::ObjectType as _;
use gtk::prelude::GtkSettingsExt as _;
use webkit2gtk::{
    PermissionRequestExt as _, SettingsExt as _, UserContentManagerExt as _, WebViewExt as _,
};

use super::WebviewPermissions;

const FALLBACK_FONT_DPI: i32 = 96;

/// Published Wry installs an IPC script and endpoint even with no application
/// handler. Remove both before installing any explicitly opted-in caller script.
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

/// Called after backend bridge removal and before the first navigation.
pub(super) fn install_page_bootstrap(
    webview: &webkit2gtk::WebView,
    source: &str,
) -> Result<(), super::NativeWebviewError> {
    let manager = webview.user_content_manager().ok_or_else(|| {
        super::NativeWebviewError::HostFailure("webview has no user content manager".into())
    })?;
    let script = webkit2gtk::UserScript::new(
        source,
        webkit2gtk::UserContentInjectedFrames::TopFrame,
        webkit2gtk::UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    manager.add_script(&script);
    Ok(())
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

/// WebKitGTK runtime feature gating WebGL on every OffscreenCanvas, including
/// canvases a document creates, not only canvases inside workers.
const OFFSCREEN_CANVAS_WEBGL_FEATURE: &CStr = c"AllowWebGLInWorkers";

#[repr(C)]
struct WebKitFeature {
    _private: [u8; 0],
}

#[repr(C)]
struct WebKitFeatureList {
    _private: [u8; 0],
}

type GetAllFeatures = unsafe extern "C" fn() -> *mut WebKitFeatureList;
type FeatureListLength = unsafe extern "C" fn(*mut WebKitFeatureList) -> usize;
type FeatureListGet = unsafe extern "C" fn(*mut WebKitFeatureList, usize) -> *mut WebKitFeature;
type FeatureListUnref = unsafe extern "C" fn(*mut WebKitFeatureList);
type FeatureIdentifier = unsafe extern "C" fn(*mut WebKitFeature) -> *const c_char;
type SetFeatureEnabled = unsafe extern "C" fn(*mut c_void, *mut WebKitFeature, c_int);
type GetFeatureEnabled = unsafe extern "C" fn(*mut c_void, *mut WebKitFeature) -> c_int;

/// The runtime-feature API from `WebKitFeature.h` and `WebKitSettings.h`.
struct FeatureApi {
    get_all: GetAllFeatures,
    length: FeatureListLength,
    get: FeatureListGet,
    unref: FeatureListUnref,
    identifier: FeatureIdentifier,
    set_enabled: SetFeatureEnabled,
    is_enabled: GetFeatureEnabled,
}

/// Resolves the WebKitGTK 2.42+ feature API from the already-loaded library.
/// The published bindings stop at 2.40, and linking these symbols directly
/// would stop older WebKitGTK hosts from loading at all.
fn resolve_feature_api() -> Option<FeatureApi> {
    fn symbol(name: &CStr) -> Option<*mut c_void> {
        // SAFETY: `name` is NUL-terminated, and RTLD_DEFAULT searches only
        // objects already loaded into this process.
        let address = unsafe { libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr()) };
        (!address.is_null()).then_some(address)
    }
    // SAFETY: each transmute reinterprets a verified non-null WebKitGTK symbol
    // as its documented C signature; `gsize` is `usize` and `gboolean` is
    // `c_int` on every supported Linux target.
    unsafe {
        Some(FeatureApi {
            get_all: std::mem::transmute::<*mut c_void, GetAllFeatures>(symbol(
                c"webkit_settings_get_all_features",
            )?),
            length: std::mem::transmute::<*mut c_void, FeatureListLength>(symbol(
                c"webkit_feature_list_get_length",
            )?),
            get: std::mem::transmute::<*mut c_void, FeatureListGet>(symbol(
                c"webkit_feature_list_get",
            )?),
            unref: std::mem::transmute::<*mut c_void, FeatureListUnref>(symbol(
                c"webkit_feature_list_unref",
            )?),
            identifier: std::mem::transmute::<*mut c_void, FeatureIdentifier>(symbol(
                c"webkit_feature_get_identifier",
            )?),
            set_enabled: std::mem::transmute::<*mut c_void, SetFeatureEnabled>(symbol(
                c"webkit_settings_set_feature_enabled",
            )?),
            is_enabled: std::mem::transmute::<*mut c_void, GetFeatureEnabled>(symbol(
                c"webkit_settings_get_feature_enabled",
            )?),
        })
    }
}

/// Calls `visit` with the listed feature named `identifier`, if any.
fn with_feature<T>(
    api: &FeatureApi,
    identifier: &CStr,
    visit: impl FnOnce(*mut WebKitFeature) -> T,
) -> Option<T> {
    // SAFETY: the list is returned with full ownership and released exactly
    // once; each feature is borrowed from that live list, and identifiers are
    // NUL-terminated strings owned by WebKit for the feature's lifetime.
    unsafe {
        let list = (api.get_all)();
        if list.is_null() {
            return None;
        }
        let mut visit = Some(visit);
        let mut result = None;
        for index in 0..(api.length)(list) {
            let feature = (api.get)(list, index);
            if feature.is_null() {
                continue;
            }
            let name = (api.identifier)(feature);
            if !name.is_null() && CStr::from_ptr(name) == identifier {
                result = visit.take().map(|visit| visit(feature));
                break;
            }
        }
        (api.unref)(list);
        result
    }
}

/// Enables WebGL on OffscreenCanvas for a just-created raw WebKit view.
///
/// WebKitGTK ships `AllowWebGLInWorkers` disabled, and that feature gates WebGL
/// contexts on every OffscreenCanvas, so rendering stacks that draw from an
/// OffscreenCanvas get no context at all. Chromium, Firefox and Safari 17+
/// enable these contexts by default. Hosts whose WebKitGTK predates the feature
/// API keep the engine default. Returns whether the feature is now enabled.
pub(super) fn enable_offscreen_canvas_webgl(webview: &webkit2gtk::WebView) -> bool {
    webview
        .settings()
        .is_some_and(|settings| enable_offscreen_canvas_webgl_setting(&settings))
}

/// Applies the feature to one settings object, which needs no display.
fn enable_offscreen_canvas_webgl_setting(settings: &webkit2gtk::Settings) -> bool {
    let Some(api) = resolve_feature_api() else {
        return false;
    };
    let raw_settings = settings.as_ptr().cast::<c_void>();
    with_feature(&api, OFFSCREEN_CANVAS_WEBGL_FEATURE, |feature| {
        // SAFETY: `raw_settings` is the live WebKitSettings kept alive by
        // `settings`, and `feature` is borrowed from the live feature list.
        unsafe {
            (api.set_enabled)(raw_settings, feature, 1);
            (api.is_enabled)(raw_settings, feature) != 0
        }
    })
    .unwrap_or(false)
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

    #[test]
    fn webkit_settings_enable_offscreen_canvas_webgl() {
        // `Settings::new` asserts GTK initialization, which needs a display, but
        // a WebKitSettings object itself does not.
        // SAFETY: `webkit_settings_new` returns a new owned reference, which
        // `from_glib_full` adopts exactly once.
        let settings: webkit2gtk::Settings =
            unsafe { gtk::glib::translate::from_glib_full(webkit2gtk::ffi::webkit_settings_new()) };
        let api = resolve_feature_api().expect("WebKitGTK 2.42+ runtime feature API");
        assert_eq!(
            with_feature(&api, OFFSCREEN_CANVAS_WEBGL_FEATURE, |_| ()),
            Some(()),
            "WebKitGTK no longer lists {OFFSCREEN_CANVAS_WEBGL_FEATURE:?}"
        );
        assert!(enable_offscreen_canvas_webgl_setting(&settings));
    }

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
