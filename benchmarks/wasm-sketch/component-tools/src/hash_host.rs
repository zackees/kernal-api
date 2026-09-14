//! Private Component hash transport with bounded scalar framing.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use wasmtime::component::{
    Accessor, Linker, Resource, ResourceTable, Source, StreamConsumer, StreamResult,
};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../../../src/guest_component_hash.wit",
        world: "hash-client",
        imports: { default: trappable },
        with: { "kernal:hash-experiment/hashes.hasher": super::Hasher },
    });
}
use bindings::kernal::hash_experiment::hashes::{self, Error};

pub struct Hasher {
    inner: kernal_api::hash::Blake3Hasher,
    live: Arc<AtomicUsize>,
}
impl Drop for Hasher {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
pub(super) struct State {
    table: ResourceTable,
    pub(super) live: Arc<AtomicUsize>,
}

pub(super) fn add_to_linker<T: Send + 'static>(
    linker: &mut Linker<T>,
    get: fn(&mut T) -> &mut State,
) -> wasmtime::Result<()> {
    bindings::HashClient::add_to_linker::<T, wasmtime::component::HasSelf<State>>(linker, get)
}

impl hashes::Host for State {
    fn create(&mut self) -> wasmtime::Result<Result<Resource<Hasher>, Error>> {
        // Match Core's 64-resource ceiling for the hash-only comparison. The
        // separate legacy blob probe does not yet share this resource budget.
        if self.live.load(Ordering::SeqCst) >= 64 {
            return Ok(Err(Error::Rejected));
        }
        self.live.fetch_add(1, Ordering::SeqCst);
        let hash = Hasher {
            inner: kernal_api::hash::Blake3Hasher::new(),
            live: self.live.clone(),
        };
        Ok(Ok(self.table.push(hash)?))
    }

    fn finish(&mut self, hash: Resource<Hasher>) -> wasmtime::Result<Result<Vec<u8>, Error>> {
        let hash = self.table.delete(hash)?;
        Ok(Ok(hash.inner.finalize().as_bytes().to_vec()))
    }
}

impl hashes::HostHasher for State {
    fn drop(&mut self, hash: Resource<Hasher>) -> wasmtime::Result<()> {
        self.table.delete(hash)?;
        Ok(())
    }
}

const MAX_HASH_UPDATE_BYTES: usize = 64 * 1024;
const HASH_INPUT_CHUNK_BYTES: usize = 256;

struct HashInputConsumer {
    bytes: Vec<u8>,
    result: Option<kernal_api::async_engine::OneshotSender<Result<Vec<u8>, ()>>>,
}

enum HashInputFrame {
    Continue,
    Complete,
    Rejected,
}

impl HashInputConsumer {
    fn reject(&mut self) {
        if let Some(result) = self.result.take() {
            let _ = result.send(Err(()));
        }
    }

    fn accept(&mut self) {
        if let Some(result) = self.result.take() {
            let _ = result.send(Ok(std::mem::take(&mut self.bytes)));
        }
    }

    fn receive(&mut self, chunk: hashes::InputChunk) -> HashInputFrame {
        let used = usize::from(chunk.used);
        if used > HASH_INPUT_CHUNK_BYTES
            || (!chunk.last && used == 0)
            || self.bytes.len() + used > MAX_HASH_UPDATE_BYTES
        {
            self.reject();
            return HashInputFrame::Rejected;
        }
        let words = [
            chunk.word_0, chunk.word_1, chunk.word_2, chunk.word_3, chunk.word_4, chunk.word_5,
            chunk.word_6, chunk.word_7, chunk.word_8, chunk.word_9, chunk.word_10, chunk.word_11,
            chunk.word_12, chunk.word_13, chunk.word_14, chunk.word_15, chunk.word_16,
            chunk.word_17, chunk.word_18, chunk.word_19, chunk.word_20, chunk.word_21,
            chunk.word_22, chunk.word_23, chunk.word_24, chunk.word_25, chunk.word_26,
            chunk.word_27, chunk.word_28, chunk.word_29, chunk.word_30, chunk.word_31,
        ];
        let mut remaining = used;
        for word in words {
            let bytes = word.to_le_bytes();
            let count = remaining.min(bytes.len());
            self.bytes.extend_from_slice(&bytes[..count]);
            remaining -= count;
            if remaining == 0 {
                break;
            }
        }
        if chunk.last {
            return HashInputFrame::Complete;
        }
        HashInputFrame::Continue
    }
}

