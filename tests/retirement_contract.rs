#![cfg(unix)]
use kernal_api::platform::fs::{open_for_retire, retire_open_file};

#[test]
fn unix_retirement_runs_caller_remove_once_and_preserves_its_error() {
    let file = tempfile::tempfile().unwrap();
    let file = open_for_retire(file).unwrap();
    let mut calls = 0;
    let error = retire_open_file(file, || {
        calls += 1;
        Err(std::io::Error::from_raw_os_error(13))
    })
    .unwrap_err();
    assert_eq!(calls, 1);
    assert_eq!(error.raw_os_error(), Some(13));
}

#[test]
fn unix_retirement_removes_only_the_callers_selected_link() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let link = temp.path().join("retire");
    std::fs::write(&original, b"keep").unwrap();
    std::fs::hard_link(&original, &link).unwrap();
    let file = open_for_retire(std::fs::File::open(&link).unwrap()).unwrap();
    retire_open_file(file, || std::fs::remove_file(&link)).unwrap();
    assert!(!link.exists());
    assert_eq!(std::fs::read(original).unwrap(), b"keep");
}
