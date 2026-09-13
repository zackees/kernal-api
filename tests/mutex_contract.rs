//! Queue and ownership contracts needed by daemon state and link-output locks.
use kernal_api::async_engine::Mutex;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

fn poll_once<F: std::future::Future>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(Waker::noop()))
}

#[test]
fn cancelling_first_waiter_preserves_fifo_for_remaining_waiters() {
    let lock = Arc::new(Mutex::new(0));
    let held = lock.try_lock().expect("initial lock");
    let mut cancelled = Box::pin(lock.clone().lock_owned());
    let mut next = Box::pin(lock.clone().lock_owned());
    assert!(poll_once(cancelled.as_mut()).is_pending());
    assert!(poll_once(next.as_mut()).is_pending());
    drop(cancelled);
    drop(held);
    assert!(
        lock.clone().try_lock_owned().is_err(),
        "cannot bypass queued waiter"
    );
    let Poll::Ready(mut guard) = poll_once(next.as_mut()) else {
        panic!("remaining waiter must acquire");
    };
    *guard = 42;
    assert!(lock.try_lock().is_err());
    drop(guard);
    assert_eq!(*lock.try_lock().expect("released"), 42);
}

#[test]
fn owned_guard_keeps_value_alive_across_threads_and_facade_drop() {
    let lock = Arc::new(Mutex::new(String::from("retained")));
    let guard = lock.clone().try_lock_owned().expect("owned guard");
    drop(lock);
    std::thread::spawn(move || assert_eq!(&*guard, "retained"))
        .join()
        .expect("guard owns lock storage");
}

#[test]
fn blocking_and_borrowed_acquisition_share_the_same_value() {
    let lock = Mutex::new(1);
    *lock.blocking_lock() = 2;
    let mut acquire = Box::pin(lock.lock());
    let Poll::Ready(guard) = poll_once(acquire.as_mut()) else {
        panic!("uncontended borrowed lock");
    };
    assert_eq!(*guard, 2);
}
