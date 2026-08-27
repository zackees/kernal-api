#![cfg(feature = "wasm-sketch-host")]

//! Contract coverage for issue #43.  The looping artifacts belong in the
//! private host tests; this integration test keeps the public facade shape
//! explicit without exposing Wasmtime objects.

use std::time::Duration;

use kernal_api::async_engine::{CancellationSource, RuntimeBuilder};
use kernal_api::wasm::{SketchCompilerConfig, SketchEpochLimits};

#[test]
fn epoch_limits_are_bounded_and_cancellation_is_facade_owned() {
    let limits = SketchEpochLimits::new(Duration::from_millis(10), Duration::from_millis(1), 2)
        .expect("finite nonzero epoch limits");
    assert_eq!(limits.maximum_active_registrations(), 2);

    let config = SketchCompilerConfig::default()
        .with_epoch_limits(limits)
        .expect("epoch limits are part of compiler execution policy");
    assert_eq!(config.execution_limits().epoch_limits(), limits);

    let runtime = RuntimeBuilder::current_thread().enable_all().build().expect("runtime");
    runtime.run(async {
        let first = kernal_api::async_engine::RuntimeHandle::current().expect("current runtime");
        let second = kernal_api::async_engine::RuntimeHandle::current().expect("same runtime");
        assert_eq!(first, second, "current handles retain live-runtime identity");
        let task = first.launch_blocking(|| 7_u8);
        assert_eq!(task.await.expect("blocking task"), 7);
    });

    let cancellation = CancellationSource::new();
    assert!(!cancellation.token().is_cancelled());
    cancellation.cancel();
    assert!(cancellation.token().is_cancelled());
}
