//! Signal subscriptions on the currently entered runtime's signal driver.

/// Wait for Ctrl+C, registering when first polled. Requires a currently entered
/// runtime with signal drivers; missing drivers may panic. Registration errors
/// propagate. Installing a listener changes process-wide handling permanently:
/// dropping it does not restore the default action. Notifications may coalesce.
pub async fn wait_for_interrupt() -> std::io::Result<()> {
    let mut receiver = crate::platform_imp::interrupt::InterruptReceiver::new()?;
    receiver.recv().await.ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::BrokenPipe, "interrupt receiver closed")
    })
}

/// A SIGTERM subscription on the current Unix runtime driver.
/// Dropping it does not restore the process-wide default SIGTERM action.
#[cfg(unix)]
pub struct TerminationSignal {
    inner: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl TerminationSignal {
    /// Register immediately. Requires a currently entered runtime with signal
    /// drivers; missing drivers may panic. Native registration failures propagate.
    pub fn new() -> std::io::Result<Self> {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .map(|inner| Self { inner })
    }

    /// Wait for one notification; None means the receiver closed. Cancelling a
    /// pending wait does not consume the next notification. Signals may coalesce.
    pub async fn recv(&mut self) -> Option<()> {
        self.inner.recv().await
    }
}
