#![cfg(feature = "tar-stream")]
use kernal_api::platform::fs::extract_tar_stream;
use std::path::Path;

fn archive() -> Vec<u8> {
    let mut builder = tar::Builder::new(Vec::new());
    for name in ["skip", "keep"] {
        let mut header = tar::Header::new_gnu();
        header.set_size(7);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, name, &b"payload"[..])
            .unwrap();
    }
    builder.into_inner().unwrap()
}

#[test]
fn skipped_payload_is_drained_before_extracting_next_entry() {
    let temp = tempfile::tempdir().unwrap();
    extract_tar_stream(
        &archive()[..],
        temp.path(),
        Some(|path: &Path| Ok(path == Path::new("skip"))),
    )
    .unwrap();
    assert!(!temp.path().join("skip").exists());
    assert_eq!(std::fs::read(temp.path().join("keep")).unwrap(), b"payload");
}

#[test]
fn unfiltered_stream_extracts_all_entries() {
    let temp = tempfile::tempdir().unwrap();
    extract_tar_stream(
        &archive()[..],
        temp.path(),
        None::<fn(&Path) -> std::io::Result<bool>>,
    )
    .unwrap();
    assert_eq!(std::fs::read(temp.path().join("skip")).unwrap(), b"payload");
    assert_eq!(std::fs::read(temp.path().join("keep")).unwrap(), b"payload");
}

#[test]
fn filter_failure_aborts_before_extracting_later_members() {
    let temp = tempfile::tempdir().unwrap();
    let mut calls = 0;
    let error = extract_tar_stream(
        &archive()[..],
        temp.path(),
        Some(|_: &Path| {
            calls += 1;
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "filter rejected entry",
            ))
        }),
    )
    .unwrap_err();
    assert_eq!(calls, 1);
    assert!(error.to_string().contains("filter rejected entry"));
    assert!(!temp.path().join("keep").exists());
    assert!(!temp.path().join("skip").exists());
}

#[test]
fn forward_relative_symbolic_link_remains_readable_after_extraction() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut link = tar::Header::new_gnu();
    link.set_entry_type(tar::EntryType::Symlink);
    link.set_size(0);
    link.set_mode(0o777);
    builder.append_link(&mut link, "alias", "target").unwrap();
    let mut file = tar::Header::new_gnu();
    file.set_entry_type(tar::EntryType::Regular);
    file.set_size(7);
    file.set_mode(0o644);
    builder
        .append_data(&mut file, "target", &b"payload"[..])
        .unwrap();
    let bytes = builder.into_inner().unwrap();
    let temp = tempfile::tempdir().unwrap();
    extract_tar_stream(
        &bytes[..],
        temp.path(),
        None::<fn(&Path) -> std::io::Result<bool>>,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(temp.path().join("alias")).unwrap(),
        b"payload"
    );
    assert_eq!(
        std::fs::read(temp.path().join("target")).unwrap(),
        b"payload"
    );
}

#[cfg(unix)]
#[test]
fn unfiltered_extraction_preserves_deferred_directory_timestamp_restoration() {
    let mut builder = tar::Builder::new(Vec::new());
    let mut dir = tar::Header::new_gnu();
    dir.set_entry_type(tar::EntryType::Directory);
    dir.set_size(0);
    dir.set_mode(0o755);
    dir.set_mtime(123456);
    builder
        .append_data(&mut dir, "directory", std::io::empty())
        .unwrap();
    let mut file = tar::Header::new_gnu();
    file.set_entry_type(tar::EntryType::Regular);
    file.set_size(7);
    file.set_mode(0o644);
    file.set_mtime(123457);
    builder
        .append_data(&mut file, "directory/file", &b"payload"[..])
        .unwrap();
    let bytes = builder.into_inner().unwrap();
    let canonical = tempfile::tempdir().unwrap();
    extract_tar_stream(
        &bytes[..],
        canonical.path(),
        None::<fn(&Path) -> std::io::Result<bool>>,
    )
    .unwrap();
    let reference = tempfile::tempdir().unwrap();
    tar::Archive::new(&bytes[..])
        .unpack(reference.path())
        .unwrap();
    for relative in ["directory", "directory/file"] {
        assert_eq!(
            std::fs::metadata(canonical.path().join(relative))
                .unwrap()
                .modified()
                .unwrap(),
            std::fs::metadata(reference.path().join(relative))
                .unwrap()
                .modified()
                .unwrap(),
            "native unfiltered metadata restoration must be preserved for {relative}",
        );
    }
}
