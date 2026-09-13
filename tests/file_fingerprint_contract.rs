#![cfg(feature = "file-fingerprint")]

use kernal_api::platform::fs::{file_fingerprint, same_file_by_fingerprint};

#[test]
fn frozen_json_schema_retains_optional_host_fields_and_wide_timestamps() {
    use kernal_api::platform::fs::FileFingerprint;
    for json in [
        r#"{"len":3,"modified_ns":18446744073709551616}"#,
        r#"{"len":3,"modified_ns":4,"dev":5,"ino":6,"ctime":-7,"ctime_nsec":8}"#,
        r#"{"len":3,"modified_ns":4,"volume_serial_number":5,"file_index":6,"creation_time":7}"#,
    ] {
        let fingerprint: FileFingerprint = serde_json::from_str(json).unwrap();
        assert_eq!(serde_json::to_string(&fingerprint).unwrap(), json);
    }
}

#[test]
fn fingerprint_retains_metadata_and_missing_paths_never_compare_equal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    std::fs::write(&path, b"abc").unwrap();
    let first = file_fingerprint(&path).unwrap();
    assert_eq!(first.len, 3);
    assert!(same_file_by_fingerprint(&path, &path));
    let missing = temp.path().join("missing");
    assert!(file_fingerprint(&missing).is_none());
    assert!(!same_file_by_fingerprint(&missing, &missing));
    std::fs::write(&path, b"longer").unwrap();
    assert_ne!(first, file_fingerprint(&path).unwrap());
}

#[test]
fn hardlink_paths_share_native_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    let link = temp.path().join("link");
    std::fs::write(&path, b"abc").unwrap();
    std::fs::hard_link(&path, &link).unwrap();
    assert!(same_file_by_fingerprint(&path, &link));
}
