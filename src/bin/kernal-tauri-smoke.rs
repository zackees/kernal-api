//! Native raw-Wry lifecycle proof for the private `tauri-webview` backend.
//!
//! Run on Linux with the GTK/WebKit/Xvfb shell documented in issue #18.  This
//! binary exists because a Rust unit test executes on a test-worker thread,
//! while Tauri requires the event loop to be initialized by the process main
//! thread on every supported desktop host.

#[path = "../tauri.rs"]
mod tauri;

pub use kernal_api::async_engine;

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use tauri::{NativeWebviewBackend, NativeWebviewError, NativeWebviewLoop, NativeWebviewRequest};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request)?;
        stream.write_all(
            b"HTTP/1.0 200 OK\r\nContent-Type: text/html\r\n\r\n<!doctype html><title>kernal-api</title>",
        )?;
        Ok(())
    });

    let runtime = async_engine::RuntimeBuilder::multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()?;
    let (event_loop, backend) = NativeWebviewLoop::new(runtime.handle()).map_err(host_error)?;
    let url = format!("http://{address}/finished");
    let (result_sender, result_receiver) = mpsc::channel();
    let task_backend = backend.clone();
    runtime
        .handle()
        .launch(async move {
            let result = lifecycle(&task_backend, &url).await;
            let _ = result_sender.send(result);
            let _ = task_backend.request_exit();
        })
        .detach();

    // This process main thread owns the Wry/Tauri event loop. The awaited
    // operation above uses the same caller-provided kernal-api runtime.
    event_loop.run();
    server.join().map_err(|_| "loopback server panicked")??;
    result_receiver
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "webview lifecycle task did not exit")?
        .map_err(host_error)?;
    Ok(())
}

async fn lifecycle(backend: &NativeWebviewBackend, url: &str) -> Result<(), NativeWebviewError> {
    let request = NativeWebviewRequest::parse(url)?;
    let mut webview = backend.open(request).await?;
    let terminal = webview.wait_until_terminal()?;
    let loaded = webview.wait_until_loaded()?;
    loaded
        .await
        .map_err(|_| NativeWebviewError::HostFailure("load completion dropped".into()))??;
    webview.close()?;
    webview
        .wait_until_closed()?
        .await
        .map_err(|_| NativeWebviewError::HostFailure("close completion dropped".into()))?;
    match terminal
        .await
        .map_err(|_| NativeWebviewError::HostFailure("terminal completion dropped".into()))?
    {
        Err(NativeWebviewError::WindowClosed) => Ok(()),
        other => Err(NativeWebviewError::HostFailure(format!(
            "unexpected terminal result: {other:?}"
        ))),
    }
}

fn host_error(error: NativeWebviewError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
