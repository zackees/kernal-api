//! Join deadlines do not substitute for confirmed task cancellation.

use kernal_api::async_engine::{launch, oneshot_channel as oneshot, timeout};
use std::time::Duration;

#[tokio::test]
async fn task_handle_is_debug_without_requiring_debug_output() {
    struct Output;
    fn assert_debug<T: std::fmt::Debug>(_: &T) {}
    let task = launch(async { Output });
    assert_debug(&task);
    let _ = task.await.expect("task output");
}

#[tokio::test(start_paused = true)]
async fn timed_out_borrowed_join_can_still_be_cancelled_and_joined() {
    struct ObserveDrop(Option<kernal_api::async_engine::OneshotSender<()>>);
    impl Drop for ObserveDrop {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let (started, ready) = oneshot();
    let (dropped, destroyed) = oneshot();
    let mut task = launch(async move {
        let _observe = ObserveDrop(Some(dropped));
        let _ = started.send(());
        std::future::pending::<()>().await;
    });
    ready.await.expect("task started");
    assert!(timeout(Duration::from_secs(1), &mut task).await.is_err());
    task.cancel();
    let error = task.await.expect_err("cancelled join");
    assert!(error.is_cancelled());
    destroyed
        .await
        .expect("joining cancellation observes future destruction");
}

#[tokio::test]
async fn cancelling_started_blocking_work_still_allows_joining_its_result() {
    let (started, ready) = oneshot();
    let (release, released) = std::sync::mpsc::channel();
    let task = kernal_api::async_engine::launch_blocking(move || {
        let _ = started.send(());
        released.recv().expect("release running blocking work");
        42
    });
    ready.await.expect("blocking closure has started");
    task.cancel();
    release.send(()).expect("running work was not interrupted");
    assert_eq!(task.await.expect("blocking result remains joinable"), 42);
}

#[tokio::test]
async fn supervisor_can_distinguish_task_panics_from_cancellation() {
    let task = launch(async { panic!("watcher consumer failed") });
    let error = task
        .await
        .expect_err("panic should be reported to supervisor");
    assert!(error.is_panic());
    assert!(!error.is_cancelled());
}

#[tokio::test]
async fn explicitly_detached_work_retains_its_completion_signal() {
    let (release, released) = oneshot();
    let (complete, completed) = oneshot();
    launch(async move {
        released.await.expect("detached task release");
        let _ = complete.send(17);
    })
    .detach();
    release.send(()).expect("detached task still owns receiver");
    assert_eq!(completed.await.expect("detached task completes"), 17);
}

#[tokio::test]
async fn detach_on_drop_remains_joinable_and_explicitly_cancellable() {
    assert_eq!(
        launch(async { 42 }).detach_on_drop().await.expect("join"),
        42
    );
    let task = launch(std::future::pending::<()>()).detach_on_drop();
    task.cancel();
    assert!(task
        .await
        .expect_err("explicit cancellation")
        .is_cancelled());
}

#[test]
fn detach_on_drop_preserves_queued_blocking_work() {
    let runtime = kernal_api::async_engine::RuntimeBuilder::current_thread()
        .max_blocking_threads(1)
        .build()
        .expect("runtime");
    runtime.run(async {
        let (started, ready) = oneshot();
        let (release, released) = std::sync::mpsc::channel();
        let blocker = kernal_api::async_engine::launch_blocking(move || {
            let _ = started.send(());
            released.recv().expect("release sole blocking worker");
        });
        ready.await.expect("worker occupied");
        let (completed, completion) = oneshot();
        let queued = kernal_api::async_engine::launch_blocking(move || {
            let _ = completed.send(42);
        })
        .detach_on_drop();
        drop(queued);
        release.send(()).expect("release worker");
        blocker.await.expect("blocking worker finished");
        assert_eq!(
            completion.await.expect("queued work survives handle drop"),
            42
        );
    });
}

#[test]
fn zero_blocking_thread_limit_is_rejected_before_runtime_construction() {
    assert!(std::panic::catch_unwind(|| {
        kernal_api::async_engine::RuntimeBuilder::current_thread().max_blocking_threads(0)
    })
    .is_err());
}

#[tokio::test]
async fn cancelling_joining_caller_does_not_cancel_committed_child() {
    let (registered, ready) = oneshot();
    let (release, released) = oneshot();
    let (finished, completion) = oneshot();
    let caller = launch(async move {
        let child = launch(async move {
            released.await.expect("release committed child");
            let _ = finished.send(42);
        })
        .detach_on_drop();
        let _ = registered.send(());
        child.await.expect("committed child result")
    });
    ready
        .await
        .expect("caller owns joinable detached-on-drop child");
    caller.cancel();
    assert!(caller.await.expect_err("caller cancelled").is_cancelled());
    release
        .send(())
        .expect("caller cancellation retained child");
    assert_eq!(
        completion
            .await
            .expect("child completed independently of caller"),
        42
    );
}
