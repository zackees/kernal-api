//! Private Component transport for the same semantic compiler facade.
use super::{bindings, OperationError};

mod wire {
    wit_bindgen::generate!({ path: "src/guest_component_compiler.wit", world: "compiler-client" });
}
use wire::kernal::compiler_experiment::compilers;

fn error(value: compilers::Error) -> OperationError {
    match value {
        compilers::Error::Rejected => OperationError::Rejected,
        compilers::Error::Cancelled => OperationError::Cancelled,
        compilers::Error::Closed => OperationError::Closed,
        compilers::Error::Failed => OperationError::Failed,
        compilers::Error::TimedOut => OperationError::TimedOut,
    }
}

pub(super) struct CompilerGrant {
    inner: compilers::Grant,
}
pub(super) struct CompilerProcess {
    inner: compilers::Process,
}

impl CompilerGrant {
    pub(super) fn granted() -> Result<Option<Self>, OperationError> {
        Ok(compilers::granted()
            .map_err(error)?
            .map(|inner| Self { inner }))
    }
    pub(super) async fn spawn(self) -> Result<CompilerProcess, OperationError> {
        Ok(CompilerProcess {
            inner: compilers::spawn(self.inner).await.map_err(error)?,
        })
    }
}

impl CompilerProcess {
    pub(super) async fn read_output(
        &mut self,
        destination: &mut [u8],
    ) -> Result<bindings::CompilerOutputEvent, OperationError> {
        if destination.len() < 65536 {
            return Err(OperationError::Rejected);
        }
        let output = compilers::read(&self.inner).await.map_err(error)?;
        let data = output.copy().map_err(error)?;
        let count = data.bytes.len();
        if count > 65536 {
            return Err(OperationError::Failed);
        }
        use bindings::CompilerOutputEvent as Event;
        use compilers::OutputTag as Tag;
        let event = match data.tag {
            Tag::Stdout => Event::Stdout(count),
            Tag::Stderr => Event::Stderr(count),
            Tag::StdoutEof if count == 0 => Event::StdoutEof,
            Tag::StderrEof if count == 0 => Event::StderrEof,
            Tag::StdoutAbandoned if count == 0 => Event::StdoutAbandoned,
            Tag::StderrAbandoned if count == 0 => Event::StderrAbandoned,
            Tag::StdoutError if count == 0 => Event::StdoutError,
            Tag::StderrError if count == 0 => Event::StderrError,
            Tag::Exhausted if count == 0 => Event::Exhausted,
            _ => return Err(OperationError::Failed),
        };
        destination[..count].copy_from_slice(&data.bytes);
        Ok(event)
    }
    pub(super) async fn wait(&self) -> Result<bindings::CompilerExit, OperationError> {
        let exit = compilers::wait(&self.inner).await.map_err(error)?;
        Ok(bindings::CompilerExit {
            code: exit.code,
            success: exit.success,
        })
    }
    pub(super) async fn close(self) -> Result<(), OperationError> {
        compilers::close(self.inner).await.map_err(error)
    }
}
