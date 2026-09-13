//! Bounded HTTP client operations on the caller's async runtime.
//!
//! This initial surface does not follow redirects or attach credentials.
//! Response status interpretation and payload schemas belong to the caller.

use std::io;
use std::time::Duration;

/// Per-request ceilings. A client can be reused for requests with this policy.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum response body retained by the facade.
    pub max_body_bytes: u64,
    /// Maximum accepted response header names and values in aggregate.
    /// Checked after the backend has parsed headers, not before allocation.
    pub max_header_bytes: usize,
    /// Maximum connection-establishment duration.
    pub connect_timeout: Duration,
    /// Maximum time from request start through response-body completion.
    pub total_timeout: Duration,
    /// Maximum wait for the next network read, reset on read progress.
    pub read_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body_bytes: 4 * 1024 * 1024,
            max_header_bytes: 64 * 1024,
            connect_timeout: Duration::from_secs(10),
            total_timeout: Duration::from_secs(120),
            read_timeout: Duration::from_secs(30),
        }
    }
}

/// Reusable HTTP transport; does not construct an executor or runtime.
#[derive(Clone)]
pub struct Client {
    inner: reqwest::Client,
    limits: Limits,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn transport(error: reqwest::Error) -> io::Error {
    let kind = if error.is_timeout() {
        io::ErrorKind::TimedOut
    } else {
        io::ErrorKind::Other
    };
    // URLs can contain application secrets; diagnostics must not echo them.
    io::Error::new(kind, error.without_url())
}

impl Client {
    /// Build a transport with finite, nonzero timeouts and verified TLS.
    pub fn new(limits: Limits) -> io::Result<Self> {
        if limits.connect_timeout.is_zero()
            || limits.total_timeout.is_zero()
            || limits.read_timeout.is_zero()
        {
            return Err(invalid("HTTP timeouts must be nonzero"));
        }
        let inner = reqwest::Client::builder()
            .connect_timeout(limits.connect_timeout)
            .timeout(limits.total_timeout)
            .read_timeout(limits.read_timeout)
            .redirect(reqwest::redirect::Policy::none())
            .http1_only()
            .pool_max_idle_per_host(0)
            .build()
            .map_err(transport)?;
        Ok(Self { inner, limits })
    }

    /// Issue a GET, returning all statuses without automatically following 3xx.
    /// Drop the future or response to release the in-flight operation.
    pub async fn get(&self, url: &str) -> io::Result<Response> {
        let url = reqwest::Url::parse(url).map_err(|_| invalid("invalid HTTP URL"))?;
        if !matches!(url.scheme(), "http" | "https")
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid("HTTP URL scheme or embedded credentials rejected"));
        }
        let inner = self.inner.get(url).send().await.map_err(transport)?;
        let mut remaining_headers = self.limits.max_header_bytes;
        for (name, value) in inner.headers() {
            remaining_headers = remaining_headers
                .checked_sub(name.as_str().len())
                .and_then(|remaining| remaining.checked_sub(value.as_bytes().len()))
                .ok_or_else(|| invalid("HTTP response headers exceed limit"))?;
        }
        if inner
            .content_length()
            .is_some_and(|size| size > self.limits.max_body_bytes)
        {
            return Err(invalid("HTTP response body exceeds limit"));
        }
        Ok(Response {
            inner,
            remaining: self.limits.max_body_bytes,
        })
    }
}

/// Owned response, with backend resources released when dropped.
pub struct Response {
    inner: reqwest::Response,
    remaining: u64,
}

impl Response {
    /// Numeric HTTP status. Non-success status is not a transport error.
    pub fn status(&self) -> u16 {
        self.inner.status().as_u16()
    }

    /// Consume a small response under the request's explicit byte ceiling.
    /// No declared length is trusted for allocation or stream completion.
    pub async fn into_bytes(mut self) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        while let Some(chunk) = self.inner.chunk().await.map_err(transport)? {
            self.remaining = self
                .remaining
                .checked_sub(chunk.len() as u64)
                .ok_or_else(|| invalid("HTTP response body exceeds limit"))?;
            body.try_reserve_exact(chunk.len())
                .map_err(io::Error::other)?;
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}
