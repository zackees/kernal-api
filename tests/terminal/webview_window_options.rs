#![cfg(feature = "tauri-webview")]

use kernal_api::webview::{WebviewWindowOptions, WindowOptionsError};

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
