//! Private raw-Wry backend for the future generated webview operations.
//!
//! This module deliberately does not use `tauri::WebviewWindowBuilder`.
//! Its external-URL route installs the Tauri IPC scripts and handler even
//! when no application command has been configured.  A `PendingWebview`
//! constructed below instead starts with an empty initialization-script list,
//! no IPC handler, and no registered URI schemes.  The only native authority
//! it receives is rendering an approved external HTTP(S) document.
//!
//! There is no public facade or guest ABI here.  Issue #16 owns the
//! generation-safe resource handle and operation lifecycle; it will connect
//! those semantic objects to this backend's private completion callback.

#![allow(dead_code)] // Connected by #16's generated operation/resource table.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri_runtime::{
    window::{PendingWindow, WindowBuilder},
    ExitRequestedEventAction, RunEvent, Runtime as _, RuntimeHandle as _, WindowDispatch as _,
};
use tauri_runtime_wry::{WindowBuilderWrapper, Wry, WryHandle, WryWindowDispatcher};
use url::Url;
use wry::raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};
use wry::{NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder};

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use wry::WebViewBuilderExtUnix as _;

#[cfg(target_os = "macos")]
use wry::WebViewBuilderExtMacos as _;

use crate::async_engine::{self, OneshotReceiver, OneshotSender, RuntimeHandle};

static NEXT_LABEL: AtomicU64 = AtomicU64::new(1);

// Wry's WebView is deliberately !Send.  The Tauri Wry event-loop thread owns
// this private retention map; commands only route closures to that thread.
thread_local! {
    static UI_WEBVIEWS: RefCell<HashMap<u64, WebView>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
struct NativeWindowHandle(WryWindowDispatcher<()>);

impl HasWindowHandle for NativeWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

// This binds the raw runtime implementation to the exact high-level Tauri
// release selected in Cargo.toml without constructing its application manager
// (whose external-page builder installs IPC).
type PinnedTauriEventLoopMessage = tauri::EventLoopMessage;

/// Why native navigation could not continue.  This remains private until the
/// generated operation layer maps it to facade-owned semantic errors.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum NativeWebviewError {
    #[error("webview URL is malformed or not an HTTP(S) URL")]
    InvalidUrl,
    #[error("webview navigation was rejected because {0}")]
    RejectedNavigation(String),
    #[error("the webview window was closed before its requested page loaded")]
    WindowClosed,
    #[error("the native webview host failed: {0}")]
    HostFailure(String),
}

/// A narrowly scoped request accepted by the native backend.  The registry
/// integration will provide the operation timeout and cancellation policy;
/// this object intentionally carries no ambient capabilities.
#[derive(Clone, Debug)]
pub(crate) struct NativeWebviewRequest {
    url: Url,
}

impl NativeWebviewRequest {
    pub(crate) fn parse(url: &str) -> Result<Self, NativeWebviewError> {
        // `url::Url` intentionally repairs `https:///name` into
        // `https://name/`. That is useful for browsers but violates this
        // capability's before-effects policy: the caller did not provide a
        // syntactically valid authority, so reject the original spelling.
        let Some((_, authority_and_path)) = url.split_once("://") else {
            return Err(NativeWebviewError::InvalidUrl);
        };
        if authority_and_path.is_empty() || authority_and_path.starts_with('/') {
            return Err(NativeWebviewError::InvalidUrl);
        }
        let url = Url::parse(url).map_err(|_| NativeWebviewError::InvalidUrl)?;
        if is_allowed_url(&url) {
            Ok(Self { url })
        } else {
            Err(NativeWebviewError::InvalidUrl)
        }
    }
}

/// The event-loop owner.  Construct this on the process's UI/main thread and
/// run it there.  It intentionally does not create an async runtime: callers
/// supply the one facade-owned [`RuntimeHandle`] that receives completions.
pub(crate) struct NativeWebviewLoop {
    runtime: Wry<()>,
}

/// Private command front-end tied to one raw Wry event loop.
#[derive(Clone)]
pub(crate) struct NativeWebviewBackend {
    async_runtime: RuntimeHandle,
    wry: WryHandle<()>,
}

impl NativeWebviewLoop {
    /// Initializes raw Wry without a Tauri application, plugins, commands,
    /// capability files, or IPC.  This must execute on the platform's UI
    /// thread; the caller subsequently drives [`Self::run`].
    pub(crate) fn new(
        async_runtime: RuntimeHandle,
    ) -> Result<(Self, NativeWebviewBackend), NativeWebviewError> {
        let runtime = Wry::new(Default::default())
            .map_err(|error| NativeWebviewError::HostFailure(error.to_string()))?;
        let backend = NativeWebviewBackend {
            async_runtime,
            wry: runtime.handle(),
        };
        Ok((Self { runtime }, backend))
    }

