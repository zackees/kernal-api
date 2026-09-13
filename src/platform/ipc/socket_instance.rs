//! Opaque pathname socket primitives; endpoint retirement and parent-directory
//! policy are explicit caller operations, not implicit connection side effects.

use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// Credentials observed on an accepted connection. An unavailable PID does
/// not imply unavailable credentials (some Unix hosts report only a UID).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocketPeerCredentials {
    pid: Option<u32>,
    current_user: bool,
    available: bool,
}

impl SocketPeerCredentials {
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
    pub fn is_current_user(&self) -> bool {
        self.current_user
    }
    pub fn credentials_available(&self) -> bool {
        self.available
    }
}

/// A pathname socket listener with owner read/write socket permissions.
/// Requires an active I/O-enabled runtime. The caller must secure the parent
/// directory against replacement and explicitly retire any stale socket first.
/// Binding then setting permissions is not an atomic security operation: the
/// parent must prevent unauthorized access during that interval. Drop closes
/// the listener but does not unlink its pathname.
pub struct LocalSocketListener {
    inner: tokio::net::UnixListener,
}

impl LocalSocketListener {
    pub fn bind_owner_only(path: &Path) -> io::Result<Self> {
        let inner = tokio::net::UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { inner })
    }

    pub async fn accept(&self) -> io::Result<(LocalSocketStream, SocketPeerCredentials)> {
        let (inner, _) = self.inner.accept().await?;
        let peer = match inner.peer_cred() {
            Ok(credentials) => SocketPeerCredentials {
                pid: credentials.pid().and_then(|pid| pid.try_into().ok()),
                // SAFETY: geteuid has no preconditions and does not mutate state.
                current_user: credentials.uid() == unsafe { libc::geteuid() },
                available: true,
            },
            Err(_) => SocketPeerCredentials {
                pid: None,
                current_user: false,
                available: false,
            },
        };
        Ok((LocalSocketStream { inner }, peer))
    }
}

/// Connected pathname socket with standard-library-only polling byte I/O.
pub struct LocalSocketStream {
    inner: tokio::net::UnixStream,
}

impl LocalSocketStream {
    pub async fn connect(path: &Path) -> io::Result<Self> {
        tokio::net::UnixStream::connect(path)
            .await
            .map(|inner| Self { inner })
    }

    pub fn poll_read(&mut self, cx: &mut Context<'_>, bytes: &mut [u8]) -> Poll<io::Result<usize>> {
        let mut buffer = ReadBuf::new(bytes);
        match Pin::new(&mut self.inner).poll_read(cx, &mut buffer) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(buffer.filled().len())),
            Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
            Poll::Pending => Poll::Pending,
        }
    }

    pub fn poll_write(&mut self, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, bytes)
    }

    pub fn poll_flush(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    pub fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    #[tokio::test]
    async fn canceled_accept_keeps_listener_usable_and_shutdown_delivers_eof() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("cancel");
        let listener = LocalSocketListener::bind_owner_only(&path).unwrap();
        {
            let mut pending = Box::pin(listener.accept());
            let mut cx = Context::from_waker(std::task::Waker::noop());
            assert!(pending.as_mut().poll(&mut cx).is_pending());
        }
        let mut client = LocalSocketStream::connect(&path).await.unwrap();
        let (mut server, _) = listener.accept().await.unwrap();
        std::future::poll_fn(|cx| client.poll_shutdown(cx))
            .await
            .unwrap();
        let mut bytes = [0; 1];
        assert_eq!(
            std::future::poll_fn(|cx| server.poll_read(cx, &mut bytes))
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn owner_mode_peer_and_byte_io() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("socket");
        let listener = LocalSocketListener::bind_owner_only(&path).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let mut client = LocalSocketStream::connect(&path).await.unwrap();
        let (mut server, peer) = listener.accept().await.unwrap();
        assert!(peer.credentials_available());
        assert!(peer.is_current_user());
        #[cfg(target_os = "linux")]
        assert_eq!(peer.pid(), Some(std::process::id()));
        assert_eq!(
            std::future::poll_fn(|cx| client.poll_write(cx, b"x"))
                .await
                .unwrap(),
            1
        );
        let mut buffer = [0; 1];
        assert_eq!(
            std::future::poll_fn(|cx| server.poll_read(cx, &mut buffer))
                .await
                .unwrap(),
            1
        );
        assert_eq!(buffer, *b"x");
        drop(listener);
        assert!(
            path.exists(),
            "drop must not perform implicit endpoint retirement"
        );
    }

    #[tokio::test]
    async fn binding_never_retires_existing_regular_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("keep");
        std::fs::write(&path, b"keep").unwrap();
        assert!(LocalSocketListener::bind_owner_only(&path).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"keep");
    }
}
