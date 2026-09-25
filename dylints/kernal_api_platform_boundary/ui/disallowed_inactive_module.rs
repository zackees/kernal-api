// An out-of-line module that the host's `cfg` elides is still read: a Linux
// pass sees both the selecting `cfg` and the Windows API inside the file.
#[cfg(windows)]
#[path = "auxiliary/windows_only.rs"]
mod windows_only;

fn main() {}
