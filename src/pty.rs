//! Owned pseudo-terminal session facade.

use crate::{
    platform::terminal::{PtyBackend, PtyChild, PtyMaster, PtySize, PtySlave},
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
