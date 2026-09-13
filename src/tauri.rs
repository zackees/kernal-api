//! Private raw-Wry backend for the future generated webview operations.
//!
//! This module deliberately does not use `tauri::WebviewWindowBuilder`.
//! Its external-URL route installs the Tauri IPC scripts and handler even
//! when no application command has been configured. A plain native window
//! instead hosts a raw Wry view. Before its first external navigation, the
//! Linux adapter removes Wry's injected scripts and IPC endpoint. No application
//! IPC handler or custom URI scheme is installed. The only native authority it
//! receives is rendering an approved external HTTP(S) document.
//!
//! The public semantic façade at the end of this module submits every native
//! transition through the shared generation-safe operation hub. Raw Wry
//! objects remain private physical backing only.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri_runtime::{
    window::{PendingWindow, WindowBuilder},
    ExitRequestedEventAction, RunEvent, Runtime as _, RuntimeHandle as _, WindowDispatch as _,
};
use tauri_runtime_wry::{WindowBuilderWrapper, Wry, WryHandle, WryWindowDispatcher};
use url::Url;
use wry::{NewWindowResponse, PageLoadEvent, WebView, WebViewBuilder};

#[cfg(target_os = "linux")]
#[path = "tauri/linux_webkitgtk.rs"]
mod linux_webkitgtk;

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
use wry::raw_window_handle::{HandleError, HasWindowHandle, WindowHandle};

#[cfg(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
))]
use wry::WebViewBuilderExtUnix as _;

#[cfg(target_os = "linux")]
use wry::WebViewExtUnix as _;

use crate::async_engine::{self, OneshotReceiver, OneshotSender, RuntimeHandle};
use crate::operations::{HubError, OpaqueToken, OperationHub, Terminal};

static NEXT_LABEL: AtomicU64 = AtomicU64::new(1);
static NEXT_WEBVIEW_STORE: AtomicU64 = AtomicU64::new(1);

// Wry's WebView is deliberately !Send.  The Tauri Wry event-loop thread owns
// this private retention map; commands only route closures to that thread.
thread_local! {
    static UI_WEBVIEWS: RefCell<HashMap<u64, WebView>> = RefCell::new(HashMap::new());
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
#[derive(Clone)]
struct NativeWindowHandle(WryWindowDispatcher<()>);

#[cfg(not(any(
    target_os = "linux",
    target_os = "dragonfly",
    target_os = "freebsd",
    target_os = "netbsd",
    target_os = "openbsd"
)))]
impl HasWindowHandle for NativeWindowHandle {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        self.0.window_handle()
    }
}

// This binds the raw runtime implementation to the exact high-level Tauri
// release selected in Cargo.toml without constructing its application manager
// (whose external-page builder installs IPC).
#[allow(dead_code)] // Compile-time exact-release binding; no application manager is constructed.
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
    permissions: WebviewPermissions,
}

