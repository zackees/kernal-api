use super::*;
use crate::async_engine::{Runtime, RuntimeBuilder};

const HELPER: &str = "KERNAL_COMPILER_GRANT_TEST_HELPER";

fn runtime() -> Runtime {
    RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

fn spec(mode: &str) -> SpawnSpec {
    SpawnSpec::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("operations::process_resource::tests::compiler_helper")
        .arg("--nocapture")
        .current_dir(std::env::current_dir().unwrap())
        .clear_env(true)
        .env(HELPER, mode)
}

#[test]
fn compiler_helper() {
    match std::env::var(HELPER).as_deref() {
        Ok("blocked") => {
            use std::io::Write;
            let mut output = std::io::stdout().lock();
            for _ in 0..16384 {
                output.write_all(&[0xf1; 4096]).unwrap();
            }
        }
        Ok("exit") => println!("native compiler fixture output"),
        Ok("dual") => {
            use std::io::Write;
            let mut stdout = std::io::stdout().lock();
            let mut stderr = std::io::stderr().lock();
            for _ in 0..512 {
                stdout.write_all(&[0xf1; 4096]).unwrap();
                stderr.write_all(&[0xf2; 4096]).unwrap();
            }
        }
        Ok("silent") => std::thread::sleep(Duration::from_secs(30)),
        _ => {}
    }
}

#[path = "process_output_tests.rs"]
mod output;

async fn published(
    hub: &OperationHub,
    operation: OpaqueToken,
) -> (OpaqueToken, Arc<ProcessSession>) {
    crate::async_engine::timeout(Duration::from_secs(10), async {
        loop {
            let result = {
                let state = hub.state.lock().unwrap();
                let op = state.operations.get(&operation).unwrap();
                op.terminal.map(|terminal| {
                    assert_eq!(terminal.terminal, Terminal::Completed);
                    let token = terminal.resource.unwrap();
                    let ResourceValue::CompilerProcess(value) = &state.resources[&token].value
                    else {
                        panic!("process resource");
                    };
                    (token, Arc::clone(value.session.as_ref().unwrap()))
                })
            };
            if let Some(result) = result {
                return result;
            }
            crate::async_engine::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[test]
fn compiler_grant_rejects_ambient_authority_and_invalid_deadline() {
    let hub = OperationHub::new(4, 4).unwrap();
    for (spec, deadline) in [
        (spec("exit").clear_env(false), Duration::from_secs(1)),
        (spec("exit").current_dir("relative"), Duration::from_secs(1)),
        (SpawnSpec::new("rustc"), Duration::from_secs(1)),
        (spec("exit"), Duration::ZERO),
        (spec("exit"), Duration::from_secs(301)),
        (
            spec("exit").bind_lifetime(crate::platform::process::LifetimeOwner::Process(
                crate::platform::process::ProcessId::current(),
            )),
            Duration::from_secs(1),
        ),
    ] {
        assert_eq!(
            hub.grant_compiler(7, spec, deadline),
            Err(HubError::Invalid)
        );
    }
    assert_eq!(hub.snapshot().live_resources, 0);
}

#[test]
fn compiler_quotas_and_owner_reject_before_consuming_grant() {
    let runtime = runtime();
    for (operations, resources) in [(0, 2), (1, 1)] {
        let hub = OperationHub::new(operations, resources).unwrap();
        let grant = hub
            .grant_compiler(7, spec("exit"), Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            hub.submit_compiler_spawn(runtime.handle(), 8, grant),
            Err(HubError::WrongRights)
        );
        assert_eq!(
            hub.submit_compiler_spawn(runtime.handle(), 7, grant),
            Err(HubError::Quota)
        );
        let state = hub.state.lock().unwrap();
        assert!(matches!(
            &state.resources[&grant].value,
            ResourceValue::CompilerGrant(CompilerGrant { spec: Some(_), .. })
        ));
        assert!(state.operations.is_empty());
        assert!(state.process_jobs.is_empty());
    }
}

#[test]
fn revoked_grant_before_scheduling_never_publishes_or_spawns() {
    let runtime = runtime();
    let hub = OperationHub::new(2, 2).unwrap();
    let grant = hub
        .grant_compiler(7, spec("blocked"), Duration::from_secs(10))
        .unwrap();
    let operation = hub
        .submit_compiler_spawn(runtime.handle(), 7, grant)
        .unwrap();
    hub.close_resource(grant).unwrap();
    hub.abandon_compiler_spawn(7, operation).unwrap();
    hub.close_all(Terminal::Closed);
    runtime.run(hub.join_process_jobs()).unwrap();
    assert_eq!(hub.process_spawn_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(hub.snapshot().retained_process_jobs, 0);
    assert_eq!(hub.snapshot().live_resources, 0);
    assert_eq!(hub.snapshot().pending_operations, 0);
}

#[test]
fn abandoned_uncollected_spawn_reaps_with_output_undrained() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(2, 2).unwrap();
        let grant = hub
            .grant_compiler(7, spec("blocked"), Duration::from_secs(10))
            .unwrap();
        let operation = hub
            .submit_compiler_spawn(runtime.handle(), 7, grant)
            .unwrap();
        let (_, session) = published(&hub, operation).await;
        crate::async_engine::timeout(Duration::from_secs(5), async {
            loop {
                let event = session.next_output().await.expect("fixture output");
                if let crate::ProcessOutputEvent::Chunk(chunk) = event {
                    if chunk.bytes().contains(&0xf1) {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        crate::async_engine::sleep(Duration::from_millis(50)).await;
        assert!(session.poll().await.unwrap().is_none());
        assert_eq!(
            hub.submit_compiler_spawn(runtime.handle(), 7, grant),
            Err(HubError::Closed)
        );
        hub.abandon_compiler_spawn(7, operation).unwrap();
        hub.close_all(Terminal::Closed);
        crate::async_engine::timeout(Duration::from_secs(5), hub.join_process_jobs())
            .await
            .unwrap()
            .unwrap();
        assert!(session.poll().await.unwrap().is_some());
        assert_ne!(session.wait().await.unwrap().exit_code(), Some(0));
        assert_eq!(hub.snapshot().retained_process_jobs, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
    });
}

#[test]
fn natural_exit_retains_output_until_resource_revocation() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(2, 2).unwrap();
        let grant = hub
            .grant_compiler(7, spec("exit"), Duration::from_secs(10))
            .unwrap();
        let operation = hub
            .submit_compiler_spawn(runtime.handle(), 7, grant)
            .unwrap();
        let (_, session) = published(&hub, operation).await;
        assert_eq!(session.wait().await.unwrap().exit_code(), Some(0));
        assert!(session.next_output().await.is_some());
        hub.close_all(Terminal::Closed);
        hub.join_process_jobs().await.unwrap();
    });
}

#[test]
fn cancelled_join_keeps_cleanup_handles_for_retry() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(1, 1).unwrap();
        let release = Arc::new(Notify::new());
        let task_release = Arc::clone(&release);
        hub.state
            .lock()
            .unwrap()
            .process_jobs
            .push(runtime.handle().launch(async move {
                task_release.notified().await;
                Ok(())
            }));
        hub.close_all(Terminal::Closed);
        {
            let mut join = Box::pin(hub.join_process_jobs());
            assert!(join
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending());
        }
        assert_eq!(hub.snapshot().retained_process_jobs, 1);
        release.notify_one();
        hub.join_process_jobs().await.unwrap();
        assert_eq!(hub.snapshot().retained_process_jobs, 0);
    });
}

#[test]
fn deadline_revokes_and_reaps_without_output_delivery() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(2, 2).unwrap();
        let grant = hub
            .grant_compiler(7, spec("blocked"), Duration::from_secs(3))
            .unwrap();
        let operation = hub
            .submit_compiler_spawn(runtime.handle(), 7, grant)
            .unwrap();
        let (process, session) = published(&hub, operation).await;
        let cancelled = {
            let state = hub.state.lock().unwrap();
            let ResourceValue::CompilerProcess(value) = &state.resources[&process].value else {
                panic!("process resource");
            };
            value.cancel.token()
        };
        crate::async_engine::timeout(Duration::from_secs(5), cancelled.cancelled())
            .await
            .unwrap();
        assert!(!hub.state.lock().unwrap().resources.contains_key(&process));
        hub.close_all(Terminal::Closed);
        hub.join_process_jobs().await.unwrap();
        assert!(session.poll().await.unwrap().is_some());
        assert_ne!(session.wait().await.unwrap().exit_code(), Some(0));
    });
}

