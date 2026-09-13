#![cfg(windows)]
use kernal_api::platform::fs::{open_for_retire, retire_open_file};

#[test]
fn retiring_readonly_link_preserves_other_link_attributes_and_held_reader() {
    use std::io::Read;
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    let link = temp.path().join("retire");
    std::fs::write(&original, b"keep").unwrap();
    std::fs::hard_link(&original, &link).unwrap();
    let initial = std::fs::metadata(&original).unwrap().permissions();
    let mut readonly = initial.clone();
    readonly.set_readonly(true);
    std::fs::set_permissions(&original, readonly).unwrap();
    let mut reader = std::fs::File::open(&link).unwrap();
    let upgraded = open_for_retire(std::fs::File::open(&link).unwrap()).unwrap();
    let mut fallback_called = false;
    let result = retire_open_file(upgraded, || {
        fallback_called = true;
        panic!("Windows must never use path-based fallback");
    });
    let still_readonly = std::fs::metadata(&original)
        .unwrap()
        .permissions()
        .readonly();
    // Restore permissions before assertions so failed validation leaves no
    // readonly scratch artifacts behind. The observation above is unchanged.
    std::fs::set_permissions(&original, initial).unwrap();
    result.expect("native POSIX disposition must retire the selected link");
    assert!(!fallback_called);
    assert!(
        still_readonly,
        "retirement must not clear shared readonly attributes"
    );
    assert!(!link.exists());
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes).unwrap();
    assert_eq!(bytes, b"keep");
    assert_eq!(std::fs::read(original).unwrap(), b"keep");
}
