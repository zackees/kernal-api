//! Windows journal sequence observation; no timestamp fallback.
use super::FileChangeMarker;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};

pub fn file_change_marker(path: &Path) -> Option<FileChangeMarker> {
    use windows_sys::Win32::Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Ioctl::{FSCTL_READ_FILE_USN_DATA, READ_FILE_USN_DATA};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == INVALID_HANDLE_VALUE {
            return None;
        }
        let query = READ_FILE_USN_DATA {
            MinMajorVersion: 2,
            MaxMajorVersion: 4,
        };
        let mut record = [0_u8; 512];
        let mut returned = 0_u32;
        let usn_ok = DeviceIoControl(
            handle,
            FSCTL_READ_FILE_USN_DATA,
            (&raw const query).cast(),
            std::mem::size_of::<READ_FILE_USN_DATA>() as u32,
            record.as_mut_ptr().cast(),
            record.len() as u32,
            &raw mut returned,
            std::ptr::null_mut(),
        );
        if usn_ok != 0 {
            if let Some(usn) = record.get(..returned as usize).and_then(decode_usn) {
                let _ = CloseHandle(handle);
                return Some(FileChangeMarker(i128::from(usn)));
            }
        }
        let _ = CloseHandle(handle);
        None
    }
}

fn decode_usn(record: &[u8]) -> Option<i64> {
    use windows_sys::Win32::System::Ioctl::{USN_RECORD_V2, USN_RECORD_V3};
    if record.len() < 8 {
        return None;
    }
    let major = u16::from_ne_bytes([record[4], record[5]]);
    // SAFETY: each arm checks the complete native structure size, and the
    // unaligned read copies the value without creating an aligned reference.
    unsafe {
        match major {
            2 if record.len() >= std::mem::size_of::<USN_RECORD_V2>() => {
                Some(std::ptr::read_unaligned(record.as_ptr().cast::<USN_RECORD_V2>()).Usn)
            }
            3 if record.len() >= std::mem::size_of::<USN_RECORD_V3>() => {
                Some(std::ptr::read_unaligned(record.as_ptr().cast::<USN_RECORD_V3>()).Usn)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::System::Ioctl::{USN_RECORD_V2, USN_RECORD_V3};

    #[test]
    fn decodes_supported_versions_without_alignment_assumptions() {
        for (version, size, offset) in [
            (
                2u16,
                std::mem::size_of::<USN_RECORD_V2>(),
                std::mem::offset_of!(USN_RECORD_V2, Usn),
            ),
            (
                3u16,
                std::mem::size_of::<USN_RECORD_V3>(),
                std::mem::offset_of!(USN_RECORD_V3, Usn),
            ),
        ] {
            let mut bytes = vec![0u8; size + 1];
            let record = &mut bytes[1..];
            record[4..6].copy_from_slice(&version.to_ne_bytes());
            record[offset..offset + 8].copy_from_slice(&1234i64.to_ne_bytes());
            assert_eq!(decode_usn(record), Some(1234));
            assert_eq!(decode_usn(&record[..7]), None);
            assert_eq!(decode_usn(&record[..size - 1]), None);
            record[4..6].copy_from_slice(&4u16.to_ne_bytes());
            assert_eq!(decode_usn(record), None);
        }
    }
}
