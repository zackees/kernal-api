#![cfg(feature = "wasm-sketch-host")]

use kernal_api::async_engine::RuntimeBuilder;
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchExecutionError, SketchModuleError,
    SketchModulePolicy,
};

#[cfg(feature = "tauri-webview")]
#[test]
#[ignore = "requires a native display and the actual built screenshot guest"]
fn actual_screenshot_cli_runs_the_offline_guest_and_commits_only_its_output() {
    use std::io::{Read, Write};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    use std::time::{Duration, Instant};
    struct Server {
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let artifact = std::env::var_os("KERNAL_API_SCREENSHOT_ARTIFACT_WASM")
        .expect("actual guest artifact required");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    listener.set_nonblocking(true).unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let thread = std::thread::spawn(move || {
        let page = include_str!("../examples/wasm-tauri-screenshot/fixtures/viewport.html");
        while !stopping.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = [0; 4096];
                    if stream.read(&mut request).is_ok() {
                        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(error) => panic!("fixture server: {error}"),
            }
        }
    });
    let _server = Server {
        stop,
        thread: Some(thread),
    };
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("viewport.png");
    let sentinel = directory.path().join("untouched");
    std::fs::write(&output, b"original").unwrap();
    std::fs::write(&sentinel, b"unchanged").unwrap();
    let start = Instant::now();
    let mut child = Child(
        std::process::Command::new(env!("CARGO_BIN_EXE_kernal-api-wasm-tauri"))
            .arg("--url")
            .arg(format!("http://{address}/"))
            .arg("--output")
            .arg(&output)
            .arg("--module")
            .arg(artifact)
            .spawn()
            .unwrap(),
    );
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(90),
            "screenshot runner exceeded its proof bound"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success(), "actual guest runner failed: {status}");
    assert!(
        start.elapsed() >= Duration::from_secs(5),
        "guest omitted its kernel wait"
    );
    let png = std::fs::read(&output).unwrap();
    assert!(png.len() >= 24 && png.len() <= 1024 * 1024);
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert!(width > 0 && height > 0 && u64::from(width) * u64::from(height) <= 4_000_000);
    assert_eq!(std::fs::read(sentinel).unwrap(), b"unchanged");
    assert_eq!(
        std::fs::read_dir(directory.path()).unwrap().count(),
        2,
        "temporary output survived commit"
    );
    eprintln!(
        "{}-{}: actual Wasm screenshot {}x{}, {} PNG bytes, elapsed {:?}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        width,
        height,
        png.len(),
        start.elapsed()
    );
}

#[test]
#[ignore = "build the actual screenshot guest and supply KERNAL_API_SCREENSHOT_ARTIFACT_WASM"]
fn actual_screenshot_guest_admits_but_cannot_execute_without_webview_grants() {
    let artifact = std::env::var_os("KERNAL_API_SCREENSHOT_ARTIFACT_WASM")
        .expect("explicit actual Rust screenshot artifact required");
    let bytes = std::fs::read(artifact).expect("read screenshot artifact");
    // The build script appends metadata after the real import section. Change
    // the first ABI namespace in place, preserving every section length, and
    // require rejection before compilation rather than broadening the import
    // allowlist to accommodate this smaller application.
    let mut foreign = bytes.clone();
    let offset = foreign
        .windows(b"kernal-api:v1".len())
        .position(|window| window == b"kernal-api:v1")
        .expect("actual generated ABI import namespace");
    foreign[offset..offset + b"kernal-api:v1".len()].copy_from_slice(b"evilxx-api:v1");
    let rejected = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    assert!(matches!(
        rejected.admit(
            &foreign,
            SketchModulePolicy::threaded_rust_v1(foreign.len() + 1, 16_384).unwrap()
        ),
        Err(SketchModuleError::ForbiddenImport { .. }),
    ));
    assert_eq!(rejected.compiled_module_count(), 0);
    let compiler = SketchCompiler::new(SketchCompilerConfig::default()).expect("compiler");
    let policy = SketchModulePolicy::threaded_rust_v1(bytes.len() + 1, 16_384).expect("policy");
    let sketch = compiler
        .admit(&bytes, policy)
        .expect("closed-profile admission");
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let outcome = runtime.run(sketch.execute_threaded_root(runtime.handle()));
    assert_eq!(
        outcome,
        Err(SketchExecutionError::Trapped),
        "ungranted command must trap, not time out or succeed"
    );
    assert_eq!(
        sketch.execution_limits_snapshot().active_root_executions(),
        0
    );
    assert_eq!(compiler.compiled_module_count(), 1);
}
