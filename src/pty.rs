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
}