impl NativeWebviewRequest {
    pub(crate) fn parse(
        url: &str,
        permissions: WebviewPermissions,
    ) -> Result<Self, NativeWebviewError> {
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
            Ok(Self { url, permissions })
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
        #[cfg(target_os = "linux")]
        linux_webkitgtk::prepare_renderer_environment();
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
                request.permissions,
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
    permissions: WebviewPermissions,
    completion: Arc<LoadCompletion>,
    terminal: Arc<TerminalCompletion>,
) -> Result<WebView, NativeWebviewError> {
    #[cfg(not(target_os = "linux"))]
    let _ = permissions;
    let completion_for_navigation = Arc::clone(&completion);
    let terminal_for_navigation = Arc::clone(&terminal);
    let completion_for_popup = Arc::clone(&completion);
    let terminal_for_popup = Arc::clone(&terminal);
    let completion_for_load = Arc::clone(&completion);
    let builder = WebViewBuilder::new()
        // Deliberately do not call `with_ipc_handler`: Wry documents that it
        // exposes `window.ipc.postMessage` to page JavaScript.
        // Do not navigate during construction. The Linux adapter must remove
        // backend-injected scripts and endpoints before any external page runs.
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
        linux_webkitgtk::ensure_font_dpi();
        let webview = builder
            .build_gtk(&dispatcher.default_vbox().map_err(host_failure)?)
            .map_err(host_failure)?;
        linux_webkitgtk::remove_host_bridge(&webview.webview())?;
        linux_webkitgtk::configure_permissions(&webview.webview(), permissions);
        webview.load_url(target.as_str()).map_err(host_failure)?;
        Ok(webview)
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "dragonfly",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    {
        let webview = builder
            .build(&NativeWindowHandle(dispatcher.clone()))
            .map_err(host_failure)?;
        webview.load_url(target.as_str()).map_err(host_failure)?;
        Ok(webview)
    }
}

fn host_failure(error: impl std::fmt::Display) -> NativeWebviewError {
    NativeWebviewError::HostFailure(error.to_string())
}

fn is_allowed_url(url: &Url) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.host_str().is_some()
        && !url.cannot_be_a_base()
        && url.username().is_empty()
        && url.password().is_none()
}

/// Facade-owned failures for an external webview operation.
///
/// No native backend, runtime, or window value is exposed through this type.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WebviewError {
    #[error("webview URL is malformed or not an HTTP(S) URL")]
    InvalidUrl,
    #[error("webview navigation was rejected: {0}")]
    RejectedNavigation(String),
    #[error("webview load timed out")]
    TimedOut,
    #[error("webview operation was cancelled")]
    Cancelled,
    #[error("the webview window was closed")]
    WindowClosed,
    #[error("the webview host failed: {0}")]
    HostFailure(String),
}

/// Semantic permissions for one external webview.
///
/// All permissions are denied by default. Enabling user media permits only
/// microphone/camera requests on Linux WebKitGTK; unrelated WebKit permission
/// requests retain their engine-default denial. This type intentionally does
/// not expose a WebKit, Wry, or Tauri policy object.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WebviewPermissions {
    pub(crate) allow_user_media: bool,
}

impl WebviewPermissions {
    /// Start from the deny-by-default webview permission policy.
    pub const fn deny_all() -> Self {
        Self {
            allow_user_media: false,
        }
    }

    /// Permit microphone/camera requests for this webview where the host
    /// supports user media. This does not grant geolocation, notifications,
    /// downloads, clipboard access, or host IPC.
    pub const fn allow_user_media(mut self) -> Self {
        self.allow_user_media = true;
        self
    }
}

/// Process-main-thread owner of the opt-in native event loop.
///
/// Construct this on the UI/main thread, hand [`ExternalWebviewClient`] to
/// async work, then call [`Self::run`]. The supplied facade runtime is the
/// only async runtime used by callbacks and operations.
pub struct ExternalWebviewHost {
    event_loop: NativeWebviewLoop,
    client: ExternalWebviewClient,
}

/// Instance-scoped semantic client for opening external webviews.
#[derive(Clone)]
pub struct ExternalWebviewClient {
    service: Arc<WebviewService>,
    store: u64,
}

/// Opaque, generation-safe external-webview resource.
///
/// It is intentionally non-cloneable: dropping it revokes the generation and
/// requests physical close, so abandoned operations cannot retain a window.
pub struct WebviewHandle {
    service: Arc<WebviewService>,
    store: u64,
    resource: OpaqueToken,
    terminal_operation: OpaqueToken,
}

/// Acceptance-only semantic counters for the process-main-thread smoke test.
///
/// This deliberately reports counts rather than a backend handle, native
/// window, runtime, or raw resource token.
#[cfg(feature = "tauri-webview-test-support")]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WebviewTestObservation {
    /// Private physical WebView backings retained by the facade.
    pub native_backings: usize,
    /// Live generation-safe semantic resources in the shared hub.
    pub live_resources: usize,
    /// Pending semantic operations in the shared hub.
    pub pending_operations: usize,
}

