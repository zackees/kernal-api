//! Single Windows pipe instance. Pooling and retry policy belong to callers.

use std::ffi::c_void;
use std::io;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};

/// An opaque connected Windows byte-mode pipe client. Opening performs one
/// native attempt and preserves its error, including pipe-busy (231). Retry,
/// scheduling, and deadline policy remain with the caller. Opening requires
/// an active I/O-enabled runtime and does not authenticate the server owner.
pub struct LocalPipeClient {
    inner: NamedPipeClient,
}

impl LocalPipeClient {
    pub fn open(endpoint: &str) -> io::Result<Self> {
        if endpoint.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe endpoint contains NUL",
            ));
        }
        ClientOptions::new()
            .open(endpoint)
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

#[repr(C)]
struct SecurityAttributes {
    length: u32,
    descriptor: *mut c_void,
    inherit_handle: i32,
}

struct SecurityDescriptor(*mut c_void);

#[link(name = "advapi32")]
unsafe extern "system" {
    fn ConvertStringSecurityDescriptorToSecurityDescriptorW(
        text: *const u16,
        revision: u32,
        descriptor: *mut *mut c_void,
        size: *mut u32,
    ) -> i32;
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn LocalFree(memory: *mut c_void) -> *mut c_void;
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: the conversion API allocates this descriptor with LocalAlloc.
        unsafe { LocalFree(self.0) };
    }
}

/// One local byte-mode named-pipe server instance with a protected DACL granting
/// access to its owner and SYSTEM. Remote clients are rejected. No pool, retry,
/// timeout, or background task is created by this type.
///
/// Creation requires an active I/O-enabled runtime. Dropping the instance closes
/// its handle, including a pending connection. Canceling only the `connect`
/// future retains the instance; drop the instance to abandon it.
pub struct OwnerOnlyPipeInstance {
    inner: NamedPipeServer,
}

