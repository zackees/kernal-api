//! Private, compile-time-selected Component hash candidate for the experiment.
use super::OperationError;

mod bindings {
    wit_bindgen::generate!({ path: "src/guest_component_hash.wit", world: "hash-client" });
}
use bindings::kernal::hash_experiment::hashes;

fn error(value: hashes::Error) -> OperationError {
    match value {
        hashes::Error::Rejected => OperationError::Rejected,
        hashes::Error::Failed => OperationError::Failed,
    }
}

pub(super) struct Blake3Hasher {
    inner: hashes::Hasher,
}

impl Blake3Hasher {
    pub(super) async fn new() -> Result<Self, OperationError> {
        Ok(Self {
            inner: hashes::create().map_err(error)?,
        })
    }

    pub(super) async fn update(&mut self, bytes: &[u8]) -> Result<(), OperationError> {
        if bytes.len() > 64 * 1024 {
            return Err(OperationError::Rejected);
        }
        self.inner.update(bytes).map_err(error)
    }

    pub(super) async fn finalize(self) -> Result<[u8; 32], OperationError> {
        hashes::finish(self.inner)
            .map_err(error)?
            .try_into()
            .map_err(|_| OperationError::Failed)
    }
}
