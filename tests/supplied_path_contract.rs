use kernal_api::platform::executable::find_on_path_using;
use std::ffi::OsStr;
use std::path::PathBuf;

#[test]
fn explicit_paths_are_authoritative_without_existence_checks() {
    for name in ["./absent-tool", "bin\\absent-tool"] {
        assert_eq!(
            find_on_path_using(name, OsStr::new(""), &[]),
            Some(PathBuf::from(name))
        );
    }
}

#[test]
fn supplied_path_preserves_directory_bare_and_suffix_order_for_dotted_names() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    let path = std::env::join_paths([&first, &second]).unwrap();
    let extensions = [".cmd".to_string(), ".exe".to_string()];
    std::fs::write(first.join("tool.custom.cmd"), b"").unwrap();
    std::fs::write(first.join("tool.custom.exe"), b"").unwrap();
    std::fs::write(second.join("tool.custom"), b"").unwrap();
    assert_eq!(
        find_on_path_using("tool.custom", &path, &extensions),
        Some(first.join("tool.custom.cmd"))
    );
    std::fs::write(first.join("tool.custom"), b"").unwrap();
    assert_eq!(
        find_on_path_using("tool.custom", &path, &extensions),
        Some(first.join("tool.custom"))
    );
    assert_eq!(
        find_on_path_using("tool.custom", OsStr::new(""), &extensions),
        None
    );
}

#[test]
fn directories_are_not_executable_file_matches() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("tool")).unwrap();
    std::fs::write(temp.path().join("tool.cmd"), b"").unwrap();
    let path = std::env::join_paths([temp.path()]).unwrap();
    assert_eq!(find_on_path_using("tool", &path, &[]), None);
    assert_eq!(
        find_on_path_using("tool", &path, &[".cmd".to_string()]),
        Some(temp.path().join("tool.cmd")),
    );
}

#[cfg(unix)]
#[test]
fn supplied_path_preserves_non_unicode_directory_names() {
    use std::os::unix::ffi::OsStrExt;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join(OsStr::from_bytes(b"bin-\xff"));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("tool"), b"").unwrap();
    let path = std::env::join_paths([&directory]).unwrap();
    assert_eq!(
        find_on_path_using("tool", &path, &[]),
        Some(directory.join("tool"))
    );
}

#[cfg(windows)]
#[test]
fn supplied_path_preserves_unpaired_surrogate_directory_names() {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    let temp = tempfile::tempdir().unwrap();
    let directory = temp
        .path()
        .join(OsString::from_wide(&[0x0062, 0x0069, 0x006e, 0xd800]));
    std::fs::create_dir(&directory).unwrap();
    std::fs::write(directory.join("tool.exe"), b"").unwrap();
    let path = std::env::join_paths([&directory]).unwrap();
    assert_eq!(
        find_on_path_using("tool", &path, &[".exe".to_string()]),
        Some(directory.join("tool.exe")),
    );
}
