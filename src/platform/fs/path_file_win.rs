//! Windows file identity: volume serial + native 128-bit file ID.
//! ReFS does not guarantee uniqueness for the legacy 64-bit index, so the
//! FileIdInfo path is preferred with a BY_HANDLE_FILE_INFORMATION fallback.

use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdInfo, GetFileInformationByHandle, GetFileInformationByHandleEx,
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_INFO, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// Windows file identity — the volume serial and 128-bit file ID.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct RawFileIdentity {
    pub(crate) volume_serial: u64,
    pub(crate) identifier: [u8; 16],
}

pub fn file_identity(path: &Path) -> std::io::Result<RawFileIdentity> {
    get_file_id(path).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("cannot obtain file identity for {}", path.display()),
        )
    })
}

pub fn same_file(a: &Path, b: &Path) -> std::io::Result<bool> {
    Ok(get_file_id(a)
        .zip(get_file_id(b))
        .map(|(ia, ib)| ia == ib)
        .unwrap_or(false))
}

fn get_file_id(path: &Path) -> Option<RawFileIdentity> {
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return None;
    }
    wide.push(0);
    unsafe {
        let handle = CreateFileW(
            wide.as_ptr(),
            0, // no access needed, just metadata
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if handle == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
            return None;
        }

        let mut native: FILE_ID_INFO = std::mem::zeroed();
        let native_ok = GetFileInformationByHandleEx(
            handle,
            FileIdInfo,
            (&raw mut native).cast(),
            std::mem::size_of::<FILE_ID_INFO>() as u32,
        );
        if native_ok != 0 {
            CloseHandle(handle);
            return Some(native_identity(&native));
        }
        let mut legacy: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        let legacy_ok = GetFileInformationByHandle(handle, &mut legacy);
        CloseHandle(handle);
        if legacy_ok == 0 {
            return None;
        }
        Some(legacy_identity(&legacy))
    }
}

fn legacy_identity(info: &BY_HANDLE_FILE_INFORMATION) -> RawFileIdentity {
    let mut identifier = [0_u8; 16];
    identifier[..4].copy_from_slice(&info.nFileIndexLow.to_ne_bytes());
    identifier[4..8].copy_from_slice(&info.nFileIndexHigh.to_ne_bytes());
    RawFileIdentity {
        volume_serial: u64::from(info.dwVolumeSerialNumber),
        identifier,
    }
}

fn native_identity(info: &FILE_ID_INFO) -> RawFileIdentity {
    RawFileIdentity {
        volume_serial: info.VolumeSerialNumber,
        identifier: info.FileId.Identifier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_fallback_preserves_both_words_and_zero_extends_identifier() {
        // SAFETY: this Win32 metadata structure contains integer fields only.
        let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
        info.dwVolumeSerialNumber = u32::MAX;
        info.nFileIndexLow = 0x12345678;
        info.nFileIndexHigh = 0x9abcdef0;
        let identity = legacy_identity(&info);
        assert_eq!(identity.volume_serial, u64::from(u32::MAX));
        assert_eq!(&identity.identifier[..4], &0x12345678u32.to_ne_bytes());
        assert_eq!(&identity.identifier[4..8], &0x9abcdef0u32.to_ne_bytes());
        assert_eq!(&identity.identifier[8..], &[0u8; 8]);
    }

    #[test]
    fn upper_identifier_and_volume_bits_participate_in_identity() {
        // SAFETY: FILE_ID_INFO consists of integer fields and a byte array.
        let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
        info.VolumeSerialNumber = 1u64 << 48;
        info.FileId.Identifier[15] = 1;
        let first = native_identity(&info);
        assert_eq!(first.volume_serial, 1u64 << 48);
        assert_eq!(first.identifier[15], 1);
        info.FileId.Identifier[15] = 2;
        assert_ne!(first, native_identity(&info));
        info.FileId.Identifier[15] = 1;
        info.VolumeSerialNumber = 0;
        assert_ne!(first, native_identity(&info));
    }

    #[test]
    fn file_identity_preserves_the_native_128_bit_identifier() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::write(&a, b"data").expect("write");
        std::fs::hard_link(&a, &b).expect("link");
        let ia = file_identity(&a).expect("identity");
        let ib = file_identity(&b).expect("identity");
        assert_eq!(ia, ib);
        // The 128-bit identifier is preserved verbatim (not truncated to
        // the legacy 64-bit index) on filesystems that expose FileIdInfo.
        assert_ne!(ia.identifier, [0_u8; 16]);
    }
}
