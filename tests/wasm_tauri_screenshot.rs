#![cfg(feature = "wasm-sketch-host")]

use kernal_api::async_engine::RuntimeBuilder;
use kernal_api::wasm::{
    SketchCompiler, SketchCompilerConfig, SketchExecutionError, SketchModuleError,
    SketchModulePolicy,
};

#[cfg(feature = "tauri-webview-test-support")]
#[test]
#[ignore = "requires a native display and the actual built screenshot guest"]
fn actual_screenshot_cli_runs_the_offline_guest_and_commits_only_its_output() {
    run_native_screenshot_proof(false);
}

#[cfg(feature = "tauri-webview-test-support")]
#[test]
#[ignore = "requires a native display and the actual built screenshot guest"]
fn actual_screenshot_guest_rejects_redirect_and_drains_without_output() {
    run_native_screenshot_proof(true);
}

#[cfg(feature = "tauri-webview-test-support")]
fn run_native_screenshot_proof(redirect: bool) {
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
                        if redirect {
                            let _ = write!(stream, "HTTP/1.1 302 Found\r\nLocation: tauri://localhost/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        } else {
                            let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len());
                        }
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
    // Opt-in diagnostic retention is outside the exact-output directory, so
    // logs cannot accidentally weaken the no-extra-output assertion below.
    let retained = std::env::var_os("KERNAL_API_SCREENSHOT_PROOF_DIR").map(|root| {
        std::fs::create_dir_all(&root).unwrap();
        tempfile::Builder::new()
            .prefix("screenshot-")
            .tempdir_in(root)
            .unwrap()
            .keep()
    });
    let temporary = tempfile::tempdir().unwrap();
    let proof = retained.as_deref().unwrap_or(temporary.path());
    let directory = proof.join("output");
    std::fs::create_dir(&directory).unwrap();
    let output = directory.join("viewport.png");
    let sentinel = directory.join("untouched");
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
            .stdout(std::fs::File::create(proof.join("runner.stdout.log")).unwrap())
            .stderr(std::fs::File::create(proof.join("runner.stderr.log")).unwrap())
            .spawn()
            .unwrap(),
    );
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() >= Duration::from_secs(90) {
            std::fs::write(proof.join("process.json"), "{\"outcome\":\"timeout\"}\n").unwrap();
            panic!(
                "screenshot runner exceeded its proof bound; diagnostics: {}",
                proof.display()
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    std::fs::write(
        proof.join("process.json"),
        format!(
            "{{\"os\":\"{}\",\"arch\":\"{}\",\"success\":{},\"elapsed_ms\":{}}}\n",
            std::env::consts::OS,
            std::env::consts::ARCH,
            status.success(),
            start.elapsed().as_millis()
        ),
    )
    .unwrap();
    if redirect {
        assert!(!status.success(), "disallowed redirect succeeded");
        let trace = std::fs::read_to_string(proof.join("runner.stderr.log")).unwrap();
        assert!(
            trace.contains("screenshot-load-rejected"),
            "wrong typed redirect error: {trace}"
        );
        validate_teardown_trace(&trace).expect("redirect failure must drain the actual root");
        assert_eq!(std::fs::read(&output).unwrap(), b"original");
        assert_eq!(std::fs::read(&sentinel).unwrap(), b"unchanged");
        assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 2);
        return;
    }
    assert!(
        status.success(),
        "actual guest runner failed: {status}; diagnostics: {}",
        proof.display()
    );
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
    validate_fixture_png(&png).expect("decodable native viewport with expected color regions");
    let trace = std::fs::read_to_string(proof.join("runner.stderr.log")).unwrap();
    validate_execution_trace(&trace).expect("generated ABI, native timing, and drained root trace");
    assert_eq!(std::fs::read(sentinel).unwrap(), b"unchanged");
    assert_eq!(
        std::fs::read_dir(&directory).unwrap().count(),
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

#[cfg(feature = "tauri-webview-test-support")]
fn validate_execution_trace(trace: &str) -> Result<(), &'static str> {
    fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        line.split_ascii_whitespace().find_map(|part| {
            let (name, value) = part.split_once('=')?;
            (name == key).then_some(value)
        })
    }
    let records: Vec<_> = trace
        .lines()
        .filter(|line| line.starts_with("kernal-webview-trace "))
        .collect();
    let one = |phase: &str| -> Result<&str, &'static str> {
        let matching: Vec<_> = records
            .iter()
            .copied()
            .filter(|line| field(line, "phase") == Some(phase))
            .collect();
        if matching.len() != 1 {
            return Err("missing or duplicate lifecycle event");
        }
        Ok(matching[0])
    };
    let time = |phase| -> Result<u128, &'static str> {
        field(one(phase)?, "elapsed_us")
            .and_then(|value| value.parse().ok())
            .ok_or("invalid event timestamp")
    };
    if time("capture-requested")?
        .checked_sub(time("load-finished")?)
        .is_none_or(|delta| delta < 5_000_000)
    {
        return Err("capture preceded the five-second post-load wait");
    }
    for opcode in [13, 11, 14, 15, 12, 16, 10, 17] {
        if !records.iter().any(|line| {
            field(line, "phase") == Some("submit")
                && field(line, "opcode").and_then(|v| v.parse::<u32>().ok()) == Some(opcode)
        }) {
            return Err("missing generated operation crossing");
        }
    }
    for phase in ["poll", "yield"] {
        if !records
            .iter()
            .any(|line| field(line, "phase") == Some(phase))
        {
            return Err("missing async lifecycle crossing");
        }
    }
    validate_teardown_trace(trace)
}

