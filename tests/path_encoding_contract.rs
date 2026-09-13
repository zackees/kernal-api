use kernal_api::platform::fs::{native_call_path, path_from_raw_bytes};
use std::path::Path;

#[test]
fn utf8_paths_are_preserved() {
    assert_eq!(
        path_from_raw_bytes(b"dir/header.h"),
        Some("dir/header.h".into())
    );
}

#[cfg(unix)]
#[test]
fn unix_preserves_non_utf8_and_does_not_probe_paths() {
    use std::os::unix::ffi::OsStrExt;
    let bytes = &[0xff, b'/', b'a'];
    assert_eq!(
        path_from_raw_bytes(bytes).unwrap().as_os_str().as_bytes(),
        bytes
    );
    let path = Path::new("missing-parent/missing-file");
    assert_eq!(native_call_path(path).unwrap(), path);
}

#[cfg(windows)]
#[test]
fn windows_rejects_invalid_utf8_but_does_not_require_final_file() {
    assert_eq!(path_from_raw_bytes(&[0xff]), None);
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("not-created");
    let expected = std::fs::canonicalize(temp.path())
        .unwrap()
        .join("not-created");
    assert_eq!(native_call_path(&path).unwrap(), expected);
    assert!(!path.exists());
    assert!(native_call_path(&temp.path().join("absent/child")).is_err());
}
