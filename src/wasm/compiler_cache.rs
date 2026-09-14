//! Private zccache-backed retention for admitted compiler outputs.
//!
//! The guest receives only a fixed request key and a hit/miss answer. Cache
//! roots, namespaces, and artifact bytes stay on the host side of the facade.

#[cfg(any(test, feature = "wasm-sketch-host"))]
use std::path::Path;

#[cfg(feature = "wasm-sketch-host")]
use super::operations::OperationHub;

use zccache_artifact::{Key, KvStore};

const COMPILER_ARTIFACT_NAMESPACE: &str = "kernal-compiler-v1";

#[derive(Clone)]
pub(super) struct CompilerArtifactStore {
    store: KvStore,
}

impl CompilerArtifactStore {
    #[cfg(test)]
    pub(super) fn open(root: &Path) -> Result<Self, ()> {
        KvStore::open(root).map(|store| Self { store }).map_err(|_| ())
    }

    pub(super) fn get(&self, key: [u8; 32]) -> Result<Option<Vec<u8>>, ()> {
        self.store
            .get(COMPILER_ARTIFACT_NAMESPACE, &Key(key))
            .map_err(|_| ())
    }

    pub(super) fn put(&self, key: [u8; 32], bytes: &[u8]) -> Result<(), ()> {
        self.store
            .put(COMPILER_ARTIFACT_NAMESPACE, &Key(key), bytes)
            .map(|_| ())
            .map_err(|_| ())
    }
}

/// Read a private compiler artifact and restore a hit through the one exact
/// host-owned output path.  The guest never receives the cache bytes or the
/// destination capability, regardless of which private binding experiment
/// asked for the cache decision.
#[cfg(feature = "wasm-sketch-host")]
pub(super) fn restore_cached_output_if_hit(
    store: &CompilerArtifactStore,
    hub: &OperationHub,
    owner: u64,
    key: [u8; 32],
    destination: &Path,
) -> Result<bool, CacheRestoreError> {
    let Some(bytes) = store.get(key).map_err(|_| CacheRestoreError::Cache)? else {
        return Ok(false);
    };
    hub.restore_cached_output(owner, destination, &bytes)
        .map_err(|_| CacheRestoreError::Output)?;
    Ok(true)
}

#[cfg(feature = "wasm-sketch-host")]
#[derive(Debug)]
pub(super) enum CacheRestoreError {
    Cache,
    Output,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persists_a_keyed_artifact_through_the_real_zccache_store() {
        let root = tempfile::tempdir().unwrap();
        let store = CompilerArtifactStore::open(root.path()).unwrap();
        let key = [0x5a; 32];
        assert_eq!(store.get(key).unwrap(), None);
        store.put(key, b"admitted artifact").unwrap();
        assert_eq!(store.get(key).unwrap().as_deref(), Some(b"admitted artifact".as_slice()));
    }
}