#[test]
fn process_job_quota_is_independent_of_consumed_operations() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(8, 8).unwrap();
        let release = CancellationSource::new();
        for _ in 0..MAX_PROCESS_JOBS {
            let token = release.token();
            hub.state
                .lock()
                .unwrap()
                .process_jobs
                .push(runtime.handle().launch(async move {
                    token.cancelled().await;
                    Ok(())
                }));
        }
        let grant = hub
            .grant_compiler(7, spec("exit"), Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            hub.submit_compiler_spawn(runtime.handle(), 7, grant),
            Err(HubError::Quota)
        );
        {
            let state = hub.state.lock().unwrap();
            assert!(state.operations.is_empty());
            assert!(matches!(
                &state.resources[&grant].value,
                ResourceValue::CompilerGrant(CompilerGrant { spec: Some(_), .. })
            ));
        }
        hub.close_all(Terminal::Closed);
        release.cancel();
        hub.join_process_jobs().await.unwrap();
    });
}

#[test]
fn cleanup_failure_is_latched_across_repeated_joins() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(1, 1).unwrap();
        hub.state
            .lock()
            .unwrap()
            .process_jobs
            .push(runtime.handle().launch(async { Err(HubError::Closed) }));
        hub.close_all(Terminal::Closed);
        assert_eq!(hub.join_process_jobs().await, Err(HubError::Closed));
        assert_eq!(hub.join_process_jobs().await, Err(HubError::Closed));
    });
}

