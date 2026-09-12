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
    let scenario = SmokeScenario::from_args()?;
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let address = listener.local_addr()?;
    let page = scenario.page();
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request)?;
        write!(
            stream,
            "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
            page.len()
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
            let result = lifecycle(&task_backend, &url, scenario).await;
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

async fn lifecycle(
    backend: &NativeWebviewBackend,
    url: &str,
    scenario: SmokeScenario,
) -> Result<(), NativeWebviewError> {
    let request = NativeWebviewRequest::parse(url)?;
    let mut webview = backend.open(request).await?;
    let mut terminal = Some(webview.wait_until_terminal()?);
    let loaded = webview.wait_until_loaded()?;
    loaded
        .await
        .map_err(|_| NativeWebviewError::HostFailure("load completion dropped".into()))??;
    if scenario != SmokeScenario::Close {
        let terminal_result = async_engine::timeout(
            Duration::from_secs(5),
            terminal.take().expect("terminal receiver is present"),
        )
        .await
        .map_err(|_| NativeWebviewError::HostFailure("security callback timed out".into()))?
        .map_err(|_| NativeWebviewError::HostFailure("terminal completion dropped".into()))?;
        match (scenario, terminal_result) {
            (SmokeScenario::Popup, Err(NativeWebviewError::RejectedNavigation(reason)))
                if reason.starts_with("popup to ") => {}
            (
                SmokeScenario::ProhibitedRedirect,
                Err(NativeWebviewError::RejectedNavigation(reason)),
            ) if reason == "file" => {}
            (_, other) => {
                return Err(NativeWebviewError::HostFailure(format!(
                    "unexpected security terminal result: {other:?}"
                )));
            }
        }
    }
    webview.close()?;
    webview
        .wait_until_closed()?
        .await
        .map_err(|_| NativeWebviewError::HostFailure("close completion dropped".into()))?;
    if scenario == SmokeScenario::Close {
        match terminal
            .take()
            .expect("close scenario retains terminal receiver")
            .await
            .map_err(|_| NativeWebviewError::HostFailure("terminal completion dropped".into()))?
        {
            Err(NativeWebviewError::WindowClosed) => {}
            other => {
                return Err(NativeWebviewError::HostFailure(format!(
                    "unexpected close terminal result: {other:?}"
                )));
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmokeScenario {
    Close,
    Popup,
    ProhibitedRedirect,
}

impl SmokeScenario {
    fn from_args() -> Result<Self, Box<dyn std::error::Error>> {
        match std::env::args().nth(1).as_deref() {
            None | Some("close") => Ok(Self::Close),
            Some("popup") => Ok(Self::Popup),
            Some("redirect") => Ok(Self::ProhibitedRedirect),
            Some(other) => Err(format!(
                "unknown smoke scenario {other:?}; use close, popup, or redirect"
            )
            .into()),
        }
    }

    fn page(self) -> &'static str {
        match self {
            Self::Close => "<!doctype html><title>kernal-api</title>",
            Self::Popup => concat!(
                "<!doctype html><title>kernal-api</title><script>",
                "setTimeout(() => window.open('https://example.test/'), 50);",
                "</script>"
            ),
            Self::ProhibitedRedirect => concat!(
                "<!doctype html><title>kernal-api</title><script>",
                "setTimeout(() => location.href = 'file:///etc/passwd', 50);",
                "</script>"
            ),
        }
    }
}

fn host_error(error: NativeWebviewError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