struct WebviewService {
    runtime: RuntimeHandle,
    backend: NativeWebviewBackend,
    hub: Arc<OperationHub>,
    // This is a physical backing table, not a second authority registry:
    // all identity, ownership, quota, terminal state, and revocation remain
    // in OperationHub. Wry objects cannot be stored in that runtime-neutral
    // hub because they are UI-thread-affine.
    native: Mutex<BTreeMap<OpaqueToken, NativeWebview>>,
    // An explicit semantic close owns its terminal result.  The independent
    // native-terminal watcher must not race it into reporting WindowClosed.
    closing: Mutex<BTreeSet<OpaqueToken>>,
}

impl ExternalWebviewHost {
    /// Create the private event loop and one root semantic instance.
    pub fn new(runtime: RuntimeHandle) -> Result<Self, WebviewError> {
        let (event_loop, backend) = NativeWebviewLoop::new(runtime.clone()).map_err(map_native)?;
        let hub = OperationHub::new(64, 64).map_err(map_hub)?;
        Ok(Self {
            event_loop,
            client: ExternalWebviewClient {
                service: Arc::new(WebviewService {
                    runtime,
                    backend,
                    hub,
                    native: Mutex::new(BTreeMap::new()),
                    closing: Mutex::new(BTreeSet::new()),
                }),
                store: next_store()?,
            },
        })
    }

    /// Obtain the root instance-scoped semantic client.
    pub fn client(&self) -> ExternalWebviewClient {
        self.client.clone()
    }

    /// Run the canonical Tauri/Wry event loop on the creating main thread.
    pub fn run(self) -> i32 {
        self.event_loop.run()
    }
}

impl ExternalWebviewClient {
    /// Create an independently authorized logical instance over the same
    /// host. Handles from one instance cannot be used by another.
    pub fn new_instance(&self) -> Result<Self, WebviewError> {
        Ok(Self {
            service: Arc::clone(&self.service),
            store: next_store()?,
        })
    }

    /// Validate and asynchronously create an isolated external webview.
    pub async fn open_webview(&self, url: &str) -> Result<WebviewHandle, WebviewError> {
        self.open_webview_with_permissions(url, WebviewPermissions::deny_all())
            .await
    }

    /// Validate and asynchronously create an isolated external webview with
    /// the supplied facade-owned permission policy.
    pub async fn open_webview_with_permissions(
        &self,
        url: &str,
        permissions: WebviewPermissions,
    ) -> Result<WebviewHandle, WebviewError> {
        let request = NativeWebviewRequest::parse(url, permissions).map_err(map_native)?;
        let (resource, operation) = self
            .service
            .hub
            .begin_external_webview_open(self.store)
            .map_err(map_hub)?;
        let mut native = match self.service.backend.open(request).await {
            Ok(native) => native,
            Err(error) => {
                self.service
                    .hub
                    .finish_external_operation(operation, terminal_for_native(&error));
                self.service.revoke(resource);
                return Err(map_native(error));
            }
        };
        let terminal = native.wait_until_terminal().map_err(map_native)?;
        self.service
            .native
            .lock()
            .map_err(|_| WebviewError::HostFailure("native backing table poisoned".into()))?
            .insert(resource, native);
        self.service.hub.finish_external_open(operation, resource);
        let service = Arc::clone(&self.service);
        self.service
            .runtime
            .launch(async move {
                // Native callbacks retain only this hub/service state and a
                // receiver; a Store, Caller, guest memory, or UI object never
                // crosses into the callback task.
                let terminal = match terminal.await {
                    Ok(Ok(())) => return,
                    Ok(Err(error)) => terminal_for_native(&error),
                    Err(_) => Terminal::Closed,
                };
                if !service.is_explicitly_closing(resource) {
                    service.revoke_with_terminal(resource, terminal);
                }
            })
            .detach();
        match self.service.hub.observe_terminal(self.store, operation) {
            Ok(Some(result)) if result.terminal == Terminal::Completed => {
                let terminal_operation = self
                    .service
                    .hub
                    .begin_external_webview_wait(self.store, resource)
                    .map_err(map_hub)?;
                Ok(WebviewHandle {
                    service: Arc::clone(&self.service),
                    store: self.store,
                    resource,
                    terminal_operation,
                })
            }
            Ok(Some(result)) => Err(map_terminal(result.terminal)),
            Ok(None) => Err(WebviewError::HostFailure(
                "webview open did not complete".into(),
            )),
            Err(error) => Err(map_hub(error)),
        }
    }

