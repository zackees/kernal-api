use kernal_api::platform::fs::write_at;
use std::io::{Read, Seek, SeekFrom, Write};

#[test]
fn offset_write_updates_requested_region_not_current_cursor() {
    let mut file = tempfile::tempfile().unwrap();
    file.write_all(b"abcdef").unwrap();
    file.seek(SeekFrom::Start(5)).unwrap();
    assert_eq!(write_at(&file, b"XY", 1).unwrap(), 2);
    file.seek(SeekFrom::Start(0)).unwrap();
    let mut contents = Vec::new();
    file.read_to_end(&mut contents).unwrap();
    assert_eq!(contents, b"aXYdef");
}

#[cfg(unix)]
#[test]
fn unix_offset_write_leaves_shared_cursor_unchanged() {
    let mut file = tempfile::tempfile().unwrap();
    file.seek(SeekFrom::Start(7)).unwrap();
    assert_eq!(write_at(&file, b"x", 0).unwrap(), 1);
    assert_eq!(file.stream_position().unwrap(), 7);
}
