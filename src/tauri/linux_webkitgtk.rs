//! Private Linux setup for the WebKitGTK-backed external-webview capability.
//!
//! WebKitGTK snapshots some renderer environment at process launch and reads
//! GTK's screen DPI while creating a view. Keeping those host facts and raw
//! bindings here prevents them from leaking through the semantic facade.

use std::cell::RefCell;
use std::ffi::{c_char, c_int, c_void, CStr};

use gtk::gdk::prelude::MonitorExt as _;
use gtk::glib::object::ObjectType as _;
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
        let settings: webkit2gtk::Settings = unsafe {
            gtk::glib::translate::from_glib_full(webkit2gtk::ffi::webkit_settings_new())
        };
        let api = resolve_feature_api().expect("WebKitGTK 2.42+ runtime feature API");
        assert_eq!(
            with_feature(&api, OFFSCREEN_CANVAS_WEBGL_FEATURE, |_| ()),
            Some(()),
            "WebKitGTK no longer lists {OFFSCREEN_CANVAS_WEBGL_FEATURE:?}"
        );
        assert!(enable_offscreen_canvas_webgl_setting(&settings));
    }
}