    /// Runs Wry's canonical event loop on its owner thread.  Async callers
    /// never block this loop: creation runs in the supplied runtime's blocking
    /// lane and Wry routes it back using its supported event-loop messages.
    pub(crate) fn run(self) -> i32 {
        self.runtime.run_return(|event| {
            // Keep the shell alive after the final window disappears until
            // the caller has observed its completion and explicitly asks to
            // exit. Wry otherwise auto-exits first, losing the final result.
            if let RunEvent::ExitRequested { code: None, tx, .. } = event {
                let _ = tx.send(ExitRequestedEventAction::Prevent);
            }
        })
    }
}

impl NativeWebviewBackend {
    /// Stops the raw Wry event loop after its resources have been released.
    /// Test/application shell code owns this decision; webview operations only
    /// dispatch through the existing loop.
    pub(crate) fn request_exit(&self) -> Result<(), NativeWebviewError> {
        self.wry
            .request_exit(0)
            .map_err(|error| NativeWebviewError::HostFailure(error.to_string()))
    }

    /// Validates before dispatching any native work, then asks raw Wry to make
    /// a window on the event-loop thread.  This function itself does not make
    /// a generated operation: the caller owns the operation/resource token.
    pub(crate) async fn open(
        &self,
        request: NativeWebviewRequest,
    ) -> Result<NativeWebview, NativeWebviewError> {
        let (created_sender, created_receiver) = async_engine::oneshot_channel();
        let backend = self.clone();
        self.async_runtime
            .launch_blocking(move || backend.create_on_wry_thread(request, created_sender))
            .detach();

        created_receiver.await.map_err(|_| {
            NativeWebviewError::HostFailure("event loop stopped during creation".into())
        })?
    }

    fn create_on_wry_thread(
        &self,
        request: NativeWebviewRequest,
        created_sender: OneshotSender<Result<NativeWebview, NativeWebviewError>>,
    ) {
        let (completion, load_waiter) = LoadCompletion::new(request.url.clone());
        let (terminal, terminal_waiter) = TerminalCompletion::new();
        let (closed, close_waiter) = CloseCompletion::new();
        let created_sender = Arc::new(Mutex::new(Some(created_sender)));
        let native_id = NEXT_LABEL.fetch_add(1, Ordering::Relaxed);
        let label = format!("kernal-api-webview-{native_id}");
        let pending_window = match PendingWindow::<(), Wry<()>>::new(
            WindowBuilderWrapper::new().title("kernal-api external-content proof"),
            label,
        ) {
            Ok(window) => window,
            Err(error) => {
                let _ = created_sender
                    .lock()
                    .expect("creation sender lock poisoned")
                    .take()
                    .expect("creation sender is present")
                    .send(Err(NativeWebviewError::HostFailure(error.to_string())));
                return;
            }
        };
        // This call intentionally occurs off the UI thread. Tauri Wry
        // documents
        // that its handle synchronously routes `create_window` to the event
        // loop, which avoids the Windows callback deadlock caused by invoking
        // it inside a Wry event-loop callback.
        let detached = match self.wry.create_window(
            pending_window,
            None::<for<'a> fn(tauri_runtime::window::RawWindow<'a>)>,
        ) {
            Ok(window) => window,
            Err(error) => {
                let _ = created_sender
                    .lock()
                    .expect("creation sender lock poisoned")
                    .take()
                    .expect("creation sender is present")
                    .send(Err(NativeWebviewError::HostFailure(error.to_string())));
                return;
            }
        };
        let dispatcher = detached.dispatcher;
        debug_assert!(detached.webview.is_none());
        let completion_on_close = Arc::clone(&completion);
        let terminal_on_close = Arc::clone(&terminal);
        let closed_on_close = Arc::clone(&closed);
        dispatcher.on_window_event(move |event| {
            if matches!(event, tauri_runtime::window::WindowEvent::Destroyed) {
                let removed = UI_WEBVIEWS.with(|webviews| webviews.borrow_mut().remove(&native_id));
                drop(removed);
                completion_on_close.finish(Err(NativeWebviewError::WindowClosed));
                terminal_on_close.finish(Err(NativeWebviewError::WindowClosed));
                closed_on_close.finish();
            }
        });