#[cfg(feature = "tauri-webview-test-support")]
fn validate_teardown_trace(trace: &str) -> Result<(), &'static str> {
    fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
        line.split_ascii_whitespace().find_map(|part| {
            let (name, value) = part.split_once('=')?;
            (name == key).then_some(value)
        })
    }
    let one = |phase: &str| -> Result<&str, &'static str> {
        let matching: Vec<_> = trace
            .lines()
            .filter(|line| {
                line.starts_with("kernal-webview-trace ") && field(line, "phase") == Some(phase)
            })
            .collect();
        if matching.len() != 1 {
            return Err("missing or duplicate teardown event");
        }
        Ok(matching[0])
    };
    let drained = one("hub-drained")?;
    for counter in [
        "clocks",
        "output_jobs",
        "captures",
        "opens",
        "blobs",
        "transfer_bytes",
        "backings",
        "resources",
        "operations",
    ] {
        if field(drained, counter) != Some("0") {
            return Err("root retains native or semantic resources");
        }
    }
    let end = one("trace-end")?;
    for counter in [
        "omitted",
        "roots",
        "threads",
        "stores",
        "instances",
        "epochs",
        "memory_bytes",
    ] {
        if field(end, counter) != Some("0") {
            return Err("incomplete trace or retained Wasm state");
        }
    }
    Ok(())
}

#[cfg(feature = "tauri-webview-test-support")]
fn validate_fixture_png(bytes: &[u8]) -> Result<(), &'static str> {
    if bytes.len() > 1024 * 1024 {
        return Err("encoded image exceeds proof limit");
    }
    let mut decoder = png::Decoder::new_with_limits(
        std::io::Cursor::new(bytes),
        png::Limits {
            bytes: 32 * 1024 * 1024,
        },
    );
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|_| "invalid PNG header")?;
    let info = reader.info();
    if info.width < 4
        || info.height < 4
        || u64::from(info.width) * u64::from(info.height) > 4_000_000
    {
        return Err("invalid viewport dimensions");
    }
    if reader.output_buffer_size() > 16_000_000 {
        return Err("decoded image exceeds proof limit");
    }
    let mut pixels = vec![0; reader.output_buffer_size()];
    let frame = reader
        .next_frame(&mut pixels)
        .map_err(|_| "invalid PNG pixels")?;
    let channels = match frame.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return Err("fixture requires RGB color regions"),
    };
    let width = frame.width as usize;
    let height = frame.height as usize;
    for y in [height / 4, height / 2, height * 3 / 4] {
        for (x, expected) in [(width / 4, [255_u8, 0, 0]), (width * 3 / 4, [0, 0, 255])] {
            let offset = (y * width + x) * channels;
            let sample = &pixels[offset..offset + channels];
            if sample[..3]
                .iter()
                .zip(expected)
                .any(|(actual, expected)| actual.abs_diff(expected) > 35)
                || (channels == 4 && sample[3] < 220)
            {
                return Err("unexpected viewport color region");
            }
        }
    }
    Ok(())
}

#[cfg(feature = "tauri-webview-test-support")]
#[test]
fn execution_trace_assertions_reject_early_capture_leaks_and_missing_crossings() {
    // Parser controls only; the ignored acceptance test still requires the
    // actual native callback and actual compiled guest to produce its trace.
    let mut trace = String::new();
    for opcode in [13, 11, 14, 15, 12, 16, 10, 17] {
        trace.push_str(&format!(
            "kernal-webview-trace phase=submit opcode={opcode}\n"
        ));
    }
    trace.push_str("kernal-webview-trace phase=poll\nkernal-webview-trace phase=yield\n");
    trace.push_str("kernal-webview-trace phase=load-finished elapsed_us=100\n");
    trace.push_str("kernal-webview-trace phase=capture-requested elapsed_us=5000100\n");
    trace.push_str("kernal-webview-trace phase=hub-drained clocks=0 output_jobs=0 captures=0 opens=0 blobs=0 transfer_bytes=0 backings=0 resources=0 operations=0\n");
    trace.push_str("kernal-webview-trace phase=trace-end omitted=0 roots=0 threads=0 stores=0 instances=0 epochs=0 memory_bytes=0\n");
    assert_eq!(validate_execution_trace(&trace), Ok(()));
    for (from, to) in [
        ("elapsed_us=5000100", "elapsed_us=5000099"),
        ("resources=0", "resources=1"),
        ("threads=0", "threads=1"),
        ("omitted=0", "omitted=1"),
        ("opcode=16", "opcode=99"),
        ("phase=yield", "phase=unknown"),
    ] {
        assert!(
            validate_execution_trace(&trace.replace(from, to)).is_err(),
            "accepted invalid trace mutation {from} -> {to}"
        );
    }
}

#[cfg(feature = "tauri-webview-test-support")]
#[test]
fn portable_png_assertions_reject_wrong_regions_and_corrupt_pixels() {
    fn fixture(swapped: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 8, 8);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            let pixels: Vec<u8> = (0..64)
                .flat_map(|index| {
                    if (index % 8 < 4) != swapped {
                        [255, 0, 0]
                    } else {
                        [0, 0, 255]
                    }
                })
                .collect();
            writer.write_image_data(&pixels).unwrap();
        }
        bytes
    }
    let valid = fixture(false);
    assert_eq!(validate_fixture_png(&valid), Ok(()));
    assert_eq!(
        validate_fixture_png(&fixture(true)),
        Err("unexpected viewport color region")
    );
    assert!(validate_fixture_png(&valid[..valid.len() / 2]).is_err());
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
        Err(SketchExecutionError::NonzeroExit { code: 17 }),
        "ungranted command must report URL-grant rejection, not trap, time out, or succeed"
    );
    assert_eq!(
        sketch.execution_limits_snapshot().active_root_executions(),
        0
    );
    assert_eq!(compiler.compiled_module_count(), 1);
}
