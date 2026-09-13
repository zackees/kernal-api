#![cfg(windows)]

use kernal_api::platform::fs::{allocated_bytes, volume_identity};

#[test]
fn embedded_nul_never_queries_the_existing_prefix_file() {
    let temp = tempfile::tempdir().unwrap();
    let prefix = temp.path().join("prefix");
    std::fs::write(&prefix, b"payload").unwrap();
    let metadata = std::fs::metadata(&prefix).unwrap();
    let invalid = temp.path().join("prefix\0suffix");
    assert_eq!(
        volume_identity(&invalid).unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(allocated_bytes(&invalid, &metadata), metadata.len());
    assert_eq!(std::fs::read(&prefix).unwrap(), b"payload");
}
