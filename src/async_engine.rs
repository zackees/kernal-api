//! The canonical asynchronous engine for every `kernal-api` client.
//!
//! Every public value in this module is owned by `kernal-api`. The current
//! implementation is Tokio, but no Tokio type, trait, module, or macro is
//! re-exported across the facade.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

/// Current engine implementation, retained for diagnostics and bug reports.
pub const BACKEND_NAME: &str = "tokio";
/// Exact backend version selected by this `kernal-api` release.
pub const BACKEND_VERSION: &str = "1.53.1";

/// Handle to a task launched on the shared async engine.
#[derive(Debug)]
pub struct Task<T> {
    inner: tokio::task::JoinHandle<T>,
}

impl<T> Task<T> {
    /// Request cancellation. Cancellation completes at the next yield point.
    pub fn cancel(&self) {
        self.inner.abort();
    }

    /// Whether the task has completed.
    pub fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }
}

impl<T> Future for Task<T> {
    type Output = Result<T, TaskError>;

    fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.inner)
            .poll(context)
            .map(|result| result.map_err(TaskError::from_backend))
    }
}

/// Failure returned while joining an async task.
#[derive(Debug)]
pub struct TaskError {
    inner: tokio::task::JoinError,
}

impl TaskError {
    fn from_backend(inner: tokio::task::JoinError) -> Self {
        Self { inner }
    }

    /// Whether cancellation ended the task.
    pub fn is_cancelled(&self) -> bool {
        self.inner.is_cancelled()
    }

    /// Whether a panic ended the task.
    pub fn is_panic(&self) -> bool {
        self.inner.is_panic()
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.inner.fmt(formatter)
    }
}

impl std::error::Error for TaskError {}

/// Owned asynchronous runtime.
pub struct Runtime {
    inner: tokio::runtime::Runtime,
}

impl Runtime {
    /// Run one future to completion on this runtime.
    pub fn run<F: Future>(&self, future: F) -> F::Output {
        self.inner.block_on(future)
    }

    /// Obtain a clonable handle for this runtime.
    pub fn handle(&self) -> RuntimeHandle {
        RuntimeHandle {
            inner: self.inner.handle().clone(),
        }
    }
}

/// Builder for an owned asynchronous runtime.
pub struct RuntimeBuilder {
    inner: tokio::runtime::Builder,
}

impl RuntimeBuilder {
    /// Build a runtime that executes tasks on the calling thread.
    pub fn current_thread() -> Self {
        Self {
            inner: tokio::runtime::Builder::new_current_thread(),
        }
    }

    /// Build a runtime backed by a worker pool.
    pub fn multi_thread() -> Self {
        Self {
            inner: tokio::runtime::Builder::new_multi_thread(),
        }
    }

    /// Enable the engine's I/O, time, and signal drivers.
    pub fn enable_all(mut self) -> Self {
        self.inner.enable_all();
        self
    }

    /// Set the number of async worker threads.
    pub fn worker_threads(mut self, count: usize) -> Self {
        self.inner.worker_threads(count);
        self
    }

    /// Set the runtime worker thread name.
    pub fn thread_name(mut self, name: impl Into<String>) -> Self {
        self.inner.thread_name(name.into());
        self
    }

    /// Create the configured runtime.
    pub fn build(mut self) -> std::io::Result<Runtime> {
        self.inner.build().map(|inner| Runtime { inner })
    }
}

/// Clonable handle to a running asynchronous runtime.
#[derive(Clone)]
pub struct RuntimeHandle {
    inner: tokio::runtime::Handle,
}

impl RuntimeHandle {
    /// Obtain the current runtime handle.
    pub fn current() -> Result<Self, NoRuntime> {
        tokio::runtime::Handle::try_current()
            .map(|inner| Self { inner })
            .map_err(|_| NoRuntime)
    }

    /// Launch a task on this runtime.
    pub fn launch<F>(&self, future: F) -> Task<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        Task {
            inner: self.inner.spawn(future),
        }
    }
}

/// No async runtime is active on this thread.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("no kernal-api async runtime is active on this thread")]
pub struct NoRuntime;

/// Launch a `Send` task on the current runtime.
pub fn launch<F>(future: F) -> Task<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    Task {
        inner: tokio::spawn(future),
    }
}

/// Run blocking work without occupying an async worker thread.
pub fn launch_blocking<F, R>(operation: F) -> Task<R>
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    Task {
        inner: tokio::task::spawn_blocking(operation),
    }
}

/// Yield execution back to the async scheduler once.
pub async fn yield_now() {
    tokio::task::yield_now().await;
}

/// Wait for a duration without blocking an async worker.
pub async fn sleep(duration: Duration) {
    tokio::time::sleep(duration).await;
}

/// Run `future` until it completes or the deadline expires.
pub async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, DeadlineElapsed> {
    tokio::time::timeout(duration, future)
        .await
        .map_err(|_| DeadlineElapsed)
}

/// The configured deadline elapsed before an operation completed.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("the kernal-api async operation exceeded its deadline")]
pub struct DeadlineElapsed;

#[cfg(feature = "tokio-console")]
pub use crate::runtime::{DiagnosticsConfig, DiagnosticsInstallError};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_runtime_launches_owned_tasks() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let answer = runtime.run(async { launch(async { 42_u8 }).await.unwrap() });
        assert_eq!(answer, 42);
    }

    #[test]
    fn deadline_error_is_facade_owned() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let result = runtime.run(timeout(Duration::ZERO, std::future::pending::<()>()));
        assert!(result.is_err());
    }
}
