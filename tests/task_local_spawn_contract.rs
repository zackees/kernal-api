//! Scope bindings belong to polling tasks, not newly spawned children.

use kernal_api::async_engine::{launch, TaskLocalAccessError};

kernal_api::task_local! {
    static REQUEST_ID: u64 = REQUEST_ID_STORAGE;
}

#[tokio::test(flavor = "current_thread")]
async fn spawned_work_does_not_inherit_the_callers_task_local_binding() {
    REQUEST_ID
        .scope(42, async {
            assert_eq!(REQUEST_ID.with(|value| *value), 42);
            let child = launch(async { REQUEST_ID.try_with(|value| *value) });
            assert_eq!(
                child.await.expect("child task"),
                Err(TaskLocalAccessError::Unbound)
            );
            assert_eq!(REQUEST_ID.with(|value| *value), 42);
        })
        .await;
    assert_eq!(
        REQUEST_ID.try_with(|value| *value),
        Err(TaskLocalAccessError::Unbound)
    );
}

#[tokio::test(flavor = "current_thread")]
async fn child_can_install_its_own_binding_without_changing_the_parent() {
    REQUEST_ID
        .scope(42, async {
            let child = launch(REQUEST_ID.scope(7, async {
                kernal_api::async_engine::yield_now().await;
                REQUEST_ID.with(|value| *value)
            }));
            assert_eq!(child.await.expect("scoped child"), 7);
            assert_eq!(REQUEST_ID.with(|value| *value), 42);
        })
        .await;
}
