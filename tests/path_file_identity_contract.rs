use kernal_api::platform::fs::path_file::{file_identity, same_file, FileIdentity};

#[test]
fn hardlinks_match_but_copies_do_not() {
    fn traits<T: Copy + Eq + std::hash::Hash + std::fmt::Debug>() {}
    traits::<FileIdentity>();
    let temp = tempfile::tempdir().unwrap();
    let a = temp.path().join("a");
    let b = temp.path().join("b");
    let c = temp.path().join("c");
    std::fs::write(&a, b"data").unwrap();
    std::fs::hard_link(&a, &b).unwrap();
    std::fs::copy(&a, &c).unwrap();
    assert_eq!(file_identity(&a).unwrap(), file_identity(&b).unwrap());
    assert_ne!(file_identity(&a).unwrap(), file_identity(&c).unwrap());
    assert!(same_file(&a, &b).unwrap());
    assert!(!same_file(&a, &c).unwrap());
}

#[test]
fn missing_identity_never_matches() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing");
    assert!(file_identity(&path).is_err());
    #[cfg(windows)]
    assert!(!same_file(&path, &path).unwrap());
    #[cfg(unix)]
    assert!(same_file(&path, &path).is_err());
}

#[test]
fn directory_identity_is_supported() {
    let temp = tempfile::tempdir().unwrap();
    assert!(same_file(temp.path(), temp.path()).unwrap());
}
