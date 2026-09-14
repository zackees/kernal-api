//! Private Component hash transport; canonical list lifting precedes size checks.
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use wasmtime::component::{Linker, Resource, ResourceTable};

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

pub(super) fn add_to_linker<T: 'static>(
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
    fn update(
        &mut self,
        hash: Resource<Hasher>,
        bytes: Vec<u8>,
    ) -> wasmtime::Result<Result<(), Error>> {
        if bytes.len() > 64 * 1024 {
            return Ok(Err(Error::Rejected));
        }
        self.table.get_mut(&hash)?.inner.update(&bytes);
        Ok(Ok(()))
    }

    fn drop(&mut self, hash: Resource<Hasher>) -> wasmtime::Result<()> {
        self.table.delete(hash)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashes::{Host, HostHasher};

    #[test]
    fn quota_and_oversize_reject_before_mutation_and_drop_reclaims() {
        let mut state = State::default();
        let hash = state.create().unwrap().unwrap();
        let others: Vec<_> = (0..63).map(|_| state.create().unwrap().unwrap()).collect();
        assert!(matches!(state.create().unwrap(), Err(Error::Rejected)));
        for other in others {
            HostHasher::drop(&mut state, other).unwrap();
        }
        assert_eq!(
            state
                .update(Resource::new_borrow(hash.rep()), vec![0; 65537])
                .unwrap(),
            Err(Error::Rejected)
        );
        let digest = state.finish(hash).unwrap().unwrap();
        assert_eq!(
            digest,
            kernal_api::hash::Blake3Hasher::new().finalize().as_bytes()
        );
        assert!(state.table.is_empty());
        assert_eq!(state.live.load(Ordering::SeqCst), 0);
        let hash = state.create().unwrap().unwrap();
        HostHasher::drop(&mut state, hash).unwrap();
        assert!(state.table.is_empty());
        assert_eq!(state.live.load(Ordering::SeqCst), 0);
    }
}
