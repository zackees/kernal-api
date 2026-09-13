use kernal_api::platform::host::{
    process_target, target_is_linux, target_is_macos, target_is_windows, target_uses_musl,
};

const WINDOWS: bool = target_is_windows();
const MACOS: bool = target_is_macos();
const LINUX: bool = target_is_linux();
const MUSL: bool = target_uses_musl();

#[test]
fn constant_predicates_agree_with_canonical_target_names() {
    let target = process_target();
    assert_eq!(WINDOWS, target.os == "windows");
    assert_eq!(MACOS, target.os == "macos");
    assert_eq!(LINUX, target.os == "linux");
    assert_eq!(MUSL, cfg!(target_env = "musl"));
    assert_eq!(
        [WINDOWS, MACOS, LINUX].into_iter().filter(|v| *v).count(),
        1
    );
}
