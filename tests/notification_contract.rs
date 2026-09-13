//! Notification ordering relied on by cache publication and coalescing.

use kernal_api::async_engine::Notify;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

#[test]
fn broadcast_between_waiter_creation_and_first_poll_is_observed() {
    let notify = Notify::new();
    let waiter = notify.notified();
    notify.notify_waiters();
    let mut waiter = pin!(waiter);
    assert_eq!(
        std::future::Future::poll(waiter.as_mut(), &mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    );
}

#[test]
fn notify_one_stores_only_one_permit_without_waiters() {
    let notify = Notify::new();
    notify.notify_one();
    notify.notify_one();
    let mut first = pin!(notify.notified());
    let mut second = pin!(notify.notified());
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(
        std::future::Future::poll(first.as_mut(), &mut context),
        Poll::Ready(())
    );
    assert_eq!(
        std::future::Future::poll(second.as_mut(), &mut context),
        Poll::Pending
    );
}

#[test]
fn broadcast_does_not_create_a_permit_for_later_waiters() {
    let notify = Notify::new();
    notify.notify_waiters();
    let mut waiter = pin!(notify.notified());
    assert_eq!(
        std::future::Future::poll(waiter.as_mut(), &mut Context::from_waker(Waker::noop())),
        Poll::Pending
    );
}

#[test]
fn enable_reports_an_already_available_notification() {
    let notify = Notify::new();
    notify.notify_one();
    let mut waiter = pin!(notify.notified());
    assert!(waiter.as_mut().enable());
    assert_eq!(
        std::future::Future::poll(waiter.as_mut(), &mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    );
}

#[test]
fn owned_waiters_register_before_first_poll_and_survive_facade_drop() {
    let notify = std::sync::Arc::new(Notify::new());
    let mut first = pin!(notify.clone().owned_notified());
    let mut second = pin!(notify.clone().owned_notified());
    assert!(!first.as_mut().enable());
    assert!(!second.as_mut().enable());
    // Both enabled waiters must receive a permit; without pre-registration
    // these calls would coalesce into a single stored permit.
    notify.notify_one();
    notify.notify_one();
    drop(notify);
    let mut context = Context::from_waker(Waker::noop());
    assert_eq!(
        std::future::Future::poll(first.as_mut(), &mut context),
        Poll::Ready(())
    );
    assert_eq!(
        std::future::Future::poll(second.as_mut(), &mut context),
        Poll::Ready(())
    );
}

#[test]
fn cancelling_a_notified_waiter_transfers_its_permit_to_the_next_waiter() {
    let notify = std::sync::Arc::new(Notify::new());
    let mut first = Box::pin(notify.clone().owned_notified());
    let mut second = pin!(notify.clone().owned_notified());
    assert!(!first.as_mut().enable());
    assert!(!second.as_mut().enable());
    notify.notify_one();
    // First owns the notification but has not consumed it through poll.
    // Cancelling it must not strand the second registered waiter.
    drop(first);
    assert_eq!(
        std::future::Future::poll(second.as_mut(), &mut Context::from_waker(Waker::noop())),
        Poll::Ready(())
    );
}

#[test]
fn notification_futures_can_move_between_executor_threads() {
    fn assert_send<T: Send>(_: &T) {}
    let notify = std::sync::Arc::new(Notify::new());
    assert_send(&notify.notified());
    let owned = notify.clone().owned_notified();
    assert_send(&owned);
    notify.notify_waiters();
    std::thread::spawn(move || {
        let mut owned = pin!(owned);
        assert_eq!(
            std::future::Future::poll(owned.as_mut(), &mut Context::from_waker(Waker::noop())),
            Poll::Ready(())
        );
    })
    .join()
    .expect("owned notification crosses threads");
}