#[test]
fn concurrent_cleanup_joiners_both_complete() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(1, 1).unwrap();
        let release = Arc::new(Notify::new());
        let task_release = Arc::clone(&release);
        hub.state
            .lock()
            .unwrap()
            .process_jobs
            .push(runtime.handle().launch(async move {
                task_release.notified().await;
                Ok(())
            }));
        hub.close_all(Terminal::Closed);
        let mut joins = Vec::new();
        for _ in 0..2 {
            let (started, ready) = crate::async_engine::oneshot_channel();
            let hub = Arc::clone(&hub);
            joins.push(runtime.handle().launch(async move {
                started.send(()).unwrap();
                hub.join_process_jobs().await
            }));
            ready.await.unwrap();
        }
        release.notify_one();
        crate::async_engine::timeout(Duration::from_secs(2), async {
            for join in joins {
                join.await.unwrap().unwrap();
            }
        })
        .await
        .expect("every cleanup joiner must wake");
    });
}

#[test]
fn cancellation_after_spawn_before_publication_reaps_unpublished_child() {
    let runtime = runtime();
    runtime.run(async {
        let hub = OperationHub::new(2, 2).unwrap();
        let (started, ready) = crate::async_engine::oneshot_channel();
        let (resume, resumed) = crate::async_engine::oneshot_channel();
        *hub.process_spawn_checkpoint.lock().unwrap() = Some(SpawnCheckpoint {
            started,
            resume: resumed,
        });
        let grant = hub
            .grant_compiler(7, spec("blocked"), Duration::from_secs(10))
            .unwrap();
        let operation = hub
            .submit_compiler_spawn(runtime.handle(), 7, grant)
            .unwrap();
        let session = crate::async_engine::timeout(Duration::from_secs(5), ready)
            .await
            .unwrap()
            .unwrap();
        hub.cancel_wire(7, operation.wire()).unwrap();
        assert_eq!(
            hub.poll_wire(7, operation.wire()),
            u64::from(STATUS_CANCELLED)
        );
        assert_eq!(
            hub.snapshot().live_resources,
            1,
            "only consumed grant remains"
        );
        assert_eq!(
            hub.snapshot().retained_process_jobs,
            1,
            "unpublished child still needs reaping"
        );
        resume.send(()).unwrap();
        hub.close_all(Terminal::Closed);
        crate::async_engine::timeout(Duration::from_secs(5), hub.join_process_jobs())
            .await
            .unwrap()
            .unwrap();
        assert!(session.poll().await.unwrap().is_some());
        assert_ne!(session.wait().await.unwrap().exit_code(), Some(0));
        assert_eq!(hub.snapshot().retained_process_jobs, 0);
        assert_eq!(hub.snapshot().pending_operations, 0);
        assert_eq!(hub.snapshot().live_resources, 0);
    });
}

#[test]
fn expired_queued_command_is_rejected_without_native_spawn() {
    let runtime = runtime();
    let hub = OperationHub::new(2, 2).unwrap();
    let grant = hub
        .grant_compiler(7, spec("blocked"), Duration::from_millis(1))
        .unwrap();
    let operation = hub
        .submit_compiler_spawn(runtime.handle(), 7, grant)
        .unwrap();
    // This current-thread runtime has not polled the supervisor yet.
    std::thread::sleep(Duration::from_millis(10));
    runtime.run(async {
        let status = crate::async_engine::timeout(Duration::from_secs(5), async {
            loop {
                let status = hub.poll_wire(7, operation.wire());
                if status != 0 {
                    break status;
                }
                crate::async_engine::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(status, u64::from(STATUS_TIMED_OUT));
        hub.close_all(Terminal::Closed);
        hub.join_process_jobs().await.unwrap();
        assert_eq!(hub.process_spawn_attempts.load(Ordering::SeqCst), 0);
    });
}

#[test]
fn compiler_grants_require_spawner_owned_lifetimes() {
    let hub = OperationHub::new(2, 2).unwrap();
    for command in [spec("exit"), spec("exit").kill_when_owner_dies(true)] {
        let grant = hub
            .grant_compiler(7, command, Duration::from_secs(1))
            .unwrap();
        let state = hub.state.lock().unwrap();
        let ResourceValue::CompilerGrant(grant) = &state.resources[&grant].value else {
            panic!("compiler grant");
        };
        assert_eq!(
            grant.spec.as_ref().unwrap().lifetime_owner,
            Some(crate::platform::process::LifetimeOwner::Spawner)
        );
    }
}