impl OwnerOnlyPipeInstance {
    /// Create an instance. `first` requests Windows first-instance protection;
    /// an existing pipe of the same name then causes creation to fail. Native
    /// errors are preserved so the caller can apply its own retry policy.
    pub fn create(endpoint: &str, first: bool) -> io::Result<Self> {
        if endpoint.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "pipe endpoint contains NUL",
            ));
        }
        let sddl: Vec<u16> = "D:P(A;;GA;;;OW)(A;;GA;;;SY)"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: the input is terminated and the output points to live storage.
        let result = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        if descriptor.is_null() {
            return Err(io::Error::other(
                "Windows returned a null security descriptor",
            ));
        }
        let descriptor = SecurityDescriptor(descriptor);
        let attributes = SecurityAttributes {
            length: size_of::<SecurityAttributes>() as u32,
            descriptor: descriptor.0,
            inherit_handle: 0,
        };
        let mut options = ServerOptions::new();
        options
            .first_pipe_instance(first)
            .reject_remote_clients(true);
        // SAFETY: attributes and descriptor remain alive through creation;
        // Windows copies the descriptor and the returned handle is not inherited.
        let inner = unsafe {
            options.create_with_security_attributes_raw(
                endpoint,
                std::ptr::from_ref(&attributes).cast_mut().cast(),
            )
        }?;
        Ok(Self { inner })
    }

    /// Wait for a client on this instance without replenishing a listener pool.
    pub async fn connect(&self) -> io::Result<()> {
        self.inner.connect().await
    }

    /// Poll byte input using only standard-library types. A successful zero
    /// byte read on a nonempty buffer denotes EOF.
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
    use std::os::windows::io::AsRawHandle;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[tokio::test]
    async fn canceled_connect_retains_instance_until_owner_drops_it() {
        let endpoint = format!(r"\\.\pipe\kernal-api-cancel-{}", std::process::id());
        let server = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
        {
            let mut pending = Box::pin(server.connect());
            let mut cx = Context::from_waker(std::task::Waker::noop());
            assert!(pending.as_mut().poll(&mut cx).is_pending());
        }
        assert!(OwnerOnlyPipeInstance::create(&endpoint, true).is_err());
        let client = LocalPipeClient::open(&endpoint).unwrap();
        server.connect().await.unwrap();
        drop(client);
        drop(server);
        let _replacement = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
    }

    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn GetSecurityInfo(
            handle: isize,
            object_type: i32,
            security_info: u32,
            owner: *mut *mut c_void,
            group: *mut *mut c_void,
            dacl: *mut *mut c_void,
            sacl: *mut *mut c_void,
            descriptor: *mut *mut c_void,
        ) -> u32;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor: *mut c_void,
            revision: u32,
            security_info: u32,
            text: *mut *mut u16,
            length: *mut u32,
        ) -> i32;
    }

    fn bound_dacl(pipe: &OwnerOnlyPipeInstance) -> String {
        let mut descriptor = std::ptr::null_mut();
        // SAFETY: the pipe handle is live and the descriptor output is valid.
        let status = unsafe {
            GetSecurityInfo(
                pipe.inner.as_raw_handle() as isize,
                6,
                4,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        assert_eq!(status, 0, "GetSecurityInfo failed");
        assert!(!descriptor.is_null());
        let descriptor = SecurityDescriptor(descriptor);
        let mut text = std::ptr::null_mut();
        let mut length = 0;
        // SAFETY: the descriptor stays live through conversion, and outputs
        // point to initialized writable storage.
        let converted = unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor.0,
                1,
                4,
                &mut text,
                &mut length,
            )
        };
        assert_ne!(converted, 0, "DACL conversion failed");
        assert!(!text.is_null());
        // The string is also a LocalAlloc allocation; use the same private
        // allocation guard so assertion failures do not leak either buffer.
        let text_owner = SecurityDescriptor(text.cast());
        // SAFETY: Windows returned this many UTF-16 units in the allocation.
        let result =
            unsafe { String::from_utf16_lossy(std::slice::from_raw_parts(text, length as usize)) };
        drop(text_owner);
        result.trim_end_matches('\0').to_owned()
    }

    #[tokio::test]
    async fn bound_pipe_has_protected_two_trustee_dacl() {
        let endpoint = format!(r"\\.\pipe\kernal-api-dacl-{}", std::process::id());
        let pipe = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
        let sddl = bound_dacl(&pipe);
        assert!(sddl.starts_with("D:P"), "DACL must be protected: {sddl}");
        assert_eq!(sddl.matches('(').count(), 2, "unexpected trustees: {sddl}");
        assert!(sddl.contains(";;;SY)"), "SYSTEM grant missing: {sddl}");
        assert!(!sddl.contains(";;;WD)"), "Everyone grant: {sddl}");
        assert!(!sddl.contains(";;;AN)"), "Anonymous grant: {sddl}");
        assert!(!sddl.contains(";;;BA)"), "Administrators grant: {sddl}");
    }

    #[test]
    fn nul_is_rejected_before_runtime_or_native_creation() {
        assert_eq!(
            LocalPipeClient::open("invalid\0pipe").err().unwrap().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            OwnerOnlyPipeInstance::create("invalid\0pipe", true)
                .err()
                .unwrap()
                .kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[tokio::test]
    async fn connected_client_preserves_busy_error_and_byte_io() {
        let endpoint = format!(r"\\.\pipe\kernal-api-client-{}", std::process::id());
        let mut server = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
        let mut client = LocalPipeClient::open(&endpoint).unwrap();
        server.connect().await.unwrap();
        assert_eq!(
            LocalPipeClient::open(&endpoint)
                .err()
                .unwrap()
                .raw_os_error(),
            Some(231)
        );
        let written = std::future::poll_fn(|cx| client.poll_write(cx, b"x"))
            .await
            .unwrap();
        assert_eq!(written, 1);
        let mut bytes = [0; 1];
        let read = std::future::poll_fn(|cx| server.poll_read(cx, &mut bytes))
            .await
            .unwrap();
        assert_eq!(read, 1);
        assert_eq!(bytes, *b"x");
    }

    #[tokio::test]
    async fn first_instance_is_exclusive_but_pool_instances_can_follow() {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let endpoint = format!(
            r"\\.\pipe\kernal-api-instance-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let first = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
        assert!(OwnerOnlyPipeInstance::create(&endpoint, true).is_err());
        let second = OwnerOnlyPipeInstance::create(&endpoint, false).unwrap();
        drop((first, second));
        let _replacement = OwnerOnlyPipeInstance::create(&endpoint, true).unwrap();
    }
}
