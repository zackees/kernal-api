//! Bounded native HTTP/1 serving on the caller's runtime.
//!
//! Applications own routing, authorization and response policy. Dropping the
//! serving future closes the listener and cancels all owned connections.

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{body::Incoming, service::service_fn};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::{convert::Infallible, future::Future, io, net::SocketAddr, time::Duration};

/// Per-server resource and time limits. Body limits bound accepted values, not
/// memory already allocated by application handlers before returning a response.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// Maximum concurrently owned connections. The accept loop waits at capacity.
    pub max_connections: usize,
    /// Maximum collected request body bytes.
    pub max_request_body_bytes: usize,
    /// Maximum accepted in-memory response body bytes.
    pub max_response_body_bytes: usize,
    /// HTTP/1 parser read-buffer bound (at least 8192 bytes).
    pub max_header_bytes: usize,
    /// Maximum parsed header count.
    pub max_headers: usize,
    /// Maximum time to receive each request's headers.
    pub header_timeout: Duration,
    /// Maximum time to collect a request body.
    pub body_timeout: Duration,
    /// Maximum time for application response preparation.
    pub handler_timeout: Duration,
    /// Absolute lifetime of a connection, including response transmission.
    pub connection_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: 64,
            max_request_body_bytes: 2 * 1024 * 1024,
            max_response_body_bytes: 64 * 1024 * 1024,
            max_header_bytes: 32 * 1024,
            max_headers: 100,
            header_timeout: Duration::from_secs(10),
            body_timeout: Duration::from_secs(30),
            handler_timeout: Duration::from_secs(30),
            connection_timeout: Duration::from_secs(3600),
        }
    }
}

impl Limits {
    fn validate(self) -> io::Result<()> {
        if self.max_connections == 0
            || self.max_connections > 65_536
            || !(8192..=1024 * 1024).contains(&self.max_header_bytes)
            || !(1..=1024).contains(&self.max_headers)
            || self.max_request_body_bytes > 1024 * 1024 * 1024
            || self.max_response_body_bytes > 1024 * 1024 * 1024
            || [
                self.header_timeout,
                self.body_timeout,
                self.handler_timeout,
                self.connection_timeout,
            ]
            .into_iter()
            .any(|d| d.is_zero() || d > Duration::from_secs(365 * 24 * 3600))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid HTTP server limits",
            ));
        }
        Ok(())
    }
}

/// A fully collected, bounded request; no implementation body type escapes.
#[derive(Debug)]
pub struct Request {
    method: String,
    target: String,
    headers: hyper::HeaderMap,
    body: Vec<u8>,
}

impl Request {
    /// HTTP method token.
    pub fn method(&self) -> &str {
        &self.method
    }
    /// Original request target including an optional query string.
    pub fn target(&self) -> &str {
        &self.target
    }
    /// First matching header's raw bytes; names are case-insensitive.
    pub fn header(&self, name: &str) -> Option<&[u8]> {
        self.headers.get(name).map(|value| value.as_bytes())
    }
    /// Accepted body bytes.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

/// An application-selected HTTP response with validated status and headers.
#[derive(Debug)]
pub struct Response {
    status: hyper::StatusCode,
    headers: hyper::HeaderMap,
    body: Vec<u8>,
}

impl Response {
    /// Construct a response. The server separately enforces its body-size limit.
    ///
    /// # Errors
    /// Rejects status values outside 200..=599; interim responses are transport-owned.
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> io::Result<Self> {
        if !(200..=599).contains(&status) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid final HTTP status",
            ));
        }
        Ok(Self {
            status: hyper::StatusCode::from_u16(status).map_err(io::Error::other)?,
            headers: hyper::HeaderMap::new(),
            body: body.into(),
        })
    }

    /// Append an application header. Framing is always transport-owned.
    ///
    /// # Errors
    /// Rejects invalid syntax and connection/body-framing headers.
    pub fn with_header(mut self, name: &str, value: &str) -> io::Result<Self> {
        let name =
            hyper::header::HeaderName::from_bytes(name.as_bytes()).map_err(io::Error::other)?;
        if matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection" | "upgrade" | "trailer"
        ) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "HTTP framing header is transport-owned",
            ));
        }
        let value = hyper::header::HeaderValue::from_str(value).map_err(io::Error::other)?;
        self.headers.append(name, value);
        Ok(self)
    }
}