        let window_for_ui = dispatcher.clone();
        let completion_for_ui = Arc::clone(&completion);
        let terminal_for_ui = Arc::clone(&terminal);
        let created_sender_for_ui = Arc::clone(&created_sender);
        if let Err(error) = dispatcher.run_on_main_thread(move || {
            let result = build_isolated_webview(
                &window_for_ui,
                request.url,
                completion_for_ui,
                terminal_for_ui,
            );
            let created_sender = created_sender_for_ui
                .lock()
                .expect("creation sender lock poisoned")
                .take();
            match (result, created_sender) {
                (Ok(webview), Some(created_sender)) if !created_sender.is_closed() => {
                    UI_WEBVIEWS.with(|webviews| {
                        webviews.borrow_mut().insert(native_id, webview);
                    });
                    let _ = created_sender.send(Ok(NativeWebview {
                        window: window_for_ui,
                        native_id,
                        completion,
                        load_waiter: Some(load_waiter),
                        terminal_waiter: Some(terminal_waiter),
                        close_waiter: Some(close_waiter),
                        close_requested: AtomicBool::new(false),
                    }));
                }
                (Ok(webview), _) => {
                    // Cancellation during UI construction leaves no resource
                    // holder, so release the direct Wry view immediately.
                    drop(webview);
                    let _ = window_for_ui.close();
                }
                (Err(error), Some(created_sender)) => {
                    let _ = window_for_ui.close();
                    let _ = created_sender.send(Err(error));
                }
                (Err(_), None) => {
                    let _ = window_for_ui.close();
                }
            }
        }) {
            let _ = dispatcher.close();
            if let Some(created_sender) = created_sender
                .lock()
                .expect("creation sender lock poisoned")
                .take()
            {
                let _ =
                    created_sender.send(Err(NativeWebviewError::HostFailure(error.to_string())));
            }
        }
    }
}

/// Private native resource.  The generated resource table owns its public
/// identity; this type owns only Wry dispatchers and one load completion.
pub(crate) struct NativeWebview {
    window: WryWindowDispatcher<()>,
    native_id: u64,
    completion: Arc<LoadCompletion>,
    load_waiter: Option<OneshotReceiver<Result<(), NativeWebviewError>>>,
    terminal_waiter: Option<OneshotReceiver<Result<(), NativeWebviewError>>>,
    close_waiter: Option<OneshotReceiver<()>>,
    close_requested: AtomicBool,
}

impl NativeWebview {
    /// Installs one waiter for the requested navigation's finished load.
    /// A resource can have one generated operation waiting at a time.
    pub(crate) fn wait_until_loaded(
        &mut self,
    ) -> Result<OneshotReceiver<Result<(), NativeWebviewError>>, NativeWebviewError> {
        self.load_waiter
            .take()
            .ok_or_else(|| NativeWebviewError::HostFailure("load waiter already consumed".into()))
    }

    /// Receives an isolation or host-terminal fault even when the page-load
    /// operation has already completed.  The generated registry uses this to
    /// revoke its resource generation and wake later operations.
    pub(crate) fn wait_until_terminal(
        &mut self,
    ) -> Result<OneshotReceiver<Result<(), NativeWebviewError>>, NativeWebviewError> {
        self.terminal_waiter.take().ok_or_else(|| {
            NativeWebviewError::HostFailure("terminal waiter already consumed".into())
        })
    }

    /// Resolves only once the native window has actually been destroyed.
    pub(crate) fn wait_until_closed(&mut self) -> Result<OneshotReceiver<()>, NativeWebviewError> {
        self.close_waiter
            .take()
            .ok_or_else(|| NativeWebviewError::HostFailure("close waiter already consumed".into()))
    }

    /// Requests native close once and wakes a pending waiter.  Wry routes the
    /// close to the event-loop thread; no async or native runtime is created.
    pub(crate) fn close(&self) -> Result<(), NativeWebviewError> {
        if self
            .close_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            if let Err(error) = self.window.close() {
                self.close_requested.store(false, Ordering::Release);
                let error = NativeWebviewError::HostFailure(error.to_string());
                self.completion.finish(Err(error.clone()));
                return Err(error);
            }
            let native_id = self.native_id;
            let _ = self.window.run_on_main_thread(move || {
                let removed = UI_WEBVIEWS.with(|webviews| webviews.borrow_mut().remove(&native_id));
                drop(removed);
            });
            self.completion
                .finish(Err(NativeWebviewError::WindowClosed));
        }
        Ok(())
    }
}

impl Drop for NativeWebview {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

struct LoadCompletion {
    target: Url,
    sender: Mutex<Option<OneshotSender<Result<(), NativeWebviewError>>>>,
}

struct TerminalCompletion {
    sender: Mutex<Option<OneshotSender<Result<(), NativeWebviewError>>>>,
}

impl TerminalCompletion {
    fn new() -> (Arc<Self>, OneshotReceiver<Result<(), NativeWebviewError>>) {
        let (sender, receiver) = async_engine::oneshot_channel();
        (
            Arc::new(Self {
                sender: Mutex::new(Some(sender)),
            }),
            receiver,
        )
    }

