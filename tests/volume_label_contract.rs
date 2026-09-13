#[cfg(unix)]
#[test]
fn unix_volume_label_retains_decimal_device_id_and_missing_path_behavior() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(
        kernal_api::platform::fs::volume_label(temp.path()),
        Some(std::fs::metadata(temp.path()).unwrap().dev().to_string())
    );
    assert_eq!(
        kernal_api::platform::fs::volume_label(&temp.path().join("missing")),
        None
    );
}

#[cfg(windows)]
#[test]
fn windows_unc_without_drive_letter_has_no_compatibility_label() {
    assert_eq!(
        kernal_api::platform::fs::volume_label(std::path::Path::new(
            r"\\?\UNC\absent-server\absent-share"
        )),
        None
    );
}
