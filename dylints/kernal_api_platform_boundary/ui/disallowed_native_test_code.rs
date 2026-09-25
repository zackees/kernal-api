// Test code is not a blanket exemption, and code the host compiles away is
// still inspected: this module is elided outside `cargo test`.
#[cfg(test)]
mod tests {
    use windows_sys::Win32::Foundation::CloseHandle;

    #[test]
    fn closes() {
        let _ = CloseHandle;
    }
}

fn main() {}
