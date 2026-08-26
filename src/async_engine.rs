//! The canonical asynchronous engine for every `kernal-api` client.
//!
//! Every public value in this module is owned by `kernal-api`. The current
//! implementation is Tokio, but no Tokio type, trait, module, or macro is
//! re-exported across the facade.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::sync::Notify;

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

/// An absolute deadline that can be shared across composed async operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deadline {
    at: tokio::time::Instant,
}

impl Deadline {
    /// Create a deadline `duration` from now.
    pub fn after(duration: Duration) -> Self {
        Self {
            at: tokio::time::Instant::now() + duration,
        }
    }

    /// Return the remaining budget, clamped to zero after expiry.
    pub fn remaining(self) -> Duration {
        self.at
            .checked_duration_since(tokio::time::Instant::now())
            .unwrap_or(Duration::ZERO)
    }

    /// Whether this deadline has elapsed.
    pub fn is_elapsed(self) -> bool {
        self.remaining().is_zero()
    }
}

/// Run `future` until it completes or the deadline expires.
pub async fn timeout<F: Future>(
    duration: Duration,
    future: F,
) -> Result<F::Output, DeadlineElapsed> {
    timeout_at(Deadline::after(duration), future).await
}

/// Run `future` until an already-composed deadline expires.
pub async fn timeout_at<F: Future>(
    deadline: Deadline,
    future: F,
) -> Result<F::Output, DeadlineElapsed> {
    tokio::time::timeout_at(deadline.at, future)
        .await
        .map_err(|_| DeadlineElapsed)
}

/// The configured deadline elapsed before an operation completed.
#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("the kernal-api async operation exceeded its deadline")]
pub struct DeadlineElapsed;

/// Source that can request cancellation of operations sharing its token.
#[derive(Clone, Debug)]
pub struct CancellationSource {
    state: Arc<CancellationState>,
}

#[derive(Debug)]
struct CancellationState {
    cancelled: AtomicBool,
    notify: Notify,
    // This is a deterministic regression seam for the Notify registration
    // race. It is excluded from production artifacts.
    #[cfg(test)]
    cancel_after_waiter_check: AtomicBool,
}

/// Cloneable cancellation capability for one or more async operations.
///
/// The token is an operation boundary, not a task handle: cancelling it never
/// aborts unrelated work on the runtime. Operations opt in through
/// [`cancellable`] or by awaiting [`CancellationToken::cancelled`].
#[derive(Clone, Debug)]
pub struct CancellationToken {
    state: Arc<CancellationState>,
}

impl CancellationSource {
    /// Create a new, initially active cancellation domain.
    pub fn new() -> Self {
        Self {
            state: Arc::new(CancellationState {
                cancelled: AtomicBool::new(false),
                notify: Notify::new(),
                #[cfg(test)]
                cancel_after_waiter_check: AtomicBool::new(false),
            }),
        }
    }

    /// Return a token that can observe this source's cancellation request.
    pub fn token(&self) -> CancellationToken {
        CancellationToken {
            state: Arc::clone(&self.state),
        }
    }

    /// Request cancellation for every operation using a token from this source.
    ///
    /// This is idempotent. The request is cooperative: an operation observes it
    /// at an await point or when it explicitly checks its token.
    pub fn cancel(&self) {
        if !self.state.cancelled.swap(true, Ordering::AcqRel) {
            self.state.notify.notify_waiters();
        }
    }

    /// Whether this source has requested cancellation.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancellationSource {
    fn default() -> Self {
        Self::new()
    }
}

impl CancellationToken {
    /// Whether cancellation has been requested.
    pub fn is_cancelled(&self) -> bool {
        self.state.cancelled.load(Ordering::Acquire)
    }

    /// Wait until cancellation is requested.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
            // Register before the second state observation. `notify_waiters`
            // intentionally retains no permit, so without this a cancellation
            // between constructing `Notified` and its first poll could strand
            // this waiter forever.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            #[cfg(test)]
            if self
                .state
                .cancel_after_waiter_check
                .swap(false, Ordering::AcqRel)
            {
                self.state.cancelled.store(true, Ordering::Release);
                self.state.notify.notify_waiters();
            }
            notified.await;
        }
    }
}

/// Run an operation until it completes or `token` requests cancellation.
pub async fn cancellable<F>(token: &CancellationToken, future: F) -> Result<F::Output, Cancelled>
where
    F: Future,
{
    tokio::select! {
        biased;
        () = token.cancelled() => Err(Cancelled),
        output = future => Ok(output),
    }
}

/// An operation was cancelled through a kernal-api cancellation token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the kernal-api async operation was cancelled")]
pub struct Cancelled;

/// Run a connection-establishment operation until it completes or its
/// connection deadline expires.
///
/// This deliberately differs from [`progress_timeout`]: it is for the period
/// before a connection exists and therefore cannot report transfer progress.
pub async fn connection_timeout<F>(
    duration: Duration,
    future: F,
) -> Result<F::Output, ConnectionDeadlineElapsed>
where
    F: Future,
{
    connection_until(Deadline::after(duration), future).await
}

/// Run a connection-establishment operation within an already-composed
/// deadline.
pub async fn connection_until<F>(
    deadline: Deadline,
    future: F,
) -> Result<F::Output, ConnectionDeadlineElapsed>
where
    F: Future,
{
    tokio::time::timeout_at(deadline.at, future)
        .await
        .map_err(|_| ConnectionDeadlineElapsed)
}

