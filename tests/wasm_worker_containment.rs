#![cfg(feature = "wasm-sketch-worker")]

//! Real-worker containment coverage for #28.  Crash and parent-death cases
//! remain TODO(#28): they need a separately controlled process harness.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use kernal_api::async_engine::{self, CancellationSource, RuntimeBuilder, RuntimeHandle};
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchEpochLimits, SketchExecutionError,
    SketchExecutionLimits, SketchFuelLimits, SketchModulePolicy, SketchWorkerConfig,
    SketchWorkerStopReason, SketchWorkerTerminal, ThreadedRootOutcome,
};

#[path = "support/threaded_fixture.rs"]
mod threaded_fixture;

const OUTER_BOUND: Duration = Duration::from_secs(10);
const DEADLINE: Duration = Duration::from_millis(150);
const GRACE: Duration = Duration::from_millis(100);

fn worker_config() -> SketchWorkerConfig {
    let executable = PathBuf::from(env!("CARGO_BIN_EXE_kernal-wasm-worker"));
    assert!(
        executable.is_absolute(),
        "Cargo supplied an absolute worker path"
    );
    SketchWorkerConfig::new(executable, GRACE).expect("explicit worker configuration")
}

fn compiler(deadline: Duration, fuel: SketchFuelLimits) -> SketchCompiler {
    let epoch = SketchEpochLimits::new(deadline, Duration::from_millis(1), 17)
        .expect("one millisecond epoch tick");
    let limits = SketchExecutionLimits::default()
        .with_fuel_limits(fuel)
        .expect("fuel limits")
        .with_epoch_limits(epoch)
        .expect("epoch limits");
    SketchCompiler::new(
        SketchCompilerConfig::default()
            .with_execution_limits(limits)
            .expect("execution limits"),
    )
    .expect("compiler")
}

fn normal_fuel() -> SketchFuelLimits {
    SketchFuelLimits::default()
}

fn long_fuel() -> SketchFuelLimits {
    SketchFuelLimits::new(1_700_000_000_000, 100_000_000_000, 100_000_000_000).expect("long fuel")
}

fn tiny_fuel() -> SketchFuelLimits {
    SketchFuelLimits::new(10_000, 10_000, 1_000).expect("tiny fuel")
}

fn admit(compiler: &SketchCompiler, bytes: Vec<u8>) -> Arc<kernal_api::wasm::AdmittedSketch> {
    compiler
        .admit(
            &bytes,
            SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy"),
        )
        .expect("admission")
}

async fn contained(
    sketch: &Arc<kernal_api::wasm::AdmittedSketch>,
    runtime: RuntimeHandle,
    config: &SketchWorkerConfig,
    cancellation: Option<CancellationSource>,
) -> SketchWorkerTerminal {
    let token = cancellation
        .as_ref()
        .map(CancellationSource::token)
        .unwrap_or_else(|| CancellationSource::new().token());
    async_engine::timeout(
        OUTER_BOUND,
        sketch.execute_threaded_root_contained_cancellable(runtime, config, token),
    )
    .await
    .expect("worker containment exceeded outer bound")
}

async fn assert_clean(compiler: &SketchCompiler, sketch: &Arc<kernal_api::wasm::AdmittedSketch>) {
    sketch.close_threaded_root().expect("close sketch");
    async_engine::timeout(OUTER_BOUND, async {
        loop {
            let worker = sketch.worker_execution_snapshot();
            if worker.live_workers == 0
                && worker.live_protocol_tasks == 0
                && worker.pending_root_leases == 0
            {
                break;
            }
            async_engine::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("worker cleanup exceeded outer bound");
    assert_eq!(compiler.execution_limits_snapshot(), Default::default());
    let worker = sketch.worker_execution_snapshot();
    assert_eq!(worker.live_workers, 0);
    assert_eq!(worker.live_protocol_tasks, 0);
    assert_eq!(worker.pending_root_leases, 0);
}

fn run_case(
    bytes: Vec<u8>,
    deadline: Duration,
    fuel: SketchFuelLimits,
    cancel: bool,
    expected: SketchWorkerTerminal,
) {
    let compiler = compiler(deadline, fuel);
    let sketch = admit(&compiler, bytes);
    let config = worker_config();
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.run(async {
        let source = CancellationSource::new();
        let task = runtime.handle().launch({
            let sketch = Arc::clone(&sketch);
            let config = config.clone();
            let source = source.clone();
            let handle = runtime.handle();
            async move { contained(&sketch, handle, &config, Some(source)).await }
        });
        if cancel {
            async_engine::sleep(Duration::from_millis(10)).await;
            source.cancel();
        }
        assert_eq!(task.await.expect("contained task"), expected);
        assert_clean(&compiler, &sketch).await;
    });
}

#[test]
fn real_worker_classifies_normal_and_trap() {
    run_case(
        threaded_fixture::threaded_root_wasm(None, false, false, false),
        DEADLINE,
        normal_fuel(),
        false,
        SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started),
    );
    run_case(
        threaded_fixture::threaded_root_wasm(Some(0), false, false, false),
        DEADLINE,
        normal_fuel(),
        false,
        SketchWorkerTerminal::Completed(ThreadedRootOutcome::Exited),
    );
    run_case(
        threaded_fixture::unreachable_root_wasm(),
        DEADLINE,
        normal_fuel(),
        false,
        SketchWorkerTerminal::Execution(SketchExecutionError::Trapped),
    );
}

#[test]
fn real_worker_classifies_fuel_cancellation_and_deadline() {
    run_case(
        threaded_fixture::looping_root_wasm(),
        Duration::from_secs(2),
        tiny_fuel(),
        false,
        SketchWorkerTerminal::Execution(SketchExecutionError::OutOfFuel),
    );
    run_case(
        threaded_fixture::looping_root_wasm(),
        Duration::from_secs(2),
        long_fuel(),
        true,
        SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled),
    );
    run_case(
        threaded_fixture::looping_root_wasm(),
        DEADLINE,
        long_fuel(),
        false,
        SketchWorkerTerminal::Stopped(SketchWorkerStopReason::DeadlineExceeded),
    );
}

#[test]
fn real_worker_forces_containment_for_atomic_wait() {
    run_case(
        threaded_fixture::atomic_wait32_wasm(),
        DEADLINE,
        long_fuel(),
        false,
        SketchWorkerTerminal::ForcedContainment {
            trigger: SketchWorkerStopReason::DeadlineExceeded,
        },
    );
}

#[test]
fn real_worker_sequential_stress_leaves_no_parent_state() {
    for _ in 0..3 {
        run_case(
            threaded_fixture::threaded_root_wasm(None, false, false, false),
            DEADLINE,
            normal_fuel(),
            false,
            SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started),
        );
        run_case(
            threaded_fixture::atomic_wait32_wasm(),
            DEADLINE,
            long_fuel(),
            false,
            SketchWorkerTerminal::ForcedContainment {
                trigger: SketchWorkerStopReason::DeadlineExceeded,
            },
        );
        run_case(
            threaded_fixture::unreachable_root_wasm(),
            DEADLINE,
            normal_fuel(),
            false,
            SketchWorkerTerminal::Execution(SketchExecutionError::Trapped),
        );
        run_case(
            threaded_fixture::looping_root_wasm(),
            Duration::from_secs(2),
            long_fuel(),
            true,
            SketchWorkerTerminal::Stopped(SketchWorkerStopReason::Cancelled),
        );
    }
}
