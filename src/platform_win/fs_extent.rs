//! Windows extent-sharing observation from the volume's file system name.

use std::io;
use std::os::windows::io::AsRawHandle as _;
use std::path::Path;

use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationByHandleW;

use crate::platform::fs::ExtentSharing;

/// File systems that cannot clone blocks between files. ReFS can.
const NON_SHARING: &[&str] = &["NTFS", "FAT", "FAT32", "exFAT"];

pub fn extent_sharing(path: &Path) -> io::Result<ExtentSharing> {
    let file = std::fs::File::open(path)?;
    let mut name = [0_u16; 64];
    // SAFETY: the handle is open for the call, `name` is live writable
    // storage of the stated length, and every other output is declined.
    let ok = unsafe {
        GetVolumeInformationByHandleW(
            file.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let len = name.iter().position(|&unit| unit == 0).unwrap_or(name.len());
    let name = String::from_utf16_lossy(&name[..len]);
    Ok(if NON_SHARING.iter().any(|fs| name.eq_ignore_ascii_case(fs)) {
        ExtentSharing::Exclusive
    } else {
        ExtentSharing::Unknown
    })
}