/// The configured connection-establishment deadline elapsed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the kernal-api connection deadline elapsed before the operation completed")]
pub struct ConnectionDeadlineElapsed;

/// Reports caller-observable progress for one transfer or streaming operation.
///
/// Clones of this reporter, and every [`ProgressWatch`] made from it, share one
/// intentionally common progress domain. Use a new reporter for independent
/// transfers so activity on one cannot extend another's idle budget.
#[derive(Clone, Debug)]
pub struct ProgressReporter {
    state: Arc<ProgressState>,
}

#[derive(Debug)]
struct ProgressState {
    last_progress: Mutex<tokio::time::Instant>,
    notify: Notify,
}

/// Observes progress reported through a paired [`ProgressReporter`].
///
/// Clones observe the same intentionally shared progress domain as the
/// reporter and its other watches.
#[derive(Clone, Debug)]
pub struct ProgressWatch {
    state: Arc<ProgressState>,
}

impl ProgressReporter {
    /// Start a transfer-progress domain. Its initial idle window starts now.
    pub fn new() -> Self {
        Self {
            state: Arc::new(ProgressState {
                last_progress: Mutex::new(tokio::time::Instant::now()),
                notify: Notify::new(),
            }),
        }
    }

    /// Obtain a watch capability for use with [`progress_timeout`].
    pub fn watch(&self) -> ProgressWatch {
        ProgressWatch {
            state: Arc::clone(&self.state),
        }
    }

    /// Record that the caller observed forward progress.
    ///
    /// Call this only after a meaningful byte, frame, or other contractually
    /// defined unit has been consumed or produced. Merely polling an idle
    /// transport must not reset the progress deadline.
    pub fn report_progress(&self) {
        *self
            .state
            .last_progress
            .lock()
            .expect("progress state mutex must not be poisoned") = tokio::time::Instant::now();
        self.state.notify.notify_waiters();
    }
}

impl Default for ProgressReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgressWatch {
    async fn wait_until_stalled(&self, idle: Duration) -> ProgressIdleElapsed {
        loop {
            // Register before observing the timestamp so a concurrent progress
            // report cannot be missed between the observation and the await.
            let notified = self.state.notify.notified();
            let deadline = *self
                .state
                .last_progress
                .lock()
                .expect("progress state mutex must not be poisoned")
                + idle;
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);

            tokio::select! {
                () = notified => continue,
                () = &mut sleep => {
                    let last_progress = *self
                        .state
                        .last_progress
                        .lock()
                        .expect("progress state mutex must not be poisoned");
                    if last_progress + idle <= tokio::time::Instant::now() {
                        return ProgressIdleElapsed;
                    }
                }
            }
        }
    }
}

/// Run an operation while caller-observable progress keeps resetting the idle
/// budget. Unlike [`timeout`], this does not impose a total transfer duration.
pub async fn progress_timeout<F>(
    idle: Duration,
    watch: &ProgressWatch,
    future: F,
) -> Result<F::Output, ProgressIdleElapsed>
where
    F: Future,
{
    tokio::select! {
        output = future => Ok(output),
        elapsed = watch.wait_until_stalled(idle) => Err(elapsed),
    }
}

/// No caller-observable progress occurred before the configured idle deadline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("the kernal-api async operation made no progress before its idle deadline")]
pub struct ProgressIdleElapsed;

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

    #[test]
    fn cancellation_source_unblocks_a_pending_operation() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            let source = CancellationSource::new();
            let token = source.token();
            let waiter = launch({
                let token = token.clone();
                async move { cancellable(&token, std::future::pending::<()>()).await }
            });
            yield_now().await;
            source.cancel();
            assert_eq!(waiter.await.unwrap(), Err(Cancelled));
        });
    }

    #[test]
    fn cancellation_between_state_check_and_waiter_poll_is_not_lost() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            let source = CancellationSource::new();
            let token = source.token();
            token
                .state
                .cancel_after_waiter_check
                .store(true, Ordering::Release);

            token.cancelled().await;
            assert!(source.is_cancelled());
        });
    }

    #[test]
    fn connection_deadline_has_its_own_typed_error() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        let deadline = Deadline::after(Duration::ZERO);
        assert!(deadline.is_elapsed());
        let result = runtime.run(connection_until(deadline, std::future::pending::<()>()));
        assert_eq!(result, Err(ConnectionDeadlineElapsed));
    }

    #[test]
    fn progress_keeps_an_operation_alive_beyond_one_idle_window() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            tokio::time::pause();
            let reporter = ProgressReporter::new();
            let watch = reporter.watch();
            let result = launch(async move {
                let operation = async move {
                    for _ in 0..3 {
                        sleep(Duration::from_millis(10)).await;
                        reporter.report_progress();
                    }
                    7_u8
                };
                progress_timeout(Duration::from_millis(15), &watch, operation).await
            });

            for _ in 0..3 {
                yield_now().await;
                tokio::time::advance(Duration::from_millis(10)).await;
            }
            assert_eq!(result.await.unwrap(), Ok(7));
        });
    }

    #[test]
    fn stalled_operation_expires_after_the_progress_idle_window() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            tokio::time::pause();
            let reporter = ProgressReporter::new();
            let watch = reporter.watch();
            let result = launch(async move {
                progress_timeout(Duration::from_millis(10), &watch, std::future::pending::<()>())
                    .await
            });
            yield_now().await;
            tokio::time::advance(Duration::from_millis(10)).await;
            assert_eq!(result.await.unwrap(), Err(ProgressIdleElapsed));
        });
    }
}
