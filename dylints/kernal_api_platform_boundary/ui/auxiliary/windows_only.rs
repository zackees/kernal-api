use std::os::windows::ffi::OsStrExt;

pub fn wide(value: &std::ffi::OsStr) -> Vec<u16> {
    value.encode_wide().collect()
}
