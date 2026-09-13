//! Facade-owned asynchronous reader/writer locking.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// An async reader/writer lock with FIFO writer preference supplied by the
/// private runtime implementation.
#[derive(Debug)]
pub struct RwLock<T> {
    inner: Arc<tokio::sync::RwLock<T>>,
}

impl<T> RwLock<T> {
    /// Create an unlocked value.
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(tokio::sync::RwLock::new(value)),
        }
    }
    /// Acquire an owned read guard. Cancelling this future removes its waiter.
    pub async fn read_owned(self: Arc<Self>) -> OwnedRwLockReadGuard<T> {
        OwnedRwLockReadGuard {
            inner: Arc::clone(&self.inner).read_owned().await,
        }
    }
    /// Acquire an owned write guard. Later readers queue behind an earlier writer.
    pub async fn write_owned(self: Arc<Self>) -> OwnedRwLockWriteGuard<T> {
        OwnedRwLockWriteGuard {
            inner: Arc::clone(&self.inner).write_owned().await,
        }
    }
    /// Acquire a borrowed read guard, queuing behind earlier writers.
    pub async fn read(&self) -> RwLockReadGuard<'_, T> {
        RwLockReadGuard {
            inner: self.inner.read().await,
        }
    }
    /// Acquire a borrowed exclusive guard. Cancellation loses queue position.
    pub async fn write(&self) -> RwLockWriteGuard<'_, T> {
        RwLockWriteGuard {
            inner: self.inner.write().await,
        }
    }
    /// Block this thread until a read guard is available.
    ///
    /// Panics inside an asynchronous execution context. Use only from a
    /// synchronous thread or the runtime's blocking-work lane.
    pub fn blocking_read(&self) -> RwLockReadGuard<'_, T> {
        RwLockReadGuard {
            inner: self.inner.blocking_read(),
        }
    }
    /// Block this thread until an exclusive guard is available.
    ///
    /// Panics inside an asynchronous execution context. Use only from a
    /// synchronous thread or the runtime's blocking-work lane.
    pub fn blocking_write(&self) -> RwLockWriteGuard<'_, T> {
        RwLockWriteGuard {
            inner: self.inner.blocking_write(),
        }
    }
    /// Attempt an owned read without bypassing an already queued writer.
    pub fn try_read_owned(self: Arc<Self>) -> Result<OwnedRwLockReadGuard<T>, RwLockTryLockError> {
        Arc::clone(&self.inner)
            .try_read_owned()
            .map(|inner| OwnedRwLockReadGuard { inner })
            .map_err(|_| RwLockTryLockError)
    }
}
impl<T: Default> Default for RwLock<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Owned read lease returned by [`RwLock::read_owned`].
pub struct OwnedRwLockReadGuard<T> {
    inner: tokio::sync::OwnedRwLockReadGuard<T>,
}
impl<T> Deref for OwnedRwLockReadGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
/// Owned write lease returned by [`RwLock::write_owned`].
pub struct OwnedRwLockWriteGuard<T> {
    inner: tokio::sync::OwnedRwLockWriteGuard<T>,
}
impl<T> Deref for OwnedRwLockWriteGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
impl<T> DerefMut for OwnedRwLockWriteGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}
pub struct RwLockReadGuard<'a, T> {
    inner: tokio::sync::RwLockReadGuard<'a, T>,
}
impl<T> Deref for RwLockReadGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
pub struct RwLockWriteGuard<'a, T> {
    inner: tokio::sync::RwLockWriteGuard<'a, T>,
}
impl<T> Deref for RwLockWriteGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
impl<T> DerefMut for RwLockWriteGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RwLockTryLockError;
impl std::fmt::Display for RwLockTryLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rwlock is busy")
    }
}
impl std::error::Error for RwLockTryLockError {}

#[cfg(test)]
mod tests {
    use super::RwLock;

    #[test]
    fn blocking_write_mutates_from_a_synchronous_context() {
        let lock = RwLock::new(1_u8);
        *lock.blocking_write() = 2;
        assert_eq!(*lock.blocking_read(), 2);
    }
}
