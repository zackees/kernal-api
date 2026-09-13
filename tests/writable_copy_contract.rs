#[cfg(unix)]
#[test]
fn adopting_permissions_adds_only_owner_write() {
    use std::os::unix::fs::PermissionsExt;
    for mode in [0o400, 0o500, 0o600, 0o755] {
        let file = tempfile::tempfile().unwrap();
        kernal_api::platform::fs::make_writable_like(&file, &std::fs::Permissions::from_mode(mode))
            .unwrap();
        assert_eq!(
            file.metadata().unwrap().permissions().mode() & 0o7777,
            mode | 0o200
        );
    }
}

#[cfg(windows)]
#[test]
fn adopting_permissions_clears_readonly_on_copy() {
    let file = tempfile::tempfile().unwrap();
    let mut source = file.metadata().unwrap().permissions();
    source.set_readonly(true);
    kernal_api::platform::fs::make_writable_like(&file, &source).unwrap();
    assert!(!file.metadata().unwrap().permissions().readonly());
    assert!(source.readonly(), "source permissions remain unchanged");
}