/// A bound listener. Serving uses the current runtime and never starts another.
#[derive(Debug)]
pub struct Server {
    listener: tokio::net::TcpListener,
    limits: Limits,
}

impl Server {
    /// Bind a caller-selected address after validating limits.
    ///
    /// # Errors
    /// Reports invalid limits and native listener errors.
    pub async fn bind(address: SocketAddr, limits: Limits) -> io::Result<Self> {
        limits.validate()?;
        Ok(Self {
            listener: tokio::net::TcpListener::bind(address).await?,
            limits,
        })
    }

    /// The bound address, including the assigned port when zero was requested.
    ///
    /// # Errors
    /// Reports a native socket inspection failure.
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serve until cancelled or a listener error occurs. Connection tasks are
    /// owned by this future and cancelled on drop, including incomplete requests.
    ///
    /// # Errors
    /// Returns native listener errors; individual client failures are isolated.
    pub async fn serve<H, F>(self, handler: H) -> io::Result<()>
    where
        H: Fn(Request) -> F + Clone + Send + Sync + 'static,
        F: Future<Output = Response> + Send + 'static,
    {
        let mut tasks = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = tasks.join_next(), if !tasks.is_empty() => {}
                accepted = self.listener.accept(), if tasks.len() < self.limits.max_connections => {
                    let (socket, _) = accepted?;
                    let handler = handler.clone();
                    let limits = self.limits;
                    tasks.spawn(async move {
                        let service = service_fn(move |request| dispatch(request, handler.clone(), limits));
                        let mut builder = hyper::server::conn::http1::Builder::new();
                        builder.timer(TokioTimer::new())
                            .header_read_timeout(limits.header_timeout)
                            .max_buf_size(limits.max_header_bytes)
                            .max_headers(limits.max_headers);
                        let connection = builder.serve_connection(TokioIo::new(socket), service);
                        let _ = tokio::time::timeout(limits.connection_timeout, connection).await;
                    });
                }
            }
        }
    }
}

fn empty(status: hyper::StatusCode) -> hyper::Response<Full<Bytes>> {
    let mut response = hyper::Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

async fn dispatch<H, F>(
    request: hyper::Request<Incoming>,
    handler: H,
    limits: Limits,
) -> Result<hyper::Response<Full<Bytes>>, Infallible>
where
    H: Fn(Request) -> F,
    F: Future<Output = Response>,
{
    let (parts, body) = request.into_parts();
    let collected = tokio::time::timeout(
        limits.body_timeout,
        Limited::new(body, limits.max_request_body_bytes).collect(),
    )
    .await;
    let body = match collected {
        Err(_) => return Ok(empty(hyper::StatusCode::REQUEST_TIMEOUT)),
        Ok(Err(error)) => {
            return Ok(empty(if error.is::<http_body_util::LengthLimitError>() {
                hyper::StatusCode::PAYLOAD_TOO_LARGE
            } else {
                hyper::StatusCode::BAD_REQUEST
            }))
        }
        Ok(Ok(body)) => body.to_bytes().to_vec(),
    };
    let request = Request {
        method: parts.method.to_string(),
        target: parts.uri.to_string(),
        headers: parts.headers,
        body,
    };
    let response = match tokio::time::timeout(limits.handler_timeout, handler(request)).await {
        Ok(response) => response,
        Err(_) => return Ok(empty(hyper::StatusCode::GATEWAY_TIMEOUT)),
    };
    if response.body.len() > limits.max_response_body_bytes {
        return Ok(empty(hyper::StatusCode::INTERNAL_SERVER_ERROR));
    }
    let mut result = hyper::Response::new(Full::new(Bytes::from(response.body)));
    *result.status_mut() = response.status;
    *result.headers_mut() = response.headers;
    Ok(result)
}
