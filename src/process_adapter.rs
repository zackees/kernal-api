//! Private bridge from facade process semantics to the native substrate.

use std::io;
use std::process::ExitStatus;
use std::time::Duration;

use running_process::{
    AsyncProcess, AsyncProcessBuilder, AsyncProcessSessionControl, AsyncProcessSessionEvent,
    AsyncProcessSessionOptions, AsyncProcessSessionOutput, AsyncStdio, BoundedRunOptions,
    ProcessError, StreamKind,
};

use crate::{
    BoundedProcessError, BoundedProcessOutput, PlatformChild, ProcessCaptureError, ProcessExit,
    ProcessOutput, ProcessOutputChunk, ProcessOutputCompletion, ProcessOutputEvent,
    ProcessOutputFault, ProcessPostExitDrain, ProcessSession, ProcessSessionExit,
    ProcessSessionOptions, SpawnSpec, StreamMode,
};

/// Private child state from the selected native substrate.
pub(crate) struct ProcessAdapter {
    process: AsyncProcess,
    pid: Option<u32>,
}

/// Private state for the facade-owned streaming session.
pub(crate) struct ProcessSessionAdapter {
    // The native split keeps this lane as the sole terminal owner. Declaring
    // it before the receiver ensures drop activates lifecycle cleanup before
    // facade output detaches.
    control: AsyncProcessSessionControl,
    // The mutex establishes the facade's single output consumer without
    // serializing lifecycle methods behind a pending pipe read.
    output: tokio::sync::Mutex<AsyncProcessSessionOutput>,
    pid: u32,
}

pub(crate) async fn spawn(spec: SpawnSpec) -> io::Result<PlatformChild> {
    let mut process = builder(spec).build();
    process.start().await.map_err(process_error_to_io)?;
    let pid = process.pid().await.map_err(process_error_to_io)?;
    Ok(PlatformChild::new(ProcessAdapter {
        process,
        pid: Some(pid),
    }))
}

/// Start the substrate's one-owner session and retain only facade-owned
/// lifecycle, output, and native-exit semantic types above this boundary.
pub(crate) async fn spawn_session(
    spec: SpawnSpec,
    options: ProcessSessionOptions,
) -> io::Result<ProcessSession> {
    let mut session = builder(spec).session(session_options(options));
    session.start().await.map_err(process_error_to_io)?;
    let (control, output) = session.into_parts().map_err(process_error_to_io)?;
    let pid = control.pid();
    Ok(ProcessSession::new(ProcessSessionAdapter {
        control,
        output: tokio::sync::Mutex::new(output),
        pid,
    }))
}

