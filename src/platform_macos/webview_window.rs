//! macOS half of the external webview's window presentation.
//!
//! A macOS application id is the bundle identifier, fixed by `Info.plist`
//! for a bundled app and absent for a bare executable; it cannot be changed
//! at run time, so a validated app id is accepted and not applied here.
//! Keep-above is tao's floating window level. Taskbar exclusion has no
//! per-window form: it switches the application to the accessory activation
//! policy (no Dock icon, no menu bar). A transparent NSWindow needs Tauri's
//! `macos-private-api` feature, which this crate does not enable; only the
//! WKWebView background is made transparent. tao applies size and position
//! through an asynchronous main-queue dispatch that the hosted macOS runner
//! never drained within seconds, so both are also applied directly here, on
//! the main thread, once the event loop has processed the request.

use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSView, NSWindow};
use objc2_core_graphics::{CGDisplayPixelsHigh, CGMainDisplayID};
use objc2_foundation::{NSPoint, NSSize};
use tauri_runtime::RuntimeInitArgs;
use tauri_runtime::WindowDispatch as _;
use tauri_runtime_wry::{WindowBuilderWrapper, WryWindowDispatcher};
use wry::raw_window_handle::RawWindowHandle;

/// Event-loop arguments. The app id has no macOS effect.
pub(crate) fn runtime_init_args(app_id: Option<&str>) -> Result<RuntimeInitArgs, String> {
    let _ = app_id;
    Ok(RuntimeInitArgs::default())
}

/// AppKit lets the application place and stack its windows.
pub(crate) fn display_places_windows() -> bool {
    false
}

/// Window-level transparency is unavailable without the private API.
pub(crate) fn transparent_window(
    builder: WindowBuilderWrapper,
    transparent: bool,
) -> WindowBuilderWrapper {
    let _ = transparent;
    builder
}

fn ns_window(window: &WryWindowDispatcher<()>) -> Result<Retained<NSWindow>, String> {
    let handle = window.window_handle().map_err(|error| error.to_string())?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return Err("unexpected native window handle".into());
    };
    // SAFETY: the AppKit handle's `ns_view` is this live window's content
    // view, and this runs on the main thread that owns it.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    view.window()
        .ok_or_else(|| "the content view has no window".to_owned())
}

/// Apply the logical client-area size now, on the main thread.
pub(crate) fn settle_size(
    window: &WryWindowDispatcher<()>,
    width: u32,
    height: u32,
) -> Result<(), String> {
    ns_window(window)?.setContentSize(NSSize::new(f64::from(width), f64::from(height)));
    Ok(())
}

/// Move the outer top-left corner now, on the main thread, using tao's
/// conversion from top-left logical coordinates to AppKit's bottom-left
/// screen space (the main display's height).
pub(crate) fn settle_position(
    window: &WryWindowDispatcher<()>,
    x: i32,
    y: i32,
) -> Result<(), String> {
    let top = CGDisplayPixelsHigh(CGMainDisplayID()) as f64;
    ns_window(window)?.setFrameTopLeftPoint(NSPoint::new(f64::from(x), top - f64::from(y)));
    Ok(())
}

/// Hide the application from the Dock and the app switcher. Runs on the
/// main thread (inside an event-loop closure); the policy is process-wide
/// and stays in effect until the process exits.
pub(crate) fn exclude_from_taskbar(window: &WryWindowDispatcher<()>) -> Result<(), String> {
    let _ = window;
    let main_thread = MainThreadMarker::new()
        .ok_or("the activation policy can only change on the main thread")?;
    let application = NSApplication::sharedApplication(main_thread);
    if application.activationPolicy() == NSApplicationActivationPolicy::Accessory
        || application.setActivationPolicy(NSApplicationActivationPolicy::Accessory)
    {
        Ok(())
    } else {
        Err("AppKit refused the accessory activation policy".into())
    }
}

/// Acceptance-only check, run on the main thread: a skip-taskbar window has
/// switched the application to the accessory activation policy. The neutral
/// facade has already compared the portable getters.
#[cfg(feature = "tauri-webview-test-support")]
pub(crate) fn verify_presentation(
    _window: &WryWindowDispatcher<()>,
    _webview: Option<&wry::WebView>,
    expected: &crate::tauri::ExpectedPresentation<'_>,
) -> Result<(), String> {
    // The app id, decorations and transparency have no further AppKit
    // evidence here; the request is still recorded.
    let crate::tauri::ExpectedPresentation {
        app_id,
        decorations,
        transparent,
        skip_taskbar,
    } = expected;
    let _ = (app_id, decorations, transparent);
    if *skip_taskbar {
        let main_thread =
            MainThreadMarker::new().ok_or("presentation check ran off the main thread")?;
        let policy = NSApplication::sharedApplication(main_thread).activationPolicy();
        if policy != NSApplicationActivationPolicy::Accessory {
            return Err(format!(
                "skip-taskbar window left the activation policy at {policy:?}"
            ));
        }
    }
    Ok(())
}
