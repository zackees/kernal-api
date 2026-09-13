//! Channel handles do not require payload formatting support.

#[test]
fn unbounded_handles_are_debug_and_sender_is_clone_without_payload_bounds() {
    struct Payload;
    fn assert_debug<T: std::fmt::Debug>(_: &T) {}
    let (sender, receiver) = kernal_api::async_engine::unbounded_channel::<Payload>();
    assert_debug(&sender);
    assert_debug(&receiver);
    let cloned = sender.clone();
    assert_debug(&cloned);
    assert!(cloned.send(Payload).is_ok());
}

#[test]
fn closing_the_receiver_rejects_new_writes_but_preserves_queued_order() {
    let (sender, mut receiver) = kernal_api::async_engine::unbounded_channel();
    assert!(sender.send(1).is_ok());
    assert!(sender.send(2).is_ok());
    receiver.close();
    assert!(sender.is_closed());
    assert!(sender.send(3).is_err());
    assert_eq!(receiver.try_recv().expect("first queued write"), 1);
    assert_eq!(receiver.try_recv().expect("second queued write"), 2);
    let mut end = std::pin::pin!(receiver.recv());
    assert_eq!(
        std::future::Future::poll(
            end.as_mut(),
            &mut std::task::Context::from_waker(std::task::Waker::noop())
        ),
        std::task::Poll::Ready(None)
    );
}

#[test]
fn bounded_and_oneshot_handles_do_not_require_debug_payloads() {
    struct Payload;
    fn assert_debug<T: std::fmt::Debug>(_: &T) {}
    let (sender, receiver) = kernal_api::async_engine::channel::<Payload>(1);
    assert_debug(&sender);
    assert_debug(&receiver);
    let (sender, receiver) = kernal_api::async_engine::oneshot_channel::<Payload>();
    assert_debug(&sender);
    assert_debug(&receiver);
    let tasks = kernal_api::async_engine::TaskGroup::<Payload>::new();
    assert_debug(&tasks);
}
