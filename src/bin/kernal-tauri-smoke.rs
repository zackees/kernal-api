//! Native semantic-webview lifecycle proof for the `tauri-webview` facade.
//!
//! Run on Linux with the GTK/WebKit/Xvfb shell documented in issue #18.  This
//! binary exists because a Rust unit test executes on a test-worker thread,
//! while Tauri requires the event loop to be initialized by the process main
//! thread on every supported desktop host.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::Duration;

use kernal_api::async_engine;
use kernal_api::webview::{ExternalWebviewClient, ExternalWebviewHost, WebviewError};

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
        // Accepted sockets inherit the listener's nonblocking mode on the
        // supported Unix CI hosts.  The proof uses a bounded blocking read so
        // an in-flight navigation cannot race the harness into WouldBlock.
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let mut request = [0_u8; 4096];
        match stream.read(&mut request) {
            Ok(_) => {}
            // A real WebKit navigation may establish its loopback connection
            // just as the semantic timeout tears down the native backing,
            // before it writes HTTP bytes. That is a successful timeout
            // proof, not a server failure.
            Err(error)
                if scenario == SmokeScenario::Timeout
                    && matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        }
        if scenario == SmokeScenario::Timeout {
            // Keep the top-level navigation pending past the facade timeout.
            // The client has already sent a real loopback request, so this
            // proves teardown of an actual native backing rather than a mock.
            std::thread::sleep(Duration::from_secs(1));
            return Ok(());
        }
        write!(
            stream,
            "HTTP/1.0 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{page}",
            page.len()
        )?;
        let (mut report, _) = accept()?;
        report.set_nonblocking(false)?;
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
    let host = ExternalWebviewHost::new(runtime.handle()).map_err(host_error)?;
    let client = host.client();
    let url = format!("http://{address}/finished");
    let (result_sender, result_receiver) = mpsc::channel();
    let task_client = client.clone();
    runtime
        .handle()
        .launch(async move {
            // Keep the whole operation bounded.  A load callback that never
            // arrives must not leave the Wry event loop running forever in CI
            // or in an embedding shell.  Dropping `lifecycle` drops a created
            // NativeWebview, whose Drop requests native close.
            let result = async_engine::timeout(
                Duration::from_secs(15),
                lifecycle(&task_client, &url, scenario),
            )
            .await
            .map_err(|_| WebviewError::TimedOut)
            .and_then(|result| result);
            let _ = result_sender.send(result);
            let _ = task_client.request_exit();
        })
        .detach();

    // This process main thread owns the Wry/Tauri event loop. The awaited
    // operation above uses the same caller-provided kernal-api runtime.
    host.run();
    server.join().map_err(|_| "loopback server panicked")??;
    result_receiver
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "webview lifecycle task did not exit")?
        .map_err(host_error)?;
    Ok(())
}

async fn lifecycle(
    client: &ExternalWebviewClient,
    url: &str,
    scenario: SmokeScenario,
) -> Result<(), WebviewError> {
    let webview = client.open_webview(url).await?;
    if scenario == SmokeScenario::Timeout {
        let timed_out = webview.wait_until_loaded(Duration::from_millis(50)).await;
        if timed_out != Err(WebviewError::TimedOut) {
            return Err(WebviewError::HostFailure(format!(
                "expected typed load timeout, got {timed_out:?}"
            )));
        }
        // The resource token remains opaque, but another semantic operation
        // through this façade must observe the revoked generation.
        if webview.wait_until_terminal(Duration::ZERO).await != Err(WebviewError::WindowClosed) {
            return Err(WebviewError::HostFailure(
                "timed-out handle remained usable".into(),
            ));
        }
        return assert_clean(client);
    }
    let loaded = webview.wait_until_loaded(Duration::from_secs(5)).await;
    match (scenario, loaded) {
        (SmokeScenario::Close, Ok(())) => webview.close().await,
        (SmokeScenario::Cancel, Ok(())) => {
            webview.cancel();
            if webview.wait_until_terminal(Duration::ZERO).await != Err(WebviewError::Cancelled) {
                return Err(WebviewError::HostFailure(
                    "cancellation did not publish its typed terminal outcome".into(),
                ));
            }
            require_stale(&webview).await?;
            assert_clean(client)
        }
        (SmokeScenario::WindowClose, Ok(())) => {
            webview.request_window_close_for_test()?;
            if webview.wait_until_terminal(Duration::from_secs(5)).await
                != Err(WebviewError::WindowClosed)
            {
                return Err(WebviewError::HostFailure(
                    "window close did not revoke the semantic handle".into(),
                ));
            }
            require_stale(&webview).await?;
            assert_clean(client)
        }
        (SmokeScenario::Popup, Ok(())) | (SmokeScenario::ProhibitedRedirect, Ok(())) => {
            match webview.wait_until_terminal(Duration::from_secs(5)).await {
                Err(WebviewError::RejectedNavigation(reason))
                    if reason.contains("popup")
                        || reason.contains("tauri")
                        || reason == "navigation policy" =>
                {
                    Ok(())
                }
                other => Err(WebviewError::HostFailure(format!(
                    "unexpected semantic security terminal: {other:?}"
                ))),
            }
        }
        (SmokeScenario::Popup, Err(WebviewError::RejectedNavigation(reason)))
        | (SmokeScenario::ProhibitedRedirect, Err(WebviewError::RejectedNavigation(reason)))
            if reason.contains("popup")
                || reason.contains("tauri")
                || reason == "navigation policy" =>
        {
            Ok(())
        }
        (_, other) => Err(WebviewError::HostFailure(format!(
            "unexpected semantic webview outcome: {other:?}"
        ))),
    }
}

async fn require_stale(webview: &kernal_api::webview::WebviewHandle) -> Result<(), WebviewError> {
    if webview.wait_until_loaded(Duration::ZERO).await == Err(WebviewError::WindowClosed) {
        Ok(())
    } else {
        Err(WebviewError::HostFailure(
            "revoked webview handle remained usable".into(),
        ))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SmokeScenario {
    Close,
    Popup,
    ProhibitedRedirect,
    Timeout,
    Cancel,
    WindowClose,
}

impl SmokeScenario {
    fn from_args() -> Result<Self, Box<dyn std::error::Error>> {
        match std::env::args().nth(1).as_deref() {
            None | Some("close") => Ok(Self::Close),
            Some("popup") => Ok(Self::Popup),
            Some("redirect") => Ok(Self::ProhibitedRedirect),
            Some("timeout") => Ok(Self::Timeout),
            Some("cancel") => Ok(Self::Cancel),
            Some("window-close") => Ok(Self::WindowClose),
            Some(other) => Err(format!(
                "unknown smoke scenario {other:?}; use close, popup, redirect, timeout, cancel, or window-close"
            )
            .into()),
        }
    }

    fn page(self, address: &str) -> String {
        let action = match self {
            Self::Close => "",
            Self::Timeout | Self::Cancel | Self::WindowClose => "",
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

fn assert_clean(client: &ExternalWebviewClient) -> Result<(), WebviewError> {
    let observation = client.test_observation();
    if observation.native_backings == 0
        && observation.live_resources == 0
        && observation.pending_operations == 0
    {
        Ok(())
    } else {
        Err(WebviewError::HostFailure(format!(
            "webview cleanup leaked semantic state: {observation:?}"
        )))
    }
}

fn host_error(error: WebviewError) -> std::io::Error {
    std::io::Error::other(error.to_string())
}
