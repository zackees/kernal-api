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

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tauri_runtime::{
    webview::{PageLoadEvent, PendingWebview, WebviewAttributes},
    window::{PendingWindow, WindowBuilder},
    Runtime as _, RuntimeHandle as _, WebviewDispatch as _, WindowDispatch as _,
};
use tauri_runtime_wry::{WindowBuilderWrapper, Wry, WryHandle, WryWindowDispatcher};
use tauri_utils::config::WebviewUrl;
use url::Url;

use crate::async_engine::{self, OneshotReceiver, OneshotSender, RuntimeHandle};

static NEXT_LABEL: AtomicU64 = AtomicU64::new(1);

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
    pub(crate) fn run(self) {
        self.runtime.run(|_| {})
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
        let label = format!(
            "kernal-api-webview-{}",
            NEXT_LABEL.fetch_add(1, Ordering::Relaxed)
        );
        let target = request.url.clone();
        let mut pending_webview = match PendingWebview::<(), Wry<()>>::new(
            external_webview_attributes(target.clone()),
            label.clone(),
        ) {
            Ok(webview) => webview,
            Err(error) => {
                let _ =
                    created_sender.send(Err(NativeWebviewError::HostFailure(error.to_string())));
                return;
            }
        };
        pending_webview.url = target.as_str().to_owned();

        // `PendingWebview::new` leaves IPC and URI schemes empty.  Keep those
        // defaults; setting either would give remote content a host bridge.
        debug_assert!(pending_webview.ipc_handler.is_none());
        debug_assert!(pending_webview.uri_scheme_protocols.is_empty());
        debug_assert!(pending_webview
            .webview_attributes
            .initialization_scripts
            .is_empty());

        let completion_for_navigation = Arc::clone(&completion);
        let terminal_for_navigation = Arc::clone(&terminal);
        pending_webview.navigation_handler = Some(Box::new(move |url| {
            if is_allowed_url(url) {
                true
            } else {
                let error = NativeWebviewError::RejectedNavigation(url.scheme().to_owned());
                completion_for_navigation.finish(Err(error.clone()));
                terminal_for_navigation.finish(Err(error));
                false
            }
        }));
        let completion_for_popup = Arc::clone(&completion);
        let terminal_for_popup = Arc::clone(&terminal);
        pending_webview.new_window_handler = Some(Box::new(move |url, _| {
            let error =
                NativeWebviewError::RejectedNavigation(format!("popup to {}", url.scheme()));
            completion_for_popup.finish(Err(error.clone()));
            terminal_for_popup.finish(Err(error));
            tauri_runtime::webview::NewWindowResponse::Deny
        }));
        let completion_for_load = Arc::clone(&completion);
        pending_webview.on_page_load_handler = Some(Box::new(move |loaded_url, event| {
            // Raw Wry supplies this callback for the window's top-level page
            // navigation.  Match the requested navigation and only its final
            // `Finished` event; a Started event or another URL cannot resolve
            // the operation.
            if event == PageLoadEvent::Finished
                && completion_for_load.matches_requested(&loaded_url)
            {
                completion_for_load.finish(Ok(()));
            }
        }));
        pending_webview.download_handler = Some(Arc::new(|_| false));

        let mut pending_window =
            match PendingWindow::<(), Wry<()>>::new(WindowBuilderWrapper::new(), label) {
                Ok(window) => window,
                Err(error) => {
                    let _ = created_sender
                        .send(Err(NativeWebviewError::HostFailure(error.to_string())));
                    return;
                }
            };
        pending_window.set_webview(pending_webview);

        // This call intentionally occurs off the UI thread.  Wry documents
        // that its handle synchronously routes `create_window` to the event
        // loop, which avoids the Windows callback deadlock caused by invoking
        // it inside a Wry event-loop callback.
        let detached = match self.wry.create_window(
            pending_window,
            None::<for<'a> fn(tauri_runtime::window::RawWindow<'a>)>,
        ) {
            Ok(window) => window,
            Err(error) => {
                let _ =
                    created_sender.send(Err(NativeWebviewError::HostFailure(error.to_string())));
                return;
            }
        };
        let dispatcher = detached.dispatcher;
        let webview_dispatcher = match detached.webview {
            Some(webview) => webview.webview.dispatcher,
            None => {
                let _ = dispatcher.close();
                let _ = created_sender.send(Err(NativeWebviewError::HostFailure(
                    "raw Wry created a window without its requested webview".into(),
                )));
                return;
            }
        };
        let completion_on_close = Arc::clone(&completion);
        let terminal_on_close = Arc::clone(&terminal);
        let closed_on_close = Arc::clone(&closed);
        dispatcher.on_window_event(move |event| {
            if matches!(event, tauri_runtime::window::WindowEvent::Destroyed) {
                completion_on_close.finish(Err(NativeWebviewError::WindowClosed));
                terminal_on_close.finish(Err(NativeWebviewError::WindowClosed));
                closed_on_close.finish();
            }
        });

        let _ = created_sender.send(Ok(NativeWebview {
            window: dispatcher,
            webview: webview_dispatcher,
            completion,
            load_waiter: Some(load_waiter),
            terminal_waiter: Some(terminal_waiter),
            close_waiter: Some(close_waiter),
            close_requested: AtomicBool::new(false),
        }));
    }
}

/// Private native resource.  The generated resource table owns its public
/// identity; this type owns only Wry dispatchers and one load completion.
pub(crate) struct NativeWebview {
    window: WryWindowDispatcher<()>,
    webview: tauri_runtime_wry::WryWebviewDispatcher<()>,
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

    /// Starts a further validated top-level navigation and returns its one-shot
    /// completion.  The generated operation layer associates that receiver
    /// with its already-owned operation token; the backend owns no handle.
    pub(crate) fn navigate(
        &self,
        url: &str,
    ) -> Result<OneshotReceiver<Result<(), NativeWebviewError>>, NativeWebviewError> {
        let url = NativeWebviewRequest::parse(url)?.url;
        let receiver = self.completion.reset(url.clone())?;
        if let Err(error) = self.webview.navigate(url) {
            let error = NativeWebviewError::HostFailure(error.to_string());
            self.completion.finish(Err(error.clone()));
            return Err(error);
        }
        Ok(receiver)
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
    target: Mutex<Url>,
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
                target: Mutex::new(target),
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
        *self.target.lock().expect("load target lock poisoned") == *loaded
    }

    fn reset(
        &self,
        target: Url,
    ) -> Result<OneshotReceiver<Result<(), NativeWebviewError>>, NativeWebviewError> {
        let mut sender = self.sender.lock().expect("load completion lock poisoned");
        if sender.is_some() {
            return Err(NativeWebviewError::HostFailure(
                "a previous top-level navigation is still pending".into(),
            ));
        }
        let (next_sender, receiver) = async_engine::oneshot_channel();
        *self.target.lock().expect("load target lock poisoned") = target;
        *sender = Some(next_sender);
        Ok(receiver)
    }
}

fn external_webview_attributes(url: Url) -> WebviewAttributes {
    let mut attributes = WebviewAttributes::new(WebviewUrl::External(url));
    attributes.incognito = true;
    attributes.clipboard = false;
    attributes.browser_extensions_enabled = false;
    attributes.general_autofill_enabled = false;
    attributes.devtools = Some(false);
    attributes.allow_link_preview = false;
    attributes
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