    /// Ask the host to stop once outstanding callbacks have completed.
    pub fn request_exit(&self) -> Result<(), WebviewError> {
        self.service.backend.request_exit().map_err(map_native)
    }

    /// Return acceptance-only semantic counts without exposing a backend type.
    #[cfg(feature = "tauri-webview-test-support")]
    pub fn test_observation(&self) -> WebviewTestObservation {
        let snapshot = self.service.hub.snapshot();
        let native_backings = self.service.native.lock().map_or(0, |native| native.len());
        WebviewTestObservation {
            native_backings,
            live_resources: snapshot.live_resources,
            pending_operations: snapshot.pending_operations,
        }
    }
}

impl WebviewHandle {
    /// Await the requested top-level page's matching `Finished` event.
    /// A timeout revokes this handle and closes the backing native window.
    pub async fn wait_until_loaded(&self, timeout: Duration) -> Result<(), WebviewError> {
        let operation = self
            .service
            .hub
            .begin_external_webview_wait(self.store, self.resource)
            .map_err(map_hub)?;
        let receiver = {
            let mut native =
                self.service.native.lock().map_err(|_| {
                    WebviewError::HostFailure("native backing table poisoned".into())
                })?;
            native
                .get_mut(&self.resource)
                .ok_or(WebviewError::WindowClosed)?
                .wait_until_loaded()
                .map_err(map_native)?
        };
        let terminal = match async_engine::timeout(timeout, receiver).await {
            Ok(Ok(Ok(()))) => Terminal::Completed,
            Ok(Ok(Err(error))) => terminal_for_native(&error),
            Ok(Err(_)) => Terminal::Closed,
            Err(_) => Terminal::TimedOut,
        };
        self.service
            .hub
            .finish_external_operation(operation, terminal);
        if terminal != Terminal::Completed {
            self.service.revoke_with_terminal(self.resource, terminal);
        }
        match self.service.hub.observe_terminal(self.store, operation) {
            Ok(Some(result)) if result.terminal == Terminal::Completed => Ok(()),
            Ok(Some(result)) => Err(map_terminal(result.terminal)),
            Ok(None) => Err(WebviewError::HostFailure(
                "load operation did not complete".into(),
            )),
            Err(error) => Err(map_hub(error)),
        }
    }

    /// Await a terminal security or window-close callback after opening.
    ///
    /// This is useful when an allowed top-level document finishes and then
    /// attempts a prohibited redirect or popup. It is itself a hub-owned
    /// operation, so callback completion never needs to retain a Store.
    pub async fn wait_until_terminal(&self, timeout: Duration) -> Result<(), WebviewError> {
        // A cancellation or window callback may have completed this operation
        // before the caller first awaits it. Poll first; if completion wins
        // the short race before suspension, consume that typed terminal below
        // rather than collapsing it into HubError::Closed.
        if let Some(result) = self
            .service
            .hub
            .observe_terminal(self.store, self.terminal_operation)
            .map_err(map_hub)?
        {
            return Err(map_terminal(result.terminal));
        }
        match self
            .service
            .hub
            .wait_external_operation(self.store, self.terminal_operation)
        {
            Ok(wake) => {
                if async_engine::timeout(timeout, wake.notified())
                    .await
                    .is_err()
                {
                    self.service
                        .revoke_with_terminal(self.resource, Terminal::TimedOut);
                }
            }
            // Completion can race the poll above; the final observe below
            // retains the callback's typed terminal result.
            Err(HubError::Closed) => {}
            Err(error) => return Err(map_hub(error)),
        }
        match self
            .service
            .hub
            .observe_terminal(self.store, self.terminal_operation)
        {
            Ok(Some(result)) => Err(map_terminal(result.terminal)),
            Ok(None) => Err(WebviewError::HostFailure(
                "terminal webview operation did not complete".into(),
            )),
            Err(error) => Err(map_hub(error)),
        }
    }

