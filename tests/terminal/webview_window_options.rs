#![cfg(feature = "tauri-webview")]

use kernal_api::webview::{
    ExternalWebviewHost, WebviewError, WebviewWindowOptions, WindowOptionsError,
};

#[test]
fn window_options_preserve_product_title_and_logical_size() {
    let options = WebviewWindowOptions::new("FastLED — preview", 1280, 720).unwrap();
    assert_eq!(options.title(), "FastLED — preview");
    assert_eq!(options.logical_size(), (1280, 720));
    assert_eq!(options.clone(), options);
}

#[test]
fn window_options_reject_invalid_values_before_native_effects() {
    for (width, height) in [(0, 1), (1, 0), (16385, 1), (1, 16385), (u32::MAX, 1)] {
        assert_eq!(
            WebviewWindowOptions::new("title", width, height),
            Err(WindowOptionsError::InvalidSize)
        );
    }
    for title in ["nul\0title", "line\nbreak", "tab\ttitle"] {
        assert_eq!(
            WebviewWindowOptions::new(title, 1, 1),
            Err(WindowOptionsError::InvalidTitle)
        );
    }
    assert!(WebviewWindowOptions::new("", 1, 16384).is_ok());
    assert!(WebviewWindowOptions::new(&"é".repeat(512), 16384, 1).is_ok());
    assert_eq!(
        WebviewWindowOptions::new(&"é".repeat(513), 1, 1),
        Err(WindowOptionsError::InvalidTitle)
    );
}

#[test]
fn widget_presentation_defaults_to_an_ordinary_decorated_window() {
    let options = WebviewWindowOptions::new("widget", 320, 240).unwrap();
    assert!(options.has_decorations());
    assert!(!options.is_transparent());
    assert!(!options.is_always_on_top());
    assert!(!options.skips_taskbar());
}

#[test]
fn widget_presentation_builders_are_recorded_without_native_effects() {
    let options = WebviewWindowOptions::new("widget", 320, 240)
        .unwrap()
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true);
    assert!(!options.has_decorations());
    assert!(options.is_transparent());
    assert!(options.is_always_on_top());
    assert!(options.skips_taskbar());
    assert_eq!(options.title(), "widget");
    assert_eq!(options.logical_size(), (320, 240));
    let restored = options
        .clone()
        .decorations(true)
        .transparent(false)
        .always_on_top(false)
        .skip_taskbar(false);
    assert_eq!(
        restored,
        WebviewWindowOptions::new("widget", 320, 240).unwrap()
    );
    assert_ne!(restored, options);
}

#[test]
fn invalid_app_ids_are_rejected_before_any_native_host_exists() {
    // Validation precedes event-loop construction, so these calls are safe
    // on a test-worker thread with no display: a valid id would initialize
    // the native host here and is exercised by the main-thread smoke instead.
    let runtime = kernal_api::async_engine::RuntimeBuilder::current_thread()
        .build()
        .unwrap();
    let too_long = format!("dev.{}", "a".repeat(252));
    for app_id in [
        "",
        "nodots",
        ".leading.dot",
        "trailing.dot.",
        "double..dot",
        "dev.1digit",
        "1dev.example",
        "dev.example/widget",
        "dev.example widget",
        "dev.exämple",
        "dev.example\0",
        too_long.as_str(),
    ] {
        assert_eq!(
            ExternalWebviewHost::with_app_id(runtime.handle(), app_id).err(),
            Some(WebviewError::InvalidWindowOptions(
                WindowOptionsError::InvalidAppId
            )),
            "{app_id:?} must be rejected before native effects"
        );
    }
}
