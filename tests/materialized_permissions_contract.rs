use kernal_api::platform::fs;

#[test]
fn absent_archive_mode_does_not_probe_missing_path() {
    let temp = tempfile::tempdir().unwrap();
    fs::restore_mode(&temp.path().join("missing"), None).unwrap();
}

#[test]
fn host_metadata_mode_roundtrip_restores_original_permissions() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let original = fs::metadata_mode(&file.as_file().metadata().unwrap());
    fs::set_readonly(file.path(), true).unwrap();
    let result = fs::apply_metadata_mode(file.path(), original);
    // The tempfile started writable; restore even if the operation failed so
    // Windows cleanup is not obstructed by the test's readonly attribute.
    if result.is_err() {
        fs::set_readonly(file.path(), false).unwrap();
    }
    result.unwrap();
    assert_eq!(
        fs::metadata_mode(&file.as_file().metadata().unwrap()),
        original
    );
}

#[cfg(unix)]
#[test]
fn readonly_toggle_preserves_execute_and_nonwrite_bits() {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::restore_mode(file.path(), Some(0o775)).unwrap();
    fs::set_readonly(file.path(), true).unwrap();
    assert_eq!(fs::mode(file.path()).unwrap() & 0o7777, 0o555);
    fs::set_readonly(file.path(), false).unwrap();
    assert_eq!(fs::mode(file.path()).unwrap() & 0o7777, 0o755);
}

#[cfg(unix)]
#[test]
fn unix_modes_preserve_additive_and_fixed_policies() {
    use std::os::unix::fs::PermissionsExt;
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path();
    fs::restore_mode(path, Some(0o640)).unwrap();
    assert_eq!(fs::mode(path).unwrap() & 0o7777, 0o640);
    fs::make_executable(path).unwrap();
    assert_eq!(fs::mode(path).unwrap() & 0o7777, 0o751);
    fs::make_executable_from(path, &std::fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(fs::mode(path).unwrap() & 0o7777, 0o755);
    fs::make_private(path).unwrap();
    assert_eq!(fs::mode(path).unwrap() & 0o7777, 0o700);
}

#[cfg(windows)]
#[test]
fn windows_unix_mode_operations_are_noops_even_for_missing_paths() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("missing");
    fs::restore_mode(&path, Some(0o700)).unwrap();
    fs::make_private(&path).unwrap();
    fs::make_executable(&path).unwrap();
    assert_eq!(fs::mode(&path), None);
}

#[cfg(windows)]
#[test]
fn windows_host_mode_uses_readonly_flag_including_nonzero_restore_values() {
    let file = tempfile::NamedTempFile::new().unwrap();
    fs::apply_metadata_mode(file.path(), u32::MAX).unwrap();
    let observed = fs::metadata_mode(&file.as_file().metadata().unwrap());
    fs::apply_metadata_mode(file.path(), 0).unwrap();
    assert_eq!(observed, 1);
    assert_eq!(fs::metadata_mode(&file.as_file().metadata().unwrap()), 0);
    fs::set_readonly(file.path(), false).unwrap();
}

#[cfg(windows)]
#[test]
fn windows_executable_publication_preserves_source_readonly_attribute() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let original = file.as_file().metadata().unwrap().permissions();
    let mut source = original.clone();
    source.set_readonly(true);
    fs::make_executable_from(file.path(), &source).unwrap();
    let readonly = file.as_file().metadata().unwrap().permissions().readonly();
    file.as_file().set_permissions(original).unwrap();
    assert!(readonly);
}
