//! Lock ordering and ownership used by compile admission/publication.

use kernal_api::async_engine::RwLock;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

fn poll_once<F: std::future::Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    std::future::Future::poll(future, &mut Context::from_waker(Waker::noop()))
}

#[test]
fn queued_writer_precedes_later_readers_and_try_read() {
    let lock = Arc::new(RwLock::new(1));
    let first = lock.clone().try_read_owned().expect("initial read");
    let mut writer = pin!(lock.clone().write_owned());
    assert!(poll_once(writer.as_mut()).is_pending());
    assert!(lock.clone().try_read_owned().is_err());
    let mut reader = pin!(lock.clone().read_owned());
    assert!(poll_once(reader.as_mut()).is_pending());
    drop(first);
    assert!(poll_once(reader.as_mut()).is_pending());
    let Poll::Ready(mut write_guard) = poll_once(writer.as_mut()) else {
        panic!("queued writer must acquire after first reader releases");
    };
    *write_guard = 2;
    assert!(poll_once(reader.as_mut()).is_pending());
    drop(write_guard);
    let Poll::Ready(read_guard) = poll_once(reader.as_mut()) else {
        panic!("later reader must acquire after writer releases");
    };
    assert_eq!(*read_guard, 2);
}

#[test]
fn cancelling_a_queued_writer_unblocks_later_readers() {
    let lock = Arc::new(RwLock::new(()));
    let _first = lock.clone().try_read_owned().expect("initial read");
    let mut writer = Box::pin(lock.clone().write_owned());
    assert!(poll_once(writer.as_mut()).is_pending());
    let mut reader = pin!(lock.clone().read_owned());
    assert!(poll_once(reader.as_mut()).is_pending());
    drop(writer);
    assert!(poll_once(reader.as_mut()).is_ready());
}

#[test]
fn owned_read_guard_outlives_the_facade_and_moves_between_threads() {
    let lock = Arc::new(RwLock::new(String::from("retained")));
    let guard = lock.clone().try_read_owned().expect("read");
    drop(lock);
    std::thread::spawn(move || assert_eq!(&*guard, "retained"))
        .join()
        .expect("guard retains protected data");
}

#[tokio::test]
async fn blocking_work_retains_its_publication_guard_after_handle_cancellation() {
    let lock = Arc::new(RwLock::new(()));
    let guard = lock.clone().write_owned().await;
    let (started, ready) = kernal_api::async_engine::oneshot_channel();
    let (finished, done) = kernal_api::async_engine::oneshot_channel();
    let (release, released) = std::sync::mpsc::channel();
    let task = kernal_api::async_engine::launch_blocking(move || {
        let guard = guard;
        let _ = started.send(());
        let _ = released.recv();
        drop(guard);
        let _ = finished.send(());
    });
    ready.await.expect("blocking publication started");
    task.cancel();
    drop(task);
    assert!(
        lock.clone().try_read_owned().is_err(),
        "running blocking mutation must retain its exclusion lock"
    );
    release.send(()).expect("release blocking publication");
    done.await.expect("publication guard released");
    assert!(lock.try_read_owned().is_ok());
}
