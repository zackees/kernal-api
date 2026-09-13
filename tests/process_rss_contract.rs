#[test]
fn current_process_native_resident_memory_is_available() {
    let bytes = kernal_api::platform::resources::process_rss_bytes(std::process::id())
        .expect("native self-memory observation");
    assert!(bytes > 0);
}
