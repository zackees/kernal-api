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
    let page = scenario.page(&address.to_string());
    let server = std::thread::spawn(move || -> std::io::Result<()> {
        listener.set_nonblocking(true)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(16);
        let accept = || loop {
            match listener.accept() {
                Ok(connection) => return Ok(connection),
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "webview did not complete loopback isolation proof",
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => return Err(error),
            }
        };
        let (mut stream, _) = accept()?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = [0_u8; 4096];
        let _ = stream.read(&mut request)?;
        write!(
            stream,
            "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
            page.len()
        )?;
        let (mut report, _) = accept()?;
        report.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut report_request = [0_u8; 4096];
        let read = report.read(&mut report_request)?;
        let report_request = String::from_utf8_lossy(&report_request[..read]);
        if !report_request.starts_with("GET /_isolation?ipc=0&tauri=0&platform=0 ") {
            return Err(std::io::Error::other(format!(
                "page observed a prohibited host bridge: {report_request:?}"
            )));
        }
        report.write_all(b"HTTP/1.0 204 No Content\r\nContent-Length: 0\r\n\r\n")?;
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
            // Keep the whole operation bounded.  A load callback that never
            // arrives must not leave the Wry event loop running forever in CI
            // or in an embedding shell.  Dropping `lifecycle` drops a created
            // NativeWebview, whose Drop requests native close.
            let result = async_engine::timeout(
                Duration::from_secs(15),
                lifecycle(&task_backend, &url, scenario),
            )
            .await
            .map_err(|_| NativeWebviewError::HostFailure("lifecycle timed out".into()))
            .and_then(|result| result);
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
            ) if reason == "tauri" => {}
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

    fn page(self, address: &str) -> String {
        let action = match self {
            Self::Close => "",
            // WebKit requires a genuine user activation before it invokes the
            // new-window callback. The Linux Xvfb proof clicks this link with
            // xdotool; no host script or IPC is injected into the page.
            Self::Popup => "",
            Self::ProhibitedRedirect => "location.href = 'tauri://localhost/';",
        };
        format!(
            "<!doctype html><title>kernal-api</title>\
             <a id=\"popup\" target=\"_blank\" href=\"https://example.test/\" \
             style=\"display:block;position:absolute;left:100px;top:100px;width:200px;height:100px\">popup</a><script>\
             const probe = `ipc=${{Number(typeof window.ipc !== 'undefined')}}&\
             tauri=${{Number(typeof window.__TAURI_INTERNALS__ !== 'undefined')}}&\
             platform=${{Number(Boolean(window.webkit?.messageHandlers?.ipc))}}`;\
             const xhr = new XMLHttpRequest();\
             xhr.open('GET', 'http://{address}/_isolation?' + probe, false); xhr.send();\
             {action}</script>"
        )
    }
}

fn host_error(error: NativeWebviewError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