/// Run one short-lived command with contained, bounded capture.
///
/// The command conversion consumes program, argument, working-directory, and
/// environment fields without UTF-8 conversion. The bounded operation
/// intentionally owns stdio and containment: its native substrate forces null
/// stdin, separate piped stdout and stderr, and a child process group for
/// reliable timeout/overflow cleanup. Owner-death is forwarded to the same
/// native pre-exec containment path as the substrate's bounded runner.
pub(crate) fn run_bounded(
    spec: SpawnSpec,
    timeout: Option<Duration>,
    output_limit: usize,
) -> Result<BoundedProcessOutput, BoundedProcessError> {
    let (command, kill_when_owner_dies, priority) = std_command(spec);
    running_process::run_std_command_bounded_with_options(
        command,
        timeout,
        output_limit,
        BoundedRunOptions::default()
            .kill_when_owner_dies(kill_when_owner_dies)
            .nice(priority.substrate_nice()),
    )
    .map(|output| BoundedProcessOutput {
        exit: ProcessExit::new(output.exit_code),
        stdout: output.stdout,
        stderr: output.stderr,
    })
    .map_err(bounded_process_error)
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

impl ProcessSessionAdapter {
    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    pub(crate) async fn next_output(&self) -> Option<ProcessOutputEvent> {
        let mut output = self.output.lock().await;
        output.next_output().await.map(output_event)
    }

    pub(crate) async fn wait(&self) -> io::Result<ProcessSessionExit> {
        self.control
            .wait()
            .await
            .map(native_exit)
            .map_err(process_error_to_io)
    }

    pub(crate) async fn poll(&self) -> io::Result<Option<ProcessSessionExit>> {
        self.control
            .poll()
            .await
            .map(|status| status.map(native_exit))
            .map_err(process_error_to_io)
    }

    pub(crate) async fn kill(&self) -> io::Result<()> {
        self.control.kill().await.map_err(process_error_to_io)
    }

    pub(crate) async fn terminate_group_soft(&self) -> io::Result<bool> {
        self.control
            .terminate_group_soft()
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn write_stdin(&self, bytes: &[u8]) -> io::Result<()> {
        self.control
            .write_stdin(bytes)
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn close_stdin(&self) -> io::Result<()> {
        self.control
            .close_stdin()
            .await
            .map_err(process_error_to_io)
    }

    pub(crate) async fn cpu_time(&self) -> io::Result<Option<Duration>> {
        self.control.cpu_time().await.map_err(process_error_to_io)
    }
}

fn builder(spec: SpawnSpec) -> AsyncProcessBuilder {
    let SpawnSpec {
        program,
        args,
        current_dir,
        env,
        clear_env,
        stdin,
        stdout,
        stderr,
        create_process_group,
        kill_when_owner_dies,
        priority,
    } = spec;
    let mut builder = AsyncProcessBuilder::new(program);
    for arg in args {
        builder = builder.arg(arg);
    }
    if let Some(current_dir) = current_dir {
        builder = builder.current_dir(current_dir);
    }
    if clear_env {
        builder = builder.clear_env(true);
    }
    for (key, value) in env {
        builder = builder.env(key, value);
    }
    builder
        .stdin(stdio(stdin))
        .stdout(stdio(stdout))
        .stderr(stdio(stderr))
        .create_process_group(create_process_group)
        .kill_when_owner_dies(kill_when_owner_dies)
        .nice(priority.substrate_nice())
}

fn session_options(options: ProcessSessionOptions) -> AsyncProcessSessionOptions {
    AsyncProcessSessionOptions {
        max_queued_chunks: options.max_queued_chunks,
        max_chunk_bytes: options.max_chunk_bytes,
        post_exit_grace: match options.post_exit_drain {
            ProcessPostExitDrain::WaitForEof => None,
            ProcessPostExitDrain::AbandonAfter(grace) => Some(grace),
        },
        kill_on_drop: options.kill_on_drop,
    }
}

fn output_event(event: AsyncProcessSessionEvent) -> ProcessOutputEvent {
    match event {
        AsyncProcessSessionEvent::Chunk(chunk) => ProcessOutputEvent::Chunk(match chunk.stream {
            StreamKind::Stdout => ProcessOutputChunk::Stdout(chunk.bytes),
            StreamKind::Stderr => ProcessOutputChunk::Stderr(chunk.bytes),
        }),
        AsyncProcessSessionEvent::StreamEof(stream) => {
            ProcessOutputEvent::Completion(match stream {
                StreamKind::Stdout => ProcessOutputCompletion::StdoutEof,
                StreamKind::Stderr => ProcessOutputCompletion::StderrEof,
            })
        }
        AsyncProcessSessionEvent::StreamAbandoned(stream) => {
            ProcessOutputEvent::Completion(match stream {
                StreamKind::Stdout => ProcessOutputCompletion::StdoutAbandoned,
                StreamKind::Stderr => ProcessOutputCompletion::StderrAbandoned,
            })
        }
        AsyncProcessSessionEvent::StreamError {
            stream,
            kind,
            message,
            raw_os_error,
        } => {
            let fault = ProcessOutputFault::new(kind, message, raw_os_error);
            ProcessOutputEvent::Completion(match stream {
                StreamKind::Stdout => ProcessOutputCompletion::StdoutError(fault),
                StreamKind::Stderr => ProcessOutputCompletion::StderrError(fault),
            })
        }
    }
}

#[cfg(unix)]
fn native_exit(status: ExitStatus) -> ProcessSessionExit {
    use std::os::unix::process::ExitStatusExt as _;

    let exit_code = status.code();
    let signal = status.signal();
    ProcessSessionExit::from_native(exit_code, signal, status.into_raw() as u32)
}

#[cfg(windows)]
fn native_exit(status: ExitStatus) -> ProcessSessionExit {
    let exit_code = status.code();
    // Preserve every bit of Windows' DWORD exit result even when its signed
    // i32 convenience representation is negative.
    let native_status = exit_code.unwrap_or(1) as u32;
    ProcessSessionExit::from_native(exit_code, None, native_status)
}

fn stdio(mode: StreamMode) -> AsyncStdio {
    match mode {
        StreamMode::Inherit => AsyncStdio::Inherit,
        StreamMode::Piped => AsyncStdio::Piped,
        StreamMode::Null => AsyncStdio::Null,
    }
}

fn std_command(spec: SpawnSpec) -> (std::process::Command, bool, crate::ProcessPriority) {
    let SpawnSpec {
        program,
        args,
        current_dir,
        env,
        clear_env,
        stdin: _,
        stdout: _,
        stderr: _,
        create_process_group: _,
        kill_when_owner_dies,
        priority,
    } = spec;
    let mut command = std::process::Command::new(program);
    command.args(args);
    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }
    if clear_env {
        command.env_clear();
    }
    command.envs(env);
    (command, kill_when_owner_dies, priority)
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
        ProcessError::AlreadyStarted => BoundedProcessError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "bounded command was already started",
        )),
        ProcessError::NotRunning | ProcessError::StdinUnavailable => {
            BoundedProcessError::Io(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "bounded command is no longer running",
            ))
        }
        ProcessError::RuntimeContext => BoundedProcessError::Io(io::Error::other(
            "bounded command requires an unavailable runtime context",
        )),
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

#[cfg(test)]
mod tests {
    use super::output_event;
    use crate::{ProcessOutputCompletion, ProcessOutputEvent};
    use running_process::{AsyncProcessSessionEvent, StreamKind};

    #[test]
    fn stream_fault_preserves_kind_message_and_native_error() {
        let event = output_event(AsyncProcessSessionEvent::StreamError {
            stream: StreamKind::Stderr,
            kind: std::io::ErrorKind::PermissionDenied,
            message: "access denied by fixture".to_owned(),
            raw_os_error: Some(5),
        });

        let ProcessOutputEvent::Completion(ProcessOutputCompletion::StderrError(fault)) = event
        else {
            panic!("stderr fault must remain a terminal stderr completion");
        };
        assert_eq!(fault.kind(), std::io::ErrorKind::PermissionDenied);
        assert_eq!(fault.message(), "access denied by fixture");
        assert_eq!(fault.raw_os_error(), Some(5));
    }
}