    fn finish(&self, result: Result<(), NativeWebviewError>) {
        let sender = self
            .sender
            .lock()
            .expect("terminal completion lock poisoned")
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }
}

struct CloseCompletion {
    sender: Mutex<Option<OneshotSender<()>>>,
}

impl CloseCompletion {
    fn new() -> (Arc<Self>, OneshotReceiver<()>) {
        let (sender, receiver) = async_engine::oneshot_channel();
        (
            Arc::new(Self {
                sender: Mutex::new(Some(sender)),
            }),
            receiver,
        )
    }

    fn finish(&self) {
        let sender = self
            .sender
            .lock()
            .expect("close completion lock poisoned")
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }
}

impl LoadCompletion {
    fn new(target: Url) -> (Arc<Self>, OneshotReceiver<Result<(), NativeWebviewError>>) {
        let (sender, receiver) = async_engine::oneshot_channel();
        (
            Arc::new(Self {
                target,
                sender: Mutex::new(Some(sender)),
            }),
            receiver,
        )
    }

    fn finish(&self, result: Result<(), NativeWebviewError>) {
        let sender = self
            .sender
            .lock()
            .expect("load completion lock poisoned")
            .take();
        if let Some(sender) = sender {
            let _ = sender.send(result);
        }
    }

    fn matches_requested(&self, loaded: &Url) -> bool {
        self.target == *loaded
    }
}

fn build_isolated_webview(
    dispatcher: &WryWindowDispatcher<()>,
    target: Url,
    completion: Arc<LoadCompletion>,
    terminal: Arc<TerminalCompletion>,
) -> Result<WebView, NativeWebviewError> {
    let completion_for_navigation = Arc::clone(&completion);
    let terminal_for_navigation = Arc::clone(&terminal);
    let completion_for_popup = Arc::clone(&completion);
    let terminal_for_popup = Arc::clone(&terminal);
    let completion_for_load = Arc::clone(&completion);
    let builder = WebViewBuilder::new()
        // Deliberately do not call `with_ipc_handler`: Wry documents that it
        // exposes `window.ipc.postMessage` to page JavaScript.
        .with_url(target.as_str())
        .with_incognito(true)
        .with_clipboard(false)
        .with_devtools(false)
        .with_general_autofill_enabled(false)
        .with_navigation_handler(move |url| match Url::parse(&url) {
            Ok(url) if is_allowed_url(&url) => true,
            Ok(url) => {
                let error = NativeWebviewError::RejectedNavigation(url.scheme().to_owned());
                completion_for_navigation.finish(Err(error.clone()));
                terminal_for_navigation.finish(Err(error));
                false
            }
            Err(_) => {
                let error = NativeWebviewError::RejectedNavigation("malformed URL".into());
                completion_for_navigation.finish(Err(error.clone()));
                terminal_for_navigation.finish(Err(error));
                false
            }
        })
        .with_new_window_req_handler(move |url, _| {
            let scheme = Url::parse(&url)
                .map(|url| url.scheme().to_owned())
                .unwrap_or_else(|_| "malformed URL".into());
            let error = NativeWebviewError::RejectedNavigation(format!("popup to {scheme}"));
            completion_for_popup.finish(Err(error.clone()));
            terminal_for_popup.finish(Err(error));
            NewWindowResponse::Deny
        })
        .with_on_page_load_handler(move |event, loaded_url| {
            // Finished is the requested lifecycle signal, not a claim that
            // the network response was an HTTP success on every engine.
            if matches!(event, PageLoadEvent::Finished)
                && Url::parse(&loaded_url)
                    .is_ok_and(|loaded| completion_for_load.matches_requested(&loaded))
            {
                completion_for_load.finish(Ok(()));
            }
        })
        .with_download_started_handler(|_, _| false);

    #[cfg(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        builder
            .build_gtk(&dispatcher.default_vbox().map_err(host_failure)?)
            .map_err(host_failure)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    {
        builder
            .build(&NativeWindowHandle(dispatcher.clone()))
            .map_err(host_failure)
    }
}

fn host_failure(error: impl std::fmt::Display) -> NativeWebviewError {
    NativeWebviewError::HostFailure(error.to_string())
}

fn is_allowed_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && url.cannot_be_a_base() == false
        && url.username().is_empty()
        && url.password().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_policy_admits_loopback_and_refuses_ambient_schemes() {
        assert!(NativeWebviewRequest::parse("http://127.0.0.1:8080/page").is_ok());
        assert!(NativeWebviewRequest::parse("https://example.test/").is_ok());
        for forbidden in [
            "file:///etc/passwd",
            "data:text/html,hello",
            "tauri://localhost",
            "javascript:alert(1)",
            "https:///missing-host",
            "https://user@example.test/",
        ] {
            assert_eq!(
                NativeWebviewRequest::parse(forbidden).unwrap_err(),
                NativeWebviewError::InvalidUrl,
                "must reject {forbidden}",
            );
        }
    }
}
