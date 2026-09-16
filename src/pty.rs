//! Owned pseudo-terminal session facade.

#[cfg(not(windows))]
use crate::platform::terminal::PtyChild;
use crate::{
    platform::terminal::{PtyBackend, PtyMaster, PtySize, PtySlave},
    Backend,
};
use std::{
    ffi::OsString,
    io::{self, Read, Write},
    path::PathBuf,
};

/// Caller-selected process program and arguments for a native PTY session.
#[derive(Debug, Clone)]
pub struct PtyCommand {
    pub program: OsString,
    pub arguments: Vec<OsString>,
    pub cwd: Option<PathBuf>,
    pub environment: Option<Vec<(OsString, OsString)>>,
}

impl PtyCommand {
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            arguments: Vec::new(),
            cwd: None,
            environment: None,
        }
    }
}

/// An owned child process and its pseudo-terminal.
pub struct PtySession {
    master: <Backend as PtyBackend>::Master,
    child: <<Backend as PtyBackend>::Slave as PtySlave>::Child,
    writer: Box<dyn Write + Send>,
}

impl PtySession {
    pub fn spawn(command: PtyCommand, size: PtySize) -> io::Result<(Self, Box<dyn Read + Send>)> {
        let (mut master, slave) = Backend::openpty(size)?;
        let reader = master.try_clone_reader()?;
        let writer = master.take_writer()?;
        let mut argv = Vec::with_capacity(command.arguments.len() + 1);
        argv.push(command.program);
        argv.extend(command.arguments);
        let child = slave.spawn(
            &argv,
            command.cwd.as_deref(),
            command.environment.as_deref(),
        )?;
        Ok((
            Self {
                master,
                child,
                writer,
            },
            reader,
        ))
    }

    pub fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.writer
            .write_all(bytes)
            .and_then(|_| self.writer.flush())
    }
    pub fn resize(&self, size: PtySize) -> io::Result<()> {
        self.master.resize(size)
    }

    /// The externally meaningful process identifier for this session's child.
    ///
    /// A caller that must end a session from another thread uses this with
    /// [`crate::platform::terminal::signal_pty_tree`]. The session itself is not
    /// reachable then: this type is not `Sync`, and the thread that owns it is
    /// typically parked inside [`PtySession::write`] on a full terminal input
    /// queue, which is exactly the state that needs a way out.
    pub fn pid(&self) -> Option<u32> {
        self.master.preferred_pid(&self.child)
    }
    pub fn try_wait(&mut self) -> io::Result<Option<u32>> {
        self.child.try_wait()
    }
}

impl Drop for PtySession {
    fn drop(&mut self) {
        let _ = self.master.kill_process_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn session_owns_a_shell_command_and_its_pty_io() {
        let mut command = PtyCommand::new("/bin/sh");
        command.arguments = vec!["-c".into(), "printf kernal-pty-session".into()];
        let (mut session, mut reader) = PtySession::spawn(
            command,
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
        )
        .expect("spawn shell in PTY");

        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 64];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                // Linux PTYs report EIO when the slave closes normally.
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("read PTY output: {error}"),
            }
        }
        assert!(String::from_utf8_lossy(&bytes).contains("kernal-pty-session"));

        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(status) = session.try_wait().expect("reap shell") {
                assert_eq!(status, 0);
                break;
            }
            assert!(Instant::now() < deadline, "shell did not exit promptly");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Signalling the reported pid is the documented escape from a write parked
    /// on a full terminal input queue, so the pid must be one that ends the
    /// child rather than merely identifying it.
    #[cfg(unix)]
    #[test]
    fn reported_pid_terminates_the_session() {
        let mut command = PtyCommand::new("/bin/sh");
        command.arguments = vec!["-c".into(), "sleep 30".into()];
        let (mut session, _reader) = PtySession::spawn(
            command,
            PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            },
        )
        .expect("spawn a long-lived child in a PTY");

        let pid = session
            .pid()
            .expect("a spawned session reports its child pid");
        assert!(pid > 0, "pid must be a real identifier, got {pid}");
        assert!(
            session.try_wait().expect("poll child").is_none(),
            "the child should still be running before it is signalled"
        );

        crate::platform::terminal::signal_pty_tree(pid, true).expect("signal the session tree");
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if session.try_wait().expect("reap signalled child").is_some() {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "signalling the reported pid did not end the session"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