    /// Request close and await native destruction before revoking the handle.
    pub async fn close(self) -> Result<(), WebviewError> {
        let operation = self
            .service
            .hub
            .begin_external_webview_close(self.store, self.resource)
            .map_err(map_hub)?;
        self.service.mark_explicitly_closing(self.resource);
        let mut native = self.service.take_native(self.resource).ok_or_else(|| {
            self.service.clear_explicitly_closing(self.resource);
            WebviewError::WindowClosed
        })?;
        let closed = match native.wait_until_closed() {
            Ok(closed) => closed,
            Err(error) => {
                self.service
                    .hub
                    .finish_external_operation(operation, terminal_for_native(&error));
                self.service
                    .revoke_with_terminal(self.resource, terminal_for_native(&error));
                self.service.clear_explicitly_closing(self.resource);
                return Err(map_native(error));
            }
        };
        if let Err(error) = native.close() {
            self.service
                .hub
                .finish_external_operation(operation, terminal_for_native(&error));
            self.service
                .revoke_with_terminal(self.resource, terminal_for_native(&error));
            self.service.clear_explicitly_closing(self.resource);
            return Err(map_native(error));
        }
        let terminal = match closed.await {
            Ok(()) => Terminal::Completed,
            Err(_) => Terminal::Closed,
        };
        self.service
            .hub
            .finish_external_operation(operation, terminal);
        let outcome = match self.service.hub.observe_terminal(self.store, operation) {
            Ok(Some(result)) if result.terminal == Terminal::Completed => Ok(()),
            Ok(Some(result)) => Err(map_terminal(result.terminal)),
            Ok(None) => Err(WebviewError::HostFailure(
                "close operation did not complete".into(),
            )),
            Err(error) => Err(map_hub(error)),
        };
        // The close operation itself is resource-bound, so publish and consume
        // its successful terminal state before resource revocation wakes any
        // other operation bound to the same generation.
        let _ = self.service.hub.close_resource(self.resource);
        self.service.clear_explicitly_closing(self.resource);
        outcome
    }

    /// Explicitly cancel a handle. Cancellation is terminal and revokes the
    /// resource generation before returning.
    pub fn cancel(&self) {
        self.service
            .revoke_with_terminal(self.resource, Terminal::Cancelled);
    }

    /// Acceptance-only stand-in for a user/window-manager close gesture.
    /// The normal terminal callback path must revoke the same semantic handle
    /// as a real user close, while the smoke can invoke it deterministically.
    #[cfg(feature = "tauri-webview-test-support")]
    pub fn request_window_close_for_test(&self) -> Result<(), WebviewError> {
        let native = self
            .service
            .native
            .lock()
            .map_err(|_| WebviewError::HostFailure("native backing table poisoned".into()))?;
        native
            .get(&self.resource)
            .ok_or(WebviewError::WindowClosed)?
            .close()
            .map_err(map_native)
    }
}

impl Drop for WebviewHandle {
    fn drop(&mut self) {
        self.service
            .revoke_with_terminal(self.resource, Terminal::Cancelled);
    }
}

impl WebviewService {
    fn take_native(&self, resource: OpaqueToken) -> Option<NativeWebview> {
        self.native.lock().ok()?.remove(&resource)
    }

    fn revoke(&self, resource: OpaqueToken) {
        self.revoke_with_terminal(resource, Terminal::Closed);
    }

