//! macOS extent-sharing observation from the volume's file system type.

use std::io;
use std::os::unix::io::AsRawFd as _;
use std::path::Path;

use crate::platform::fs::ExtentSharing;

/// File systems that cannot clone blocks between files.
const NON_SHARING: &[&str] = &["hfs", "msdos", "exfat"];

pub fn extent_sharing(path: &Path) -> io::Result<ExtentSharing> {
    let file = std::fs::File::open(path)?;
    // SAFETY: `statfs` is plain data written by the call below.
    let mut info: libc::statfs = unsafe { std::mem::zeroed() };
    // SAFETY: the descriptor is open and `info` is live writable storage.
    if unsafe { libc::fstatfs(file.as_raw_fd(), &mut info) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let name: Vec<u8> = info
        .f_fstypename
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect();
    let name = String::from_utf8_lossy(&name);
    Ok(if NON_SHARING.iter().any(|fs| name.eq_ignore_ascii_case(fs)) {
        ExtentSharing::Exclusive
    } else {
        ExtentSharing::Unknown
    })
}
