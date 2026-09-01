#![cfg(feature = "wasm-sketch-worker")]

//! Real-worker containment coverage for #28, including ignored inner helpers
//! and externally controlled crash/parent-death proofs on native Windows and
//! Linux targets.

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
// Process launch and the first Wasmtime initialization are intentionally
// outside the sub-second deadline used to prove forced containment below.
// Keep ordinary guest outcomes on a generous bound so a cold Windows worker
// cannot be misclassified as a deadline expiry.
const WORKER_DEADLINE: Duration = Duration::from_secs(2);
const CONTAINMENT_DEADLINE: Duration = Duration::from_secs(1);
// The externally controlled crash and parent-death proofs must acquire an
// exact live native identity before their intentional action.  Keep their
// worker deadline beyond the outer acquisition bound so normal containment
// cannot race the proof into a false success.
const FAILURE_PROOF_DEADLINE: Duration = Duration::from_secs(30);
const GRACE: Duration = Duration::from_secs(1);

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
    SketchFuelLimits::new(30_000, 10_000, 10_000).expect("tiny fuel")
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
            // Let the parent finish the bounded upload and the child enter
            // Wasm before checking cooperative cancellation. A cancellation
            // during protocol upload intentionally exercises forced cleanup.
            async_engine::sleep(Duration::from_millis(500)).await;
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
        WORKER_DEADLINE,
        normal_fuel(),
        false,
        SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started),
    );
    run_case(
        threaded_fixture::threaded_root_wasm(Some(0), false, false, false),
        WORKER_DEADLINE,
        normal_fuel(),
        false,
        SketchWorkerTerminal::Completed(ThreadedRootOutcome::Exited),
    );
    run_case(
        threaded_fixture::unreachable_root_wasm(),
        WORKER_DEADLINE,
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
        CONTAINMENT_DEADLINE,
        long_fuel(),
        false,
        SketchWorkerTerminal::Stopped(SketchWorkerStopReason::DeadlineExceeded),
    );
}

