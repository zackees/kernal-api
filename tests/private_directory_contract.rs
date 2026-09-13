use kernal_api::platform::fs::{create_dir_all_private, ensure_dir_private};

#[test]
fn missing_directory_is_not_silently_accepted() {
    let temp = tempfile::tempdir().unwrap();
    assert!(ensure_dir_private(&temp.path().join("missing")).is_err());
}

#[test]
fn recursive_creation_and_verification_are_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("a/b");
    create_dir_all_private(&path).unwrap();
    assert!(path.is_dir());
    assert!(!ensure_dir_private(&path).unwrap());
    create_dir_all_private(&path).unwrap();
    assert!(!ensure_dir_private(&path).unwrap());
}

#[cfg(unix)]
#[test]
fn unix_shared_sticky_and_readable_modes_are_preserved() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    for mode in [0o1777, 0o755, 0o700] {
        std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(mode)).unwrap();
        assert!(!ensure_dir_private(temp.path()).unwrap());
        assert_eq!(
            std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o7777,
            mode
        );
    }
}

#[cfg(unix)]
#[test]
fn unix_writable_directory_is_tightened_and_parents_are_born_restricted() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(temp.path(), std::fs::Permissions::from_mode(0o777)).unwrap();
    assert!(ensure_dir_private(temp.path()).unwrap());
    assert_eq!(
        std::fs::metadata(temp.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let parent = temp.path().join("a");
    let child = parent.join("b");
    create_dir_all_private(&child).unwrap();
    for path in [&parent, &child] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o077,
            0
        );
    }
}
