use kernal_api::platform::fs::{file_change_marker, FileChangeMarker};

#[test]
fn marker_is_opaque_comparable_and_optional() {
    fn traits<T: Copy + Eq + std::hash::Hash + std::fmt::Debug>() {}
    traits::<FileChangeMarker>();
    let _: fn(&std::path::Path) -> Option<FileChangeMarker> = file_change_marker;
    let temp = tempfile::tempdir().unwrap();
    assert_eq!(file_change_marker(&temp.path().join("missing")), None);
}

#[cfg(not(windows))]
#[test]
fn timestamps_are_not_substituted_for_a_journal() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("file");
    std::fs::write(&path, b"first").unwrap();
    assert_eq!(file_change_marker(&path), None);
    std::fs::write(&path, b"second").unwrap();
    assert_eq!(file_change_marker(&path), None);
}