#[test]
fn real_worker_forces_containment_for_atomic_wait() {
    run_case(
        threaded_fixture::atomic_wait32_wasm(),
        CONTAINMENT_DEADLINE,
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
            WORKER_DEADLINE,
            normal_fuel(),
            false,
            SketchWorkerTerminal::Completed(ThreadedRootOutcome::Started),
        );
        run_case(
            threaded_fixture::atomic_wait32_wasm(),
            CONTAINMENT_DEADLINE,
            long_fuel(),
            false,
            SketchWorkerTerminal::ForcedContainment {
                trigger: SketchWorkerStopReason::DeadlineExceeded,
            },
        );
        run_case(
            threaded_fixture::unreachable_root_wasm(),
            WORKER_DEADLINE,
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

// These are deliberately a second, externally controlled process layer.  The
// worker's environment is explicit-empty; only this parent harness receives
// the marker paths.  Do not remove `--ignored` from the outer invocations.
#[cfg(feature = "wasm-sketch-worker-test-support")]
mod failure_proof {
    use super::*;
    use std::fs;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use std::process::Command;
    use std::time::Instant;
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    const MARKER: &str = "KERNAL_API_WASM_WORKER_IDENTITY_MARKER";
    const RESULT: &str = "KERNAL_API_WASM_WORKER_FAILURE_RESULT";
    const RELEASE: &str = "KERNAL_API_WASM_WORKER_FAILURE_RELEASE";

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    struct Artifacts {
        root: std::path::PathBuf,
        marker: std::path::PathBuf,
        result: std::path::PathBuf,
        release: std::path::PathBuf,
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    impl Artifacts {
        fn new() -> Self {
            let unique = format!(
                "kernal-api-d4-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("clock")
                    .as_nanos()
            );
            let root = std::env::temp_dir().join(unique);
            fs::create_dir(&root).expect("artifact directory");
            Self {
                marker: root.join("identity"),
                result: root.join("result"),
                release: root.join("release"),
                root,
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    impl Drop for Artifacts {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[derive(Clone, Copy)]
    struct Identity {
        pid: u32,
        a: u64,
        b: u64,
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn decode_marker(path: &std::path::Path) -> Option<Identity> {
        let text = fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        (lines.next()? == "kernal-api-worker-identity-v1").then_some(())?;
        let mut number = |key| -> Option<u64> { lines.next()?.strip_prefix(key)?.parse().ok() };
        let value = Identity {
            pid: number("pid=")?.try_into().ok()?,
            a: number("creation-a=")?,
            b: number("creation-b=")?,
        };
        lines.next().is_none().then_some(value)
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn wait_for_marker(path: &std::path::Path) -> Identity {
        let deadline = Instant::now() + OUTER_BOUND;
        while Instant::now() < deadline {
            if path.exists() {
                if let Some(value) = decode_marker(path) {
                    return value;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("worker identity marker was not published")
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    struct InnerChild(Option<std::process::Child>);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    impl InnerChild {
        fn wait_success(&mut self) {
            let deadline = Instant::now() + OUTER_BOUND;
            let child = self.0.as_mut().expect("inner child");
            let status = loop {
                if let Some(status) = child.try_wait().expect("inner exit") {
                    break status;
                }
                assert!(Instant::now() < deadline, "inner child exceeded bound");
                std::thread::sleep(Duration::from_millis(10));
            };
            assert!(status.success());
            self.0 = None;
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    impl Drop for InnerChild {
        fn drop(&mut self) {
            let Some(child) = self.0.as_mut() else {
                return;
            };
            let _ = child.kill();
            let deadline = Instant::now() + OUTER_BOUND;
            while Instant::now() < deadline {
                if child.try_wait().ok().flatten().is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn launch(inner: &str, files: &Artifacts) -> InnerChild {
        let worker = PathBuf::from(env!("CARGO_BIN_EXE_kernal-wasm-worker"));
        assert!(worker.is_absolute(), "real worker path must be absolute");
        InnerChild(Some(
            Command::new(std::env::current_exe().expect("test executable"))
                .args(["--exact", inner, "--ignored", "--nocapture"])
                .env(MARKER, &files.marker)
                .env(RESULT, &files.result)
                .env(RELEASE, &files.release)
                .env("KERNAL_API_D4_REAL_WORKER", worker)
                .spawn()
                .expect("inner harness"),
        ))
    }
    fn inner_crash() {
        let compiler = compiler(FAILURE_PROOF_DEADLINE, long_fuel());
        let sketch = admit(&compiler, threaded_fixture::atomic_wait32_wasm());
        let config = SketchWorkerConfig::new(
            PathBuf::from(std::env::var_os("KERNAL_API_D4_REAL_WORKER").expect("worker")),
            GRACE,
        )
        .expect("config");
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let actual =
            runtime.run(async { contained(&sketch, runtime.handle(), &config, None).await });
        fs::write(
            std::env::var_os(RESULT).expect("result"),
            if actual
                == SketchWorkerTerminal::Failure(
                    kernal_api::wasm::SketchWorkerFailure::UnexpectedExit,
                )
            {
                "unexpected-exit"
            } else {
                "wrong-terminal"
            },
        )
        .expect("result");
        runtime.run(async { assert_clean(&compiler, &sketch).await });
    }
    fn inner_parent_death() {
        let compiler = compiler(FAILURE_PROOF_DEADLINE, long_fuel());
        let sketch = admit(&compiler, threaded_fixture::atomic_wait32_wasm());
        let config = SketchWorkerConfig::new(
            PathBuf::from(std::env::var_os("KERNAL_API_D4_REAL_WORKER").expect("worker")),
            GRACE,
        )
        .expect("config");
        let runtime = RuntimeBuilder::current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let handle = runtime.handle();
        let _task = runtime.handle().launch(async move {
            let _ = sketch
                .execute_threaded_root_contained_cancellable(
                    handle,
                    &config,
                    CancellationSource::new().token(),
                )
                .await;
        });
        let release = std::path::PathBuf::from(std::env::var_os(RELEASE).expect("release"));
        runtime.run(async {
            let deadline = Instant::now() + OUTER_BOUND;
            while !release.exists() && Instant::now() < deadline {
                async_engine::sleep(Duration::from_millis(10)).await;
            }
            assert!(
                release.exists(),
                "outer harness did not release parent-death inner process"
            );
        });
        std::process::exit(0);
    }

    #[test]
    #[ignore]
    fn d4_inner_crash_exact_identity() {
        inner_crash();
    }
    #[test]
    #[ignore]
    fn d4_inner_parent_death_exact_identity() {
        inner_parent_death();
    }

    #[cfg(target_os = "linux")]
    fn linux_identity(pid: u32) -> Option<Identity> {
        let text = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let close = text.rfind(')')?;
        let fields: Vec<_> = text[close + 1..].split_whitespace().collect();
        Some(Identity {
            pid,
            a: fields.get(19)?.parse().ok()?,
            b: 0,
        })
    }
    #[cfg(target_os = "linux")]
    struct CloseOnlyPidFd(Option<i32>);
    #[cfg(target_os = "linux")]
    impl Drop for CloseOnlyPidFd {
        fn drop(&mut self) {
            if let Some(fd) = self.0.take() {
                unsafe {
                    libc::close(fd);
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    impl CloseOnlyPidFd {
        fn promote(mut self) -> ArmedPidFd {
            ArmedPidFd(self.0.take())
        }
    }
    #[cfg(target_os = "linux")]
    struct ArmedPidFd(Option<i32>);
    #[cfg(target_os = "linux")]
    impl ArmedPidFd {
        fn wait_gone(&mut self) {
            let fd = self.0.expect("pidfd");
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            assert!(
                unsafe { libc::poll(&mut poll, 1, OUTER_BOUND.as_millis() as i32) } > 0,
                "exact worker survived bound"
            );
            unsafe {
                libc::close(fd);
            }
            self.0 = None;
        }
    }
    #[cfg(target_os = "linux")]
    impl Drop for ArmedPidFd {
        fn drop(&mut self) {
            let Some(fd) = self.0.take() else {
                return;
            };
            let _ = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd,
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            };
            let mut poll = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let _ = unsafe { libc::poll(&mut poll, 1, OUTER_BOUND.as_millis() as i32) };
            unsafe {
                libc::close(fd);
            }
        }
    }
    #[cfg(target_os = "linux")]
    fn pidfd_open(identity: Identity) -> ArmedPidFd {
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid, 0) as i32 };
        let close_only = CloseOnlyPidFd((fd >= 0).then_some(fd));
        assert!(
            close_only.0.is_some(),
            "pidfd_open: {}",
            std::io::Error::last_os_error()
        );
        assert!(
            matches!(linux_identity(identity.pid), Some(now) if now.a == identity.a && now.b == identity.b),
            "PID was reused"
        );
        close_only.promote()
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn d4_crash_reaps_exact_worker() {
        let files = Artifacts::new();
        let mut inner = launch("failure_proof::d4_inner_crash_exact_identity", &files);
        let identity = wait_for_marker(&files.marker);
        let mut fd = pidfd_open(identity);
        assert_eq!(
            unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    fd.0.expect("pidfd"),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0,
                )
            },
            0,
            "pidfd signal"
        );
        fd.wait_gone();
        inner.wait_success();
        assert_eq!(
            fs::read_to_string(&files.result).expect("result"),
            "unexpected-exit"
        );
        assert!(linux_identity(identity.pid)
            .is_none_or(|now| now.a != identity.a || now.b != identity.b));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn d4_parent_death_kills_exact_worker() {
        let files = Artifacts::new();
        let mut inner = launch(
            "failure_proof::d4_inner_parent_death_exact_identity",
            &files,
        );
        let identity = wait_for_marker(&files.marker);
        let mut fd = pidfd_open(identity);
        fs::write(&files.release, "go").expect("release");
        inner.wait_success();
        fd.wait_gone();
        assert!(linux_identity(identity.pid)
            .is_none_or(|now| now.a != identity.a || now.b != identity.b));
    }
    #[cfg(target_os = "windows")]
    struct CloseOnlyWindowsHandle(Option<windows_sys::Win32::Foundation::HANDLE>);
    #[cfg(target_os = "windows")]
    impl Drop for CloseOnlyWindowsHandle {
        fn drop(&mut self) {
            if let Some(handle) = self.0.take() {
                unsafe {
                    windows_sys::Win32::Foundation::CloseHandle(handle);
                }
            }
        }
    }
    #[cfg(target_os = "windows")]
    impl CloseOnlyWindowsHandle {
        fn promote(mut self) -> ArmedWindowsProcess {
            ArmedWindowsProcess(self.0.take())
        }
    }
    #[cfg(target_os = "windows")]
    struct ArmedWindowsProcess(Option<windows_sys::Win32::Foundation::HANDLE>);
    #[cfg(target_os = "windows")]
    impl ArmedWindowsProcess {
        fn handle(&self) -> windows_sys::Win32::Foundation::HANDLE {
            self.0.expect("process handle")
        }
        fn wait_gone(&mut self) {
            use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
            use windows_sys::Win32::System::Threading::WaitForSingleObject;
            assert_eq!(
                unsafe { WaitForSingleObject(self.handle(), OUTER_BOUND.as_millis() as u32) },
                WAIT_OBJECT_0,
                "exact worker survived bound"
            );
            unsafe {
                CloseHandle(self.handle());
            }
            self.0 = None;
        }
    }
    #[cfg(target_os = "windows")]
    impl Drop for ArmedWindowsProcess {
        fn drop(&mut self) {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
            let Some(process) = self.0.take() else {
                return;
            };
            let _ = unsafe { TerminateProcess(process, 1) };
            let _ = unsafe { WaitForSingleObject(process, OUTER_BOUND.as_millis() as u32) };
            unsafe {
                CloseHandle(process);
            }
        }
    }
    #[cfg(target_os = "windows")]
    fn windows_handle(identity: Identity, access: u32) -> ArmedWindowsProcess {
        use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess};
        let close_only =
            CloseOnlyWindowsHandle(Some(unsafe { OpenProcess(access, 0, identity.pid) }));
        if close_only.0.expect("owned handle").is_null() {
            let error = std::io::Error::last_os_error();
            panic!("OpenProcess: {error}");
        }
        assert!(
            !close_only.0.expect("owned handle").is_null(),
            "OpenProcess unexpectedly returned a null handle"
        );
        let mut creation = unsafe { std::mem::zeroed() };
        let mut exit = unsafe { std::mem::zeroed() };
        let mut kernel = unsafe { std::mem::zeroed() };
        let mut user = unsafe { std::mem::zeroed() };
        assert_ne!(
            unsafe {
                GetProcessTimes(
                    close_only.0.expect("owned handle"),
                    &mut creation,
                    &mut exit,
                    &mut kernel,
                    &mut user,
                )
            },
            0,
            "GetProcessTimes"
        );
        let created = ((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64;
        assert_eq!((created, 0), (identity.a, identity.b), "PID was reused");
        close_only.promote()
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn d4_crash_reaps_exact_worker() {
        use windows_sys::Win32::System::Threading::{
            TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        };
        const SYNCHRONIZE: u32 = 0x0010_0000;
        let files = Artifacts::new();
        let mut inner = launch("failure_proof::d4_inner_crash_exact_identity", &files);
        let identity = wait_for_marker(&files.marker);
        let mut process = windows_handle(
            identity,
            PROCESS_TERMINATE | SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION,
        );
        // The long proof deadline keeps normal containment out of this
        // acquisition/action window. This exact live handle is the worker we
        // intentionally crash, then wait as proof of its disappearance.
        assert_ne!(
            unsafe { TerminateProcess(process.handle(), 1) },
            0,
            "TerminateProcess"
        );
        process.wait_gone();
        inner.wait_success();
        assert_eq!(
            fs::read_to_string(&files.result).expect("result"),
            "unexpected-exit"
        );
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn d4_parent_death_kills_exact_worker() {
        use windows_sys::Win32::System::Threading::PROCESS_QUERY_LIMITED_INFORMATION;
        const SYNCHRONIZE: u32 = 0x0010_0000;
        let files = Artifacts::new();
        let mut inner = launch(
            "failure_proof::d4_inner_parent_death_exact_identity",
            &files,
        );
        let identity = wait_for_marker(&files.marker);
        let mut process = windows_handle(identity, SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION);
        fs::write(&files.release, "go").expect("release");
        inner.wait_success();
        // Release the inner parent only after this creation-validated worker
        // handle is live; owner death must make this exact handle signal.
        process.wait_gone();
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "macOS owner-death evidence requires a native supervisor trace"]
    fn d4_macos_native_evidence() {}
}
