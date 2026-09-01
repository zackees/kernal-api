//! Private bridge from facade process semantics to the native substrate.

use std::io;
use std::process::{Command, ExitStatus};
use std::time::Duration;

use running_process::{
    run_std_command_bounded, AsyncProcess, AsyncProcessBuilder, AsyncStdio, ProcessError,
};

use crate::{
    BoundedProcessError, BoundedProcessExit, BoundedProcessOutput, PlatformChild,
    ProcessCaptureError, ProcessOutput, SpawnSpec, StreamMode,
};

/// Private child state from the selected native substrate.
pub(crate) struct ProcessAdapter {
    process: AsyncProcess,
    pid: Option<u32>,
}

pub(crate) async fn spawn(spec: SpawnSpec) -> io::Result<PlatformChild> {
    let mut builder = AsyncProcessBuilder::new(spec.program);
    for arg in spec.args {
        builder = builder.arg(arg);
    }
    if let Some(current_dir) = spec.current_dir {
        builder = builder.current_dir(current_dir);
    }
    if spec.clear_env {
        builder = builder.clear_env(true);
    }
    for (key, value) in spec.env {
        builder = builder.env(key, value);
    }
    builder = builder
        .stdin(stdio(spec.stdin))
        .stdout(stdio(spec.stdout))
        .stderr(stdio(spec.stderr))
        .create_process_group(spec.create_process_group)
        .kill_when_owner_dies(spec.kill_when_owner_dies);

    let mut process = builder.build();
    process.start().await.map_err(process_error_to_io)?;
    let pid = process.pid().await.map_err(process_error_to_io)?;
    Ok(PlatformChild::new(ProcessAdapter {
        process,
        pid: Some(pid),
    }))
}

/// Run a one-shot command through the private bounded native substrate.
///
/// `Command` is deliberately assembled here, rather than at the facade
/// boundary, so native command state and the selected substrate remain
/// implementation details. The substrate owns containment, pipe capture, and
/// timeout/overflow cleanup.
pub(crate) fn run_bounded(
    spec: SpawnSpec,
    timeout: Option<Duration>,
    output_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    let command = std_command(spec);
    run_std_command_bounded(command, timeout, output_limit)
        .map(|output| BoundedProcessOutput {
            exit: BoundedProcessExit::from_raw_code(output.exit_code),
            stdout: output.stdout,
            stderr: output.stderr,
        })
        .map_err(bounded_process_error)
}

fn std_command(spec: SpawnSpec) -> Command {
    let mut command = Command::new(spec.program);
    command.args(spec.args);
    if let Some(current_dir) = spec.current_dir {
        command.current_dir(current_dir);
    }
    if spec.clear_env {
        command.env_clear();
    }
    command.envs(spec.env);
    command
}

impl ProcessAdapter {
    pub(crate) fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.process.wait().await.map_err(process_error_to_io)?;
        self.pid = None;
        Ok(status)
    }

    pub(crate) async fn kill(&mut self) -> io::Result<()> {
        self.process.kill().await.map_err(process_error_to_io)?;
        self.pid = None;
        Ok(())
    }

    pub(crate) async fn terminate_group_soft(&self) -> io::Result<bool> {
        self.process
            .terminate_group_soft()
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn write_stdin(&self, bytes: &[u8]) -> io::Result<()> {
        self.process
            .write_stdin(bytes)
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn close_stdin(&self) -> io::Result<()> {
        self.process
            .close_stdin()
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn capture_bounded(
        &mut self,
        limit: usize,
    ) -> Result<ProcessOutput, ProcessCaptureError> {
        let result = self
            .process
            .capture_bounded(limit)
            .await
            .map_err(process_capture_error)
            .map(|output| ProcessOutput {
                status: output.status,
                stdout: output.stdout,
                stderr: output.stderr,
            });
        if matches!(
            &result,
            Ok(_) | Err(ProcessCaptureError::OutputLimitExceeded { .. })
        ) {
            self.pid = None;
        }
        result
    }
}

fn stdio(mode: StreamMode) -> AsyncStdio {
    match mode {
        StreamMode::Inherit => AsyncStdio::Inherit,
        StreamMode::Piped => AsyncStdio::Piped,
        StreamMode::Null => AsyncStdio::Null,
    }
}

fn process_capture_error(error: ProcessError) -> ProcessCaptureError {
    match error {
        ProcessError::OutputLimitExceeded { limit } => {
            ProcessCaptureError::OutputLimitExceeded { limit }
        }
        error => ProcessCaptureError::Io(process_error_to_io(error)),
    }
}

fn bounded_process_error(error: ProcessError) -> BoundedProcessError {
    match error {
        ProcessError::Timeout => BoundedProcessError::TimedOut,
        ProcessError::OutputLimitExceeded { limit } => {
            BoundedProcessError::OutputLimitExceeded { limit }
        }
        ProcessError::Spawn(error) => BoundedProcessError::Spawn(error),
        ProcessError::Io(error) => BoundedProcessError::Io(error),
        error => BoundedProcessError::Io(process_error_to_io(error)),
    }
}

fn process_error_to_io(error: ProcessError) -> io::Error {
    match error {
        ProcessError::Spawn(error) | ProcessError::Io(error) => error,
        ProcessError::AlreadyStarted => io::Error::new(io::ErrorKind::AlreadyExists, error),
        ProcessError::NotRunning => io::Error::new(io::ErrorKind::BrokenPipe, error),
        ProcessError::RuntimeContext => io::Error::other(error),
        ProcessError::StdinUnavailable => io::Error::new(io::ErrorKind::BrokenPipe, error),
        ProcessError::Timeout => io::Error::new(io::ErrorKind::TimedOut, error),
        ProcessError::OutputLimitExceeded { .. } => {
            io::Error::new(io::ErrorKind::FileTooLarge, error)
        }
    }
}