impl Drop for HashInputConsumer {
    fn drop(&mut self) {
        self.reject();
    }
}

impl<T: Send + 'static> StreamConsumer<T> for HashInputConsumer {
    type Item = hashes::InputChunk;

    fn poll_consume(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        mut store: wasmtime::StoreContextMut<T>,
        mut source: Source<'_, Self::Item>,
        finish: bool,
    ) -> std::task::Poll<wasmtime::Result<StreamResult>> {
        if finish {
            self.reject();
            return std::task::Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if source.remaining(&mut store) > MAX_HASH_UPDATE_BYTES - self.bytes.len() + 1 {
            self.reject();
            return std::task::Poll::Ready(Ok(StreamResult::Dropped));
        }
        while source.remaining(&mut store) != 0 {
            let mut chunk = Vec::with_capacity(1);
            source.read(&mut store, &mut chunk)?;
            let chunk = chunk.pop().expect("source reported a frame");
            match self.receive(chunk) {
                HashInputFrame::Continue => {}
                HashInputFrame::Complete => {
                    if source.remaining(&mut store) != 0 {
                        self.reject();
                    } else {
                        self.accept();
                    }
                    return std::task::Poll::Ready(Ok(StreamResult::Dropped));
                }
                HashInputFrame::Rejected => {
                    if source.remaining(&mut store) != 0 {
                        self.reject();
                    }
                    return std::task::Poll::Ready(Ok(StreamResult::Dropped));
                }
            }
        }
        std::task::Poll::Ready(Ok(StreamResult::Completed))
    }
}

impl hashes::HostHasherWithStore for wasmtime::component::HasSelf<State> {
    async fn update<T: Send>(
        accessor: &Accessor<T, Self>,
        hash: Resource<Hasher>,
        chunks: wasmtime::component::StreamReader<hashes::InputChunk>,
    ) -> wasmtime::Result<Result<(), Error>> {
        let (sender, receiver) = kernal_api::async_engine::oneshot_channel();
        let start: Result<(), Error> = accessor.with(|mut access| {
            access.get().table.get(&hash)?;
            chunks.pipe(
                access,
                HashInputConsumer {
                    bytes: Vec::with_capacity(MAX_HASH_UPDATE_BYTES),
                    result: Some(sender),
                },
            )?;
            Ok::<Result<(), Error>, wasmtime::Error>(Ok(()))
        })?;
        start?;
        let bytes = match receiver.await {
            Ok(Ok(bytes)) => bytes,
            Ok(Err(())) | Err(_) => return Ok(Err(Error::Rejected)),
        };
        accessor.with(|mut access| {
            access.get().table.get_mut(&hash)?.inner.update(&bytes);
            Ok(Ok(()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashes::{Host, HostHasher};

    #[test]
    fn quota_and_drop_reclaims() {
        let mut state = State::default();
        let hash = state.create().unwrap().unwrap();
        let others: Vec<_> = (0..63).map(|_| state.create().unwrap().unwrap()).collect();
        assert!(matches!(state.create().unwrap(), Err(Error::Rejected)));
        for other in others {
            HostHasher::drop(&mut state, other).unwrap();
        }
        HostHasher::drop(&mut state, hash).unwrap();
        assert!(state.table.is_empty());
        assert_eq!(state.live.load(Ordering::SeqCst), 0);
        let hash = state.create().unwrap().unwrap();
        HostHasher::drop(&mut state, hash).unwrap();
        assert!(state.table.is_empty());
        assert_eq!(state.live.load(Ordering::SeqCst), 0);
    }
}
