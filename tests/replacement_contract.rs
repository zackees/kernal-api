use kernal_api::platform::fs::replacement::*;

#[test]
fn atomic_replace_consumes_source_and_preserves_destination_on_missing_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&destination, b"old").unwrap();
    atomic_replace(&source, &destination).unwrap();
    assert!(!source.exists());
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert!(atomic_replace(&source, &destination).is_err());
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
}

#[test]
fn generation_rename_keeps_host_specific_collision_behavior() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&destination, b"old").unwrap();
    let result = rename_generation(&source, &destination);
    #[cfg(unix)]
    {
        result.unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"new");
        assert!(!source.exists());
    }
    #[cfg(windows)]
    {
        assert!(result.is_err());
        assert_eq!(std::fs::read(&destination).unwrap(), b"old");
        assert_eq!(std::fs::read(&source).unwrap(), b"new");
    }
}

#[test]
fn narrow_error_classifiers_do_not_inherit_generic_lock_policy() {
    for code in [5, 32, 33, 2] {
        let error = std::io::Error::from_raw_os_error(code);
        assert_eq!(
            is_transient_share_error(&error),
            cfg!(windows) && matches!(code, 5 | 32)
        );
        assert_eq!(is_lock_contention(&error), cfg!(windows) && code == 33);
    }
    for kind in [
        std::io::ErrorKind::WouldBlock,
        std::io::ErrorKind::PermissionDenied,
    ] {
        let error = std::io::Error::new(kind, "synthetic");
        assert!(!is_transient_share_error(&error));
        assert!(!is_lock_contention(&error));
    }
}

#[test]
fn directory_install_consumes_staged_tree() {
    let temp = tempfile::tempdir().unwrap();
    let staged = temp.path().join("staged");
    let requested = temp.path().join("requested");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("new"), b"new").unwrap();
    install_directory(&staged, &requested).unwrap();
    assert!(!staged.exists());
    assert_eq!(std::fs::read(requested.join("new")).unwrap(), b"new");
}

#[test]
fn directory_install_replaces_existing_contents() {
    let temp = tempfile::tempdir().unwrap();
    let staged = temp.path().join("staged");
    let requested = temp.path().join("requested");
    std::fs::create_dir(&staged).unwrap();
    std::fs::create_dir(&requested).unwrap();
    std::fs::write(staged.join("new"), b"new").unwrap();
    std::fs::write(requested.join("old"), b"old").unwrap();
    install_directory(&staged, &requested).unwrap();
    assert!(!staged.exists());
    assert!(!requested.join("old").exists());
    assert_eq!(std::fs::read(requested.join("new")).unwrap(), b"new");
}

#[test]
fn delete_fallback_exposes_its_legacy_failure_state() {
    let temp = tempfile::tempdir().unwrap();
    let missing = temp.path().join("missing");
    let destination = temp.path().join("destination");
    std::fs::write(&destination, b"old").unwrap();
    assert!(replace_with_delete_fallback(&missing, &destination).is_err());
    #[cfg(windows)]
    assert!(
        !destination.exists(),
        "legacy fallback deletes even for missing source"
    );
    #[cfg(unix)]
    assert_eq!(std::fs::read(&destination).unwrap(), b"old");
}

#[cfg(windows)]
#[test]
fn native_replacement_supports_long_paths_without_source_truncation() {
    let temp = tempfile::tempdir().unwrap();
    let mut parent = temp.path().to_path_buf();
    for index in 0..10 {
        parent.push(format!("segment-{index}-{}", "x".repeat(25)));
    }
    std::fs::create_dir_all(&parent).unwrap();
    let source = parent.join("source");
    let destination = parent.join("destination");
    std::fs::write(&source, b"new").unwrap();
    std::fs::write(&destination, b"old").unwrap();
    atomic_replace(&source, &destination).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    let renamed = parent.join("renamed");
    rename_generation(&destination, &renamed).unwrap();
    assert_eq!(std::fs::read(&renamed).unwrap(), b"new");
}

#[cfg(windows)]
#[test]
fn native_replacement_rejects_nul_without_mutating_prefix_paths() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    std::fs::write(&source, b"source").unwrap();
    std::fs::write(&destination, b"destination").unwrap();
    for operation in [atomic_replace, rename_generation] {
        assert_eq!(
            operation(&temp.path().join("source\0suffix"), &destination)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(
            operation(&source, &temp.path().join("destination\0suffix"))
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(std::fs::read(&source).unwrap(), b"source");
        assert_eq!(std::fs::read(&destination).unwrap(), b"destination");
    }
}
