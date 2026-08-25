//! The shared systems pieces must coexist in one final executable.

#[cfg(feature = "tokio-console")]
use std::time::Duration;

use kernal_api::{shell_spec, SpawnSpec, StreamMode};

#[test]
fn async_process_hal_captures_output() {
    let runtime = kernal_api::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .expect("facade-owned runtime");
    runtime.run(async {
        #[cfg(windows)]
        let spec = shell_spec("echo kernal-api");
        #[cfg(not(windows))]
        let spec = shell_spec("printf kernal-api");

        let output = spec
            .stdout(StreamMode::Piped)
            .stderr(StreamMode::Piped)
            .spawn()
            .await
            .expect("spawn through the shared HAL")
            .wait_with_output()
            .await
            .expect("capture output");

        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("kernal-api"));

        let missing = SpawnSpec::new("kernal-api-program-that-does-not-exist")
            .spawn()
            .await;
        assert!(missing.is_err());
    });
}

#[cfg(feature = "profile")]
#[test]
fn cpu_and_async_profiles_share_one_pprof_schema() {
    use kernal_api::profile::async_profile::{to_pprof, TaskSample};
    use kernal_api::profile::export::to_pprof_bytes;
    use kernal_api::profile::symbolize::Frame;
    use kernal_api::profile::{ProfileMetrics, ResolvedSample, SessionResult};

    let cpu = SessionResult {
        samples: vec![ResolvedSample {
            os_tid: 7,
            frames: vec![Frame {
                function: "worker".to_string(),
                module: "fixture".to_string(),
                relative_address: 0x10,
            }],
            truncated: false,
        }],
        metrics: ProfileMetrics {
            samples_captured: 1,
            duration_nanos: 1_000_000,
            hz: 99,
            ..ProfileMetrics::default()
        },
        start_unix_nanos: 1,
        period_nanos: 10_101_010,
    };
    assert!(!to_pprof_bytes(&cpu).is_empty());

    let tasks = vec![TaskSample {
        spawn_stack: vec!["root".into(), "waiting".into()],
        idle_nanos: 9,
        busy_nanos: 1,
        scheduled_nanos: 2,
        polls: 3,
        wakes: 4,
        name: "task".into(),
    }];
    assert!(!to_pprof(&tasks).is_empty());
}

#[cfg(feature = "allocator")]
#[test]
fn allocator_facade_exposes_dormant_heap_profiler() {
    assert!(!kernal_api::allocator::is_enabled());
    let _allocator_type = std::any::TypeId::of::<kernal_api::allocator::Allocator>();
}

#[cfg(feature = "tokio-console")]
#[test]
fn runtime_profile_configuration_is_product_neutral() {
    let config = kernal_api::async_engine::DiagnosticsConfig::new("127.0.0.1:6669")
        .with_publish_interval(Duration::from_millis(20));
    assert_eq!(config.bind(), "127.0.0.1:6669");
    assert_eq!(config.publish_interval(), Some(Duration::from_millis(20)));
}
