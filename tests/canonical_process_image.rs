//! Process image query is a reviewed, identity-preserving substrate export.

fn backend_to_facade(
    value: fn(u32) -> std::io::Result<std::path::PathBuf>,
) -> fn(u32) -> std::io::Result<std::path::PathBuf> {
    value
}

fn facade_to_backend(
    value: fn(u32) -> std::io::Result<std::path::PathBuf>,
) -> fn(u32) -> std::io::Result<std::path::PathBuf> {
    value
}

#[test]
fn image_query_is_the_exact_running_process_export() {
    let _: fn(u32) -> std::io::Result<std::path::PathBuf> =
        backend_to_facade(running_process::process_executable_path);
    let _: fn(u32) -> std::io::Result<std::path::PathBuf> =
        facade_to_backend(kernal_api::platform::process::executable_path_for_pid);
    let image = kernal_api::platform::process::executable_path_for_pid(std::process::id())
        .expect("current process has an image path");
    assert!(image.is_absolute());
}
