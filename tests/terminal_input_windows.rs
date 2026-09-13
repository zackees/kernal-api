#![cfg(all(windows, feature = "pty"))]

use std::io;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn native_console_session_restores_mode_and_excludes_overlap() {
    if std::env::var_os("KERNAL_CONSOLE_OWNERSHIP_CHILD").is_some() {
        use winapi::um::consoleapi::{AllocConsole, GetConsoleMode};
        use winapi::um::fileapi::{CreateFileW, OPEN_EXISTING};
        use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
        use winapi::um::processenv::SetStdHandle;
        use winapi::um::winbase::STD_INPUT_HANDLE;
        use winapi::um::wincon::FreeConsole;
        use winapi::um::winnt::{FILE_SHARE_READ, FILE_SHARE_WRITE, GENERIC_READ, GENERIC_WRITE};

        // This child owns a fresh console; never change the test runner's console.
        // SAFETY: detaching this process does not detach its parent.
        unsafe { FreeConsole() };
        // SAFETY: AllocConsole takes no pointers and attaches only this process.
        assert_ne!(
            unsafe { AllocConsole() },
            0,
            "{}",
            io::Error::last_os_error()
        );
        let name: Vec<u16> = "CONIN$\0".encode_utf16().collect();
        // SAFETY: name is terminated; null security/template request defaults.
        let input = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null_mut(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(input, INVALID_HANDLE_VALUE);
        // SAFETY: input is a live console handle owned by this child.
        assert_ne!(unsafe { SetStdHandle(STD_INPUT_HANDLE, input) }, 0);
        let mode = || {
            let mut value = 0;
            // SAFETY: input remains live and value is writable.
            assert_ne!(unsafe { GetConsoleMode(input, &mut value) }, 0);
            value
        };
        let before = mode();
        let session = kernal_api::TerminalInputSession::new().unwrap().unwrap();
        assert_ne!(before, mode());
        assert_eq!(
            kernal_api::TerminalInputSession::new()
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        drop(session);
        assert_eq!(before, mode());
        drop(kernal_api::TerminalInputSession::new().unwrap().unwrap());
        assert_eq!(before, mode());
        // SAFETY: all capture workers have joined before this owned handle closes.
        assert_ne!(unsafe { CloseHandle(input) }, 0);
        // SAFETY: releases only this child's console attachment.
        unsafe { FreeConsole() };
        return;
    }

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_console_session_restores_mode_and_excludes_overlap",
            "--nocapture",
        ])
        .env("KERNAL_CONSOLE_OWNERSHIP_CHILD", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("console ownership child timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