    fn revoke_with_terminal(&self, resource: OpaqueToken, terminal: Terminal) {
        if let Some(native) = self.take_native(resource) {
            let _ = native.close();
            drop(native);
        }
        let _ = self.hub.revoke_external_resource(resource, terminal);
    }

    fn mark_explicitly_closing(&self, resource: OpaqueToken) {
        if let Ok(mut closing) = self.closing.lock() {
            closing.insert(resource);
        }
    }

    fn clear_explicitly_closing(&self, resource: OpaqueToken) {
        if let Ok(mut closing) = self.closing.lock() {
            closing.remove(&resource);
        }
    }

    fn is_explicitly_closing(&self, resource: OpaqueToken) -> bool {
        self.closing
            .lock()
            .is_ok_and(|closing| closing.contains(&resource))
    }
}

fn next_store() -> Result<u64, WebviewError> {
    NEXT_WEBVIEW_STORE
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
            value.checked_add(1)
        })
        .map(|value| value + 1)
        .map_err(|_| WebviewError::HostFailure("webview instance identifiers exhausted".into()))
}

fn terminal_for_native(error: &NativeWebviewError) -> Terminal {
    match error {
        NativeWebviewError::RejectedNavigation(_) | NativeWebviewError::InvalidUrl => {
            Terminal::Rejected
        }
        NativeWebviewError::WindowClosed => Terminal::Closed,
        NativeWebviewError::HostFailure(_) => Terminal::Trapped,
    }
}

fn map_native(error: NativeWebviewError) -> WebviewError {
    match error {
        NativeWebviewError::InvalidUrl => WebviewError::InvalidUrl,
        NativeWebviewError::RejectedNavigation(reason) => WebviewError::RejectedNavigation(reason),
        NativeWebviewError::WindowClosed => WebviewError::WindowClosed,
        NativeWebviewError::HostFailure(reason) => WebviewError::HostFailure(reason),
    }
}

fn map_terminal(terminal: Terminal) -> WebviewError {
    match terminal {
        Terminal::Cancelled => WebviewError::Cancelled,
        Terminal::TimedOut => WebviewError::TimedOut,
        Terminal::Closed => WebviewError::WindowClosed,
        Terminal::Rejected => WebviewError::RejectedNavigation("navigation policy".into()),
        Terminal::Completed => WebviewError::HostFailure("unexpected completed error".into()),
        Terminal::Trapped | Terminal::OwnerExited => {
            WebviewError::HostFailure("native webview operation failed".into())
        }
    }
}

fn map_hub(error: HubError) -> WebviewError {
    match error {
        HubError::Quota => WebviewError::HostFailure("webview operation quota exhausted".into()),
        HubError::Closed | HubError::Stale => WebviewError::WindowClosed,
        HubError::Invalid | HubError::WrongKind | HubError::WrongRights | HubError::Exhausted => {
            WebviewError::HostFailure("invalid semantic webview operation".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_url_policy_admits_loopback_and_refuses_ambient_schemes() {
        assert!(NativeWebviewRequest::parse(
            "http://127.0.0.1:8080/page",
            WebviewPermissions::default(),
        )
        .is_ok());
        assert!(NativeWebviewRequest::parse(
            "https://example.test/",
            WebviewPermissions::default(),
        )
        .is_ok());
        for forbidden in [
            "file:///etc/passwd",
            "data:text/html,hello",
            "tauri://localhost",
            "javascript:alert(1)",
            "https:///missing-host",
            "https://user@example.test/",
        ] {
            assert_eq!(
                NativeWebviewRequest::parse(forbidden, WebviewPermissions::default()).unwrap_err(),
                NativeWebviewError::InvalidUrl,
                "must reject {forbidden}",
            );
        }
    }

    #[test]
    fn user_media_permission_is_explicitly_opt_in() {
        assert_eq!(
            WebviewPermissions::default(),
            WebviewPermissions::deny_all()
        );
        assert_ne!(
            WebviewPermissions::deny_all(),
            WebviewPermissions::deny_all().allow_user_media()
        );
    }
}
