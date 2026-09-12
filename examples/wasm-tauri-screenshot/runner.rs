//! Source-tree CLI for the actual Wasm screenshot application.
use kernal_api::async_engine::{CancellationSource, RuntimeBuilder};
use kernal_api::wasm::{SketchCompiler, SketchCompilerConfig, SketchModulePolicy};
use kernal_api::webview::{ExternalWebviewHost, WebviewUrlGrant};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;
mod status;

fn build_guest() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"));
    let guest = repo.join("examples/wasm-tauri-screenshot/guest");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("target/screenshot-proof"));
    let target = std::path::absolute(target)?.join("kernal-api-wasm-tauri-guest");
    let built = target.join("wasm32-wasip1-threads/release/kernal-api-wasm-tauri-guest.wasm");
    let admitted = built.with_file_name("kernal-api-wasm-tauri-guest.admitted.wasm");
    if !Command::new("soldr")
        .current_dir(guest)
        .env("SOLDR_LINKER", "default")
        .args([
            "--no-cache",
            "cargo",
            "build",
            "--locked",
            "--release",
            "--target",
            "wasm32-wasip1-threads",
            "--target-dir",
        ])
        .arg(&target)
        .status()?
        .success()
    {
        return Err("guest-build-failed".into());
    }
    std::fs::copy(built, &admitted)?;
    if !Command::new("soldr")
        .current_dir(repo)
        .args([
            "cargo",
            "run",
            "--locked",
            "--manifest-path",
            "tools/wasm-abi-generator/Cargo.toml",
            "--",
            "--embed-threaded-metadata",
        ])
        .arg(&admitted)
        .status()?
        .success()
    {
        return Err("guest-metadata-failed".into());
    }
    Ok(admitted)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args_os().skip(1);
    let (mut url, mut output, mut module) = (None, None, None);
    while let Some(flag) = arguments.next() {
        let value = arguments.next().ok_or("option requires a value")?;
        match flag.to_str() {
            Some("--url") if url.is_none() => url = Some(value),
            Some("--output") if output.is_none() => output = Some(PathBuf::from(value)),
            Some("--module") if module.is_none() => module = Some(PathBuf::from(value)),
            _ => return Err("usage: kernal-api-wasm-tauri --url <http(s)-url> --output <png> [--module <built-wasm>]".into()),
        }
    }
    let url = url.ok_or("--url is required")?;
    let grant = WebviewUrlGrant::new(url.to_str().ok_or("URL must be UTF-8")?)?;
    let output = std::path::absolute(output.ok_or("--output is required")?)?;
    let parent = std::fs::canonicalize(output.parent().ok_or("output needs a parent")?)?;
    let output = parent.join(output.file_name().ok_or("output needs a filename")?);
    if output.is_dir() {
        return Err("output must be a file".into());
    }
    let module = match module {
        Some(module) => module,
        None => build_guest()?,
    };
    // Bound the native module upload independently of any guest blob budget.
    use std::io::Read as _;
    const MAX_MODULE_BYTES: usize = 32 * 1024 * 1024;
    let mut bytes = Vec::new();
    std::fs::File::open(module)?
        .take((MAX_MODULE_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let policy = SketchModulePolicy::threaded_rust_v1(MAX_MODULE_BYTES, 16_384)?;
    let compiler = SketchCompiler::new(SketchCompilerConfig::default())?;
    let sketch = compiler.admit(&bytes, policy)?;
    let runtime = RuntimeBuilder::multi_thread().enable_all().build()?;
    let host = ExternalWebviewHost::new(runtime.handle())?;
    let client = host.client();
    let handle = runtime.handle();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let task = handle.clone().launch(async move {
        let result = sketch
            .execute_threaded_root_with_webview(
                handle,
                CancellationSource::new().token(),
                client.clone(),
                grant,
                output,
            )
            .await;
        #[cfg(feature = "tauri-webview-test-support")]
        {
            let (events, omitted) = client.test_trace();
            for event in events {
                eprint!("kernal-webview-trace phase={} elapsed_us={} opcode={}", event.phase, event.elapsed.as_micros(), event.opcode.unwrap_or(0));
                if let Some(counts) = event.observation {
                    eprint!(" clocks={} output_jobs={}", counts.active_clocks, counts.active_output_jobs);
                    eprint!(" captures={} opens={} blobs={} transfer_bytes={} backings={} resources={} operations={}", counts.active_native_captures, counts.active_native_opens, counts.live_blobs, counts.retained_transfer_capacity, counts.native_backings, counts.live_resources, counts.pending_operations);
                }
                eprintln!();
            }
            drop(sketch);
            let counts = compiler.execution_limits_snapshot();
            eprintln!("kernal-webview-trace phase=trace-end omitted={} roots={} threads={} stores={} instances={} epochs={} memory_bytes={}", omitted, counts.active_root_executions(), counts.live_guest_threads(), counts.live_stores(), counts.live_instances(), counts.active_epoch_registrations(), counts.reserved_shared_memory_bytes());
        }
        let _ = sender.send(result);
        let _ = client.request_exit();
    });
    host.run();
    let result = receiver.recv_timeout(Duration::from_secs(60))?;
    runtime.run(task)?;
    result.map_err(|error| {
        if let kernal_api::wasm::SketchExecutionError::NonzeroExit { code } = error {
            if let Some(failure) = status::Failure::from_code(code as u32) {
                debug_assert_eq!(failure.code(), code as u32);
                return std::io::Error::other(format!(
                    "screenshot-{}-{}",
                    failure.step.name(),
                    failure.cause.name()
                ));
            }
            return std::io::Error::other(format!("nonzero-exit:{code}"));
        }
        std::io::Error::other(error.code())
    })?;
    Ok(())
}
