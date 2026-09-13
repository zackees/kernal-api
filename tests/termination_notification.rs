#![cfg(unix)]
use kernal_api::async_engine::{timeout, RuntimeBuilder, TerminationSignal};
use std::time::{Duration, Instant};

#[test]
fn termination_registration_delivery_and_cancelled_wait_are_isolated() {
    const CHILD: &str = "KERNAL_TERMINATION_NOTIFICATION_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.run(async {
            let mut first = TerminationSignal::new().unwrap();
            let mut second = TerminationSignal::new().unwrap();
            // Register both before sending: delivery cannot depend on polling
            // recv before the signal, and both subscriptions must observe it.
            send_to_this_child();
            assert_eq!(
                timeout(Duration::from_secs(2), first.recv()).await.unwrap(),
                Some(())
            );
            assert_eq!(
                timeout(Duration::from_secs(2), second.recv())
                    .await
                    .unwrap(),
                Some(())
            );
            {
                use std::future::Future;
                let mut wait = std::pin::pin!(first.recv());
                std::future::poll_fn(|cx| {
                    assert!(wait.as_mut().poll(cx).is_pending());
                    std::task::Poll::Ready(())
                })
                .await;
            }
            send_to_this_child();
            assert_eq!(
                timeout(Duration::from_secs(2), first.recv()).await.unwrap(),
                Some(())
            );
            assert_eq!(
                timeout(Duration::from_secs(2), second.recv())
                    .await
                    .unwrap(),
                Some(())
            );
        });
        return;
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "termination_registration_delivery_and_cancelled_wait_are_isolated",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(
                status.success(),
                "isolated SIGTERM fixture failed: {status}"
            );
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("isolated SIGTERM fixture timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn send_to_this_child() {
    // SAFETY: only this explicitly isolated fixture is targeted, after both
    // handlers were registered. Never send to a parent or process group.
    assert_eq!(unsafe { libc::kill(libc::getpid(), libc::SIGTERM) }, 0);
}
