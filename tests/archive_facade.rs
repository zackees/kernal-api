#![cfg(feature = "archive")]

use kernal_api::archive::{extract, ArchiveFormat, ExtractionLimits};
use std::io::Write;

/// Read-only source fixture; extracted data is isolated and never executed.
#[test]
#[ignore = "requires KERNAL_ARCHIVE_ZIP_FIXTURE pointing to a real archive"]
fn real_zip_fixture_extracts_into_fresh_staging() {
    let archive = std::env::var_os("KERNAL_ARCHIVE_ZIP_FIXTURE").expect("fixture path");
    let temp = tempfile::tempdir().unwrap();
    extract(
        std::path::Path::new(&archive),
        temp.path(),
        ArchiveFormat::Zip,
        ExtractionLimits::default(),
    )
    .unwrap();
    assert!(std::fs::read_dir(temp.path()).unwrap().next().is_some());
}

#[cfg(unix)]
fn zip_links(links: &[(&str, &str)]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file("lib/tool", options).unwrap();
    writer.write_all(b"executable").unwrap();
    for (path, target) in links {
        writer.add_symlink(*path, *target, options).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[cfg(unix)]
#[test]
fn zip_preserves_internal_link_chains_and_relative_parent_targets() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    std::fs::write(
        &archive,
        zip_links(&[("bin/tool", "../alias/tool"), ("alias", "lib")]),
    )
    .unwrap();
    let out = dir.path().join("out");
    extract(
        &archive,
        &out,
        ArchiveFormat::Zip,
        ExtractionLimits::default(),
    )
    .unwrap();
    assert_eq!(std::fs::read(out.join("bin/tool")).unwrap(), b"executable");
    assert_eq!(
        std::fs::read_link(out.join("bin/tool")).unwrap(),
        std::path::PathBuf::from("../alias/tool")
    );
}

#[cfg(unix)]
#[test]
fn zip_rejects_escaping_cyclic_and_dangling_links_before_creating_them() {
    for links in [
        vec![("bad", "../outside")],
        vec![("bad", "/outside")],
        vec![("bad", "other"), ("other", "bad")],
        vec![("bad", "missing")],
        vec![("alias", "."), ("bad", "alias/../outside")],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("input.zip");
        std::fs::write(&archive, zip_links(&links)).unwrap();
        let out = dir.path().join("out");
        assert!(extract(
            &archive,
            &out,
            ArchiveFormat::Zip,
            ExtractionLimits::default()
        )
        .is_err());
        assert!(std::fs::symlink_metadata(out.join("bad")).is_err());
    }
}

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

#[test]
fn zip_rejects_inconsistent_directory_entry_counts() {
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("input.zip");
    let mut bytes = zip_fixture(&[("file", b"data")]);
    let end = bytes.len() - 22;
    bytes[end + 10..end + 12].copy_from_slice(&0_u16.to_le_bytes());
    std::fs::write(&archive, bytes).unwrap();
    assert!(extract(
        &archive,
        &dir.path().join("out"),
        ArchiveFormat::Zip,
        ExtractionLimits::default()
    )
    .is_err());
}

#[test]
fn zip64_metadata_is_bounded_before_backend_allocation() {
    let original = zip_fixture(&[("file", b"data")]);
    let footer = &original[original.len() - 22..];
    let offset = (original.len() - 22) as u64;
    let directory_size = u32::from_le_bytes(footer[12..16].try_into().unwrap()) as u64;
    let directory_offset = u32::from_le_bytes(footer[16..20].try_into().unwrap()) as u64;
    for count in [1, u64::MAX] {
        let mut bytes = original[..original.len() - 22].to_vec();
        bytes.extend_from_slice(b"PK\x06\x06");
        bytes.extend_from_slice(&44_u64.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&45_u16.to_le_bytes());
        bytes.extend_from_slice(&[0; 8]);
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&count.to_le_bytes());
        bytes.extend_from_slice(&directory_size.to_le_bytes());
        bytes.extend_from_slice(&directory_offset.to_le_bytes());
        bytes.extend_from_slice(b"PK\x06\x07");
        bytes.extend_from_slice(&0_u32.to_le_bytes());
        bytes.extend_from_slice(&offset.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        let mut footer = footer.to_vec();
        footer[8..20].fill(0xff);
        bytes.extend_from_slice(&footer);
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("input.zip");
        std::fs::write(&archive, bytes).unwrap();
        let out = dir.path().join("out");
        let result = extract(
            &archive,
            &out,
            ArchiveFormat::Zip,
            ExtractionLimits::default(),
        );
        if count == 1 {
            result.unwrap();
            assert_eq!(std::fs::read(out.join("file")).unwrap(), b"data");
        } else {
            assert!(result.is_err());
            assert!(!out.exists());
        }
    }
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
