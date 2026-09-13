//! Asynchronous exclusive locking without exposing runtime implementation types.

use std::ops::{Deref, DerefMut};
use std::sync::Arc;

/// A non-poisoning asynchronous mutex. Waiters acquire in FIFO polling order.
/// Cancelling an acquisition loses its queue position, without acquiring a guard.
#[derive(Debug)]
pub struct Mutex<T> {
    inner: Arc<tokio::sync::Mutex<T>>,
}

impl<T> Mutex<T> {
    /// Create an unlocked value.
    pub fn new(value: T) -> Self {
        Self {
            inner: Arc::new(tokio::sync::Mutex::new(value)),
        }
    }

    /// Acquire a guard borrowing this mutex.
    pub async fn lock(&self) -> MutexGuard<'_, T> {
        MutexGuard {
            inner: self.inner.lock().await,
        }
    }

    /// Acquire a guard that retains the lock storage independently of this facade.
    pub async fn lock_owned(self: Arc<Self>) -> OwnedMutexGuard<T> {
        OwnedMutexGuard {
            inner: Arc::clone(&self.inner).lock_owned().await,
        }
    }

    /// Acquire immediately, or report contention without joining the queue.
    pub fn try_lock(&self) -> Result<MutexGuard<'_, T>, MutexTryLockError> {
        self.inner
            .try_lock()
            .map(|inner| MutexGuard { inner })
            .map_err(|_| MutexTryLockError)
    }

    /// Acquire an owned guard immediately without bypassing queued waiters.
    pub fn try_lock_owned(self: Arc<Self>) -> Result<OwnedMutexGuard<T>, MutexTryLockError> {
        Arc::clone(&self.inner)
            .try_lock_owned()
            .map(|inner| OwnedMutexGuard { inner })
            .map_err(|_| MutexTryLockError)
    }

    /// Block the calling thread until it acquires the lock.
    ///
    /// Panics inside asynchronous execution. Use a synchronous thread or the
    /// runtime's blocking-work lane, never an asynchronous task's polling thread.
    pub fn blocking_lock(&self) -> MutexGuard<'_, T> {
        MutexGuard {
            inner: self.inner.blocking_lock(),
        }
    }
}

impl<T: Default> Default for Mutex<T> {
    fn default() -> Self {
        Self::new(T::default())
    }
}

/// Borrowed exclusive lease. Dropping it releases the lock.
pub struct MutexGuard<'a, T> {
    inner: tokio::sync::MutexGuard<'a, T>,
}
impl<T> Deref for MutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
impl<T> DerefMut for MutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

/// Owned exclusive lease, valid even after all facade handles are dropped.
pub struct OwnedMutexGuard<T> {
    inner: tokio::sync::OwnedMutexGuard<T>,
}
impl<T> Deref for OwnedMutexGuard<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}
impl<T> DerefMut for OwnedMutexGuard<T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

/// Immediate acquisition failed because the mutex is unavailable.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MutexTryLockError;
impl std::fmt::Display for MutexTryLockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("mutex is busy")
    }
}
impl std::error::Error for MutexTryLockError {}
