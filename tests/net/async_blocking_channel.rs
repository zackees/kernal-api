use kernal_api::async_engine::{channel, BlockingSendError, RuntimeBuilder, TryRecvError};
use std::time::Duration;

#[test]
fn blocking_sender_works_without_a_runtime_and_preserves_order() {
    let (sender, mut receiver) = channel(1);
    sender.blocking_send(1).unwrap();
    let (finished, completion) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        sender.blocking_send(2).unwrap();
        finished.send(()).unwrap();
    });
    assert!(completion.recv_timeout(Duration::from_millis(50)).is_err());
    assert_eq!(receiver.try_recv(), Ok(1));
    completion.recv_timeout(Duration::from_secs(2)).unwrap();
    worker.join().unwrap();
    assert_eq!(receiver.try_recv(), Ok(2));
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn closing_or_dropping_receiver_wakes_blocked_sender_with_its_value() {
    for close in [true, false] {
        let (sender, mut receiver) = channel(1);
        sender.blocking_send(1).unwrap();
        let (finished, completion) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            finished.send(sender.blocking_send(2)).unwrap();
        });
        assert!(completion.recv_timeout(Duration::from_millis(50)).is_err());
        if close {
            receiver.close();
            assert_eq!(receiver.try_recv(), Ok(1));
        } else {
            drop(receiver);
        }
        assert!(matches!(
            completion.recv_timeout(Duration::from_secs(2)).unwrap(),
            Err(BlockingSendError::Closed(2))
        ));
        worker.join().unwrap();
    }
}

#[test]
fn blocking_sender_rejects_async_context_without_enqueuing() {
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (sender, mut receiver) = channel(1);
    runtime.run(async {
        assert!(matches!(
            sender.blocking_send(7),
            Err(BlockingSendError::AsyncContext(7))
        ));
        assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
    });
    sender.blocking_send(8).unwrap();
    assert_eq!(receiver.try_recv(), Ok(8));
}
