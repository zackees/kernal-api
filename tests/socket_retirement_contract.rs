#![cfg(feature = "ipc")]

use kernal_api::platform::ipc::retire_socket_endpoint;

#[test]
fn missing_path_retirement_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("absent");
    retire_socket_endpoint(&path).unwrap();
    retire_socket_endpoint(&path).unwrap();
}

#[cfg(unix)]
#[test]
fn retirement_refuses_files_directories_and_symlinks() {
    use std::os::unix::fs::symlink;
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file");
    let link = temp.path().join("link");
    let missing_link = temp.path().join("missing-link");
    std::fs::write(&file, b"keep").unwrap();
    symlink(&file, &link).unwrap();
    symlink(temp.path().join("absent"), &missing_link).unwrap();
    for path in [
        file.as_path(),
        link.as_path(),
        missing_link.as_path(),
        temp.path(),
    ] {
        assert_eq!(
            retire_socket_endpoint(path).unwrap_err().kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert!(std::fs::symlink_metadata(path).is_ok());
    }
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
}

#[cfg(unix)]
#[test]
fn retirement_unlinks_only_the_socket_name() {
    let temp = tempfile::tempdir().unwrap();
    let socket = temp.path().join("socket");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    drop(listener);
    retire_socket_endpoint(&socket).unwrap();
    assert!(!socket.exists());
}

#[cfg(unix)]
#[test]
fn canonical_endpoint_retire_uses_the_checked_socket_policy() {
    use kernal_api::platform::ipc::Endpoint;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("endpoint");
    std::fs::write(&path, b"unrelated").unwrap();
    let endpoint = Endpoint::new(path.to_str().unwrap()).unwrap();
    assert_eq!(
        endpoint.retire().unwrap_err().kind(),
        std::io::ErrorKind::InvalidInput
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"unrelated");
}

#[cfg(windows)]
#[test]
fn windows_retirement_does_not_remove_a_filesystem_file() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("file");
    std::fs::write(&file, b"keep").unwrap();
    retire_socket_endpoint(&file).unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), b"keep");
}
