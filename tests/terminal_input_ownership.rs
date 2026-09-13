#![cfg(all(unix, feature = "pty"))]

use std::io;
use std::os::fd::{FromRawFd, OwnedFd};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn stdin_mode() -> libc::termios {
    let mut mode = std::mem::MaybeUninit::uninit();
    // SAFETY: mode is writable and stdin is a live PTY slave in the child.
    assert_eq!(unsafe { libc::tcgetattr(0, mode.as_mut_ptr()) }, 0);
    // SAFETY: successful tcgetattr initialized mode.
    unsafe { mode.assume_init() }
}

#[test]
fn native_session_rejects_overlap_and_restores_mode() {
    if std::env::var_os("KERNAL_INPUT_OWNERSHIP_CHILD").is_some() {
        let before = stdin_mode();
        assert_ne!(before.c_lflag & libc::ICANON, 0);
        let session = kernal_api::TerminalInputSession::new().unwrap().unwrap();
        assert_eq!(stdin_mode().c_lflag & libc::ICANON, 0);
        assert_eq!(
            kernal_api::TerminalInputSession::new()
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        // The probe must not steal input or change modes while capture owns it.
        let probe = kernal_api::platform::terminal::active_graphics_probe(Duration::ZERO);
        assert!(probe.kitty_graphics.is_none());
        assert_eq!(stdin_mode().c_lflag & libc::ICANON, 0);
        drop(session);
        let after = stdin_mode();
        assert_eq!(before.c_iflag, after.c_iflag);
        assert_eq!(before.c_oflag, after.c_oflag);
        assert_eq!(before.c_cflag, after.c_cflag);
        assert_eq!(before.c_lflag, after.c_lflag);
        assert_eq!(before.c_cc, after.c_cc);
        drop(kernal_api::TerminalInputSession::new().unwrap().unwrap());
        return;
    }

    let mut master = -1;
    let mut slave = -1;
    // SAFETY: valid output pointers; null optional arguments request defaults.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    // SAFETY: successful openpty returns two fresh owned descriptors.
    let _master = unsafe { OwnedFd::from_raw_fd(master) };
    // SAFETY: slave is the other fresh descriptor, transferred to child stdin.
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native_session_rejects_overlap_and_restores_mode",
            "--nocapture",
        ])
        .env("KERNAL_INPUT_OWNERSHIP_CHILD", "1")
        .stdin(Stdio::from(slave))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("terminal ownership child timed out");
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
