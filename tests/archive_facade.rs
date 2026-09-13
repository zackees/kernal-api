#![cfg(feature = "archive")]

use kernal_api::archive::{extract, ArchiveFormat, ExtractionLimits};
use std::io::Write;

fn zip_fixture(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (path, content) in files {
        writer
            .start_file(*path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[test]
fn zip_extracts_nested_files_and_creates_destination() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    std::fs::write(
        &archive,
        zip_fixture(&[("hello.txt", b"hello"), ("sub/world.txt", b"world")]),
    )
    .unwrap();
    let out = dir.path().join("nested/out");
    extract(
        &archive,
        &out,
        ArchiveFormat::Zip,
        ExtractionLimits::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(out.join("hello.txt")).unwrap(), b"hello");
    assert_eq!(std::fs::read(out.join("sub/world.txt")).unwrap(), b"world");
}

#[test]
fn zip_rejects_traversal_and_resource_overflow() {
    for path in ["../escape", "/absolute", "C:/drive", "sub\\escape"] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("input.zip");
        std::fs::write(&archive, zip_fixture(&[(path, b"data")])).unwrap();
        assert!(extract(
            &archive,
            &dir.path().join("out"),
            ArchiveFormat::Zip,
            ExtractionLimits::default()
        )
        .is_err());
        assert!(!dir.path().join("escape").exists());
    }
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    std::fs::write(&archive, zip_fixture(&[("large", b"12345")])).unwrap();
    let limits = ExtractionLimits {
        max_output_bytes: 4,
        ..ExtractionLimits::default()
    };
    assert!(extract(
        &archive,
        &dir.path().join("out"),
        ArchiveFormat::Zip,
        limits
    )
    .is_err());
}

#[test]
fn zip_rejects_input_metadata_entry_limits_and_nonempty_destination() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    std::fs::write(&archive, zip_fixture(&[("file", b"data")])).unwrap();
    for (index, limits) in [
        ExtractionLimits {
            max_input_bytes: 1,
            ..ExtractionLimits::default()
        },
        ExtractionLimits {
            max_entries: 0,
            ..ExtractionLimits::default()
        },
        ExtractionLimits {
            max_metadata_bytes: 1,
            ..ExtractionLimits::default()
        },
        ExtractionLimits {
            max_path_bytes: 1,
            ..ExtractionLimits::default()
        },
    ]
    .into_iter()
    .enumerate()
    {
        assert!(extract(
            &archive,
            &dir.path().join(format!("out{index}")),
            ArchiveFormat::Zip,
            limits
        )
        .is_err());
    }
    let out = dir.path().join("existing");
    std::fs::create_dir(&out).unwrap();
    std::fs::write(out.join("file"), b"preserve").unwrap();
    assert!(extract(
        &archive,
        &out,
        ArchiveFormat::Zip,
        ExtractionLimits::default()
    )
    .is_err());
    assert_eq!(std::fs::read(out.join("file")).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn zip_preserves_executable_bits_without_special_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    writer
        .start_file(
            "tool",
            zip::write::SimpleFileOptions::default().unix_permissions(0o755),
        )
        .unwrap();
    writer.write_all(b"tool").unwrap();
    std::fs::write(&archive, writer.finish().unwrap().into_inner()).unwrap();
    let out = dir.path().join("out");
    extract(
        &archive,
        &out,
        ArchiveFormat::Zip,
        ExtractionLimits::default(),
    )
    .unwrap();
    assert_eq!(
        std::fs::metadata(out.join("tool"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o755
    );
}
