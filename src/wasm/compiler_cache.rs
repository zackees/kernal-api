//! Private zccache-backed retention for admitted compiler outputs.
//!
//! The guest receives only a fixed request key and a hit/miss answer. Cache
//! roots, namespaces, and artifact bytes stay on the host side of the facade.

#[cfg(test)]
use std::path::Path;

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
