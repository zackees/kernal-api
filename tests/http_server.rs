#![cfg(feature = "http-server")]

use kernal_api::{
    async_engine,
    http_server::{Limits, Response, Server},
};
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn loopback() -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)
}

#[tokio::test]
async fn diagnostics_survive_server_and_report_rejections_and_task_failures() {
    let server = Server::bind(
        loopback(),
        Limits {
            max_request_body_bytes: 1,
            max_response_body_bytes: 1,
            handler_timeout: Duration::from_millis(10),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let diagnostics = server.diagnostics();
    let task = async_engine::launch(server.serve(|request| async move {
        match request.target() {
            "/panic" => panic!("test handler panic"),
            "/timeout" => std::future::pending().await,
            _ => Response::new(200, b"too large".to_vec()).unwrap(),
        }
    }));
    for path in ["/response", "/timeout", "/panic"] {
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        exchange(addr, request.as_bytes()).await;
    }
    exchange(
        addr,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 2\r\nConnection: close\r\n\r\naa",
    )
    .await;
    async_engine::timeout(Duration::from_secs(2), async {
        while diagnostics.snapshot().task_failures != 1 {
            async_engine::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    task.cancel();
    let _ = task.await;
    let snapshot = diagnostics.snapshot();
    assert_eq!(snapshot.accepted_connections, 4);
    assert_eq!(snapshot.response_rejections, 1);
    assert_eq!(snapshot.handler_timeouts, 1);
    assert_eq!(snapshot.request_rejections, 1);
    assert_eq!(snapshot.task_failures, 1);
}

#[test]
fn bodyless_statuses_reject_payloads_and_connection_headers_are_private() {
    for status in [204, 205, 304] {
        assert!(Response::new(status, b"unexpected".to_vec()).is_err());
        assert!(Response::new(status, Vec::new()).is_ok());
    }
    for header in ["keep-alive", "proxy-connection", "te"] {
        assert!(Response::new(200, Vec::new())
            .unwrap()
            .with_header(header, "value")
            .is_err());
    }
}

#[tokio::test]
async fn diagnostics_report_protocol_body_and_absolute_deadlines() {
    let server = Server::bind(
        loopback(),
        Limits {
            body_timeout: Duration::from_millis(10),
            connection_timeout: Duration::from_millis(50),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let diagnostics = server.diagnostics();
    let task =
        async_engine::launch(server.serve(|_| async { Response::new(200, Vec::new()).unwrap() }));
    exchange(addr, b"INVALID REQUEST\r\n\r\n").await;
    exchange(
        addr,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 1\r\nConnection: close\r\n\r\n",
    )
    .await;
    exchange(addr, b"GET / HTTP/1.1\r\n").await;
    async_engine::timeout(Duration::from_secs(2), async {
        loop {
            let s = diagnostics.snapshot();
            if s.connection_errors >= 1 && s.connection_timeouts == 1 && s.body_timeouts == 1 {
                break;
            }
            async_engine::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    drop(task);
}

#[tokio::test]
async fn response_header_acceptance_counts_duplicates_and_wire_bytes() {
    let server = Server::bind(
        loopback(),
        Limits {
            max_response_headers: 2,
            max_response_header_bytes: 16,
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(server.serve(|request| async move {
        let response = Response::new(200, Vec::new()).unwrap();
        match request.target() {
            "/count" => response
                .with_header("x", "")
                .unwrap()
                .with_header("x", "")
                .unwrap()
                .with_header("x", "")
                .unwrap(),
            "/bytes" => response.with_header("x", "123456789012").unwrap(),
            _ => response.with_header("x", "12345678901").unwrap(),
        }
    }));
    for (path, status) in [("/count", 500), ("/bytes", 500), ("/exact", 200)] {
        let request =
            format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
        let response = exchange(addr, request.as_bytes()).await;
        assert!(
            response.starts_with(&format!("HTTP/1.1 {status}")),
            "{response}"
        );
    }
    drop(task);
}

async fn exchange(addr: SocketAddr, request: &[u8]) -> String {
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(request).await.unwrap();
    let mut response = Vec::new();
    async_engine::timeout(Duration::from_secs(2), socket.read_to_end(&mut response))
        .await
        .unwrap()
        .unwrap();
    String::from_utf8(response).unwrap()
}

#[tokio::test]
async fn owned_request_response_preserves_method_target_headers_and_body() {
    let server = Server::bind(loopback(), Limits::default()).await.unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(server.serve(|request| async move {
        assert_eq!(request.method(), "POST");
        assert_eq!(request.target(), "/echo?x=1");
        assert_eq!(request.header("x-test"), Some(b"value".as_slice()));
        Response::new(201, request.body().to_vec()).unwrap()
    }));
    let response = exchange(addr, b"POST /echo?x=1 HTTP/1.1\r\nHost: localhost\r\nX-Test: value\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping").await;
    assert!(response.starts_with("HTTP/1.1 201"), "{response}");
    assert!(response.ends_with("ping"), "{response}");
    drop(task);
}

#[tokio::test]
async fn oversized_request_is_rejected_before_dispatch() {
    let server = Server::bind(
        loopback(),
        Limits {
            max_request_body_bytes: 3,
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task =
        async_engine::launch(server.serve(|_| async { panic!("oversized body reached handler") }));
    let response = exchange(
        addr,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: close\r\n\r\nping",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 413"), "{response}");
    drop(task);
}

#[tokio::test]
async fn cancelling_server_closes_listener_and_incomplete_connections() {
    let server = Server::bind(loopback(), Limits::default()).await.unwrap();
    let addr = server.local_addr().unwrap();
    let task =
        async_engine::launch(server.serve(|_| async { Response::new(200, Vec::new()).unwrap() }));
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
    async_engine::sleep(Duration::from_millis(20)).await;
    task.cancel();
    assert!(task.await.unwrap_err().is_cancelled());
    let mut byte = [0];
    let read = async_engine::timeout(Duration::from_secs(2), socket.read(&mut byte))
        .await
        .unwrap();
    assert!(matches!(read, Ok(0) | Err(_)));
    assert!(tokio::net::TcpStream::connect(addr).await.is_err());
}

#[tokio::test]
async fn invalid_limits_fail_before_binding() {
    assert!(Server::bind(
        loopback(),
        Limits {
            max_connections: 0,
            ..Limits::default()
        }
    )
    .await
    .is_err());
    assert!(Server::bind(
        loopback(),
        Limits {
            header_timeout: Duration::ZERO,
            ..Limits::default()
        }
    )
    .await
    .is_err());
    assert!(Response::new(99, Vec::new()).is_err());
}

#[tokio::test]
async fn connection_capacity_defers_acceptance_until_an_owned_connection_finishes() {
    let server = Server::bind(
        loopback(),
        Limits {
            max_connections: 1,
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(
        server.serve(|_| async { Response::new(200, b"ok".to_vec()).unwrap() }),
    );
    let mut first = tokio::net::TcpStream::connect(addr).await.unwrap();
    first.write_all(b"GET / HTTP/1.1\r\n").await.unwrap();
    let mut second = tokio::net::TcpStream::connect(addr).await.unwrap();
    second
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut byte = [0];
    assert!(
        async_engine::timeout(Duration::from_millis(30), second.read(&mut byte))
            .await
            .is_err()
    );
    drop(first);
    let mut response = String::new();
    async_engine::timeout(Duration::from_secs(2), second.read_to_string(&mut response))
        .await
        .unwrap()
        .unwrap();
    assert!(response.ends_with("ok"), "{response}");
    drop(task);
}

#[tokio::test]
async fn body_and_handler_deadlines_return_explicit_statuses() {
    let server = Server::bind(
        loopback(),
        Limits {
            body_timeout: Duration::from_millis(30),
            handler_timeout: Duration::from_millis(30),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task =
        async_engine::launch(server.serve(|_| async { std::future::pending::<Response>().await }));
    let response = exchange(
        addr,
        b"POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 4\r\nConnection: close\r\n\r\np",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 408"), "{response}");
    let response = exchange(
        addr,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 504"), "{response}");
    drop(task);
}

#[tokio::test]
async fn header_and_connection_deadlines_close_stalled_clients() {
    for header in [true, false] {
        let mut limits = Limits::default();
        if header {
            limits.header_timeout = Duration::from_millis(30);
        } else {
            limits.connection_timeout = Duration::from_millis(30);
        }
        let server = Server::bind(loopback(), limits).await.unwrap();
        let addr = server.local_addr().unwrap();
        let task = async_engine::launch(
            server.serve(|_| async { panic!("incomplete request dispatched") }),
        );
        let response = exchange(addr, b"GET / HTTP/1.1\r\n").await;
        assert!(
            response.is_empty() || response.starts_with("HTTP/1.1 408"),
            "{response}"
        );
        drop(task);
    }
}

#[tokio::test]
async fn response_limits_and_transport_owned_headers_are_enforced() {
    for header in [
        "content-length",
        "transfer-encoding",
        "connection",
        "upgrade",
        "trailer",
    ] {
        assert!(Response::new(200, Vec::new())
            .unwrap()
            .with_header(header, "x")
            .is_err());
    }
    assert!(Response::new(200, Vec::new())
        .unwrap()
        .with_header("x-test", "injected\r\nX: y")
        .is_err());
    let server = Server::bind(
        loopback(),
        Limits {
            max_response_body_bytes: 1,
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(
        server.serve(|_| async { Response::new(200, b"too long".to_vec()).unwrap() }),
    );
    let response = exchange(
        addr,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 500"), "{response}");
    drop(task);
}

#[tokio::test]
async fn file_response_streams_prefix_and_file_beyond_the_memory_body_limit() {
    use std::io::Write;
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(&vec![b'x'; 256 * 1024]).unwrap();
    let path = file.path().to_path_buf();
    let server = Server::bind(
        loopback(),
        Limits {
            max_response_body_bytes: 8,
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(server.serve(move |_| {
        let file = std::fs::File::open(&path).unwrap();
        async move { Response::file(file, b"PREFIX".to_vec()).unwrap() }
    }));
    let response = exchange(
        addr,
        b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    assert_eq!(body.len(), 256 * 1024 + 6);
    assert!(body.starts_with("PREFIX"));
    assert!(body[6..].bytes().all(|byte| byte == b'x'));
    drop(task);
}

#[tokio::test]
async fn a_client_that_stops_reading_releases_its_connection_slot() {
    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(128 * 1024 * 1024).unwrap();
    let path = file.path().to_path_buf();
    let server = Server::bind(
        loopback(),
        Limits {
            max_connections: 1,
            write_timeout: Duration::from_millis(30),
            ..Limits::default()
        },
    )
    .await
    .unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(server.serve(move |request| {
        let path = path.clone();
        async move {
            if request.target() == "/file" {
                Response::file(std::fs::File::open(path).unwrap(), Vec::new()).unwrap()
            } else {
                Response::new(200, b"ok".to_vec()).unwrap()
            }
        }
    }));
    let mut stalled = tokio::net::TcpStream::connect(addr).await.unwrap();
    stalled
        .write_all(b"GET /file HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let response = exchange(
        addr,
        b"GET /ping HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    )
    .await;
    assert!(response.ends_with("ok"), "{response}");
    drop(stalled);
    drop(task);
}

#[cfg(feature = "event-stream")]
#[tokio::test]
async fn sse_response_delivers_before_source_closure_and_encodes_multiline_data() {
    let (sender, receiver) = async_engine::broadcast_channel::<String>(4).unwrap();
    let receiver = std::sync::Arc::new(std::sync::Mutex::new(Some(receiver)));
    let server = Server::bind(loopback(), Limits::default()).await.unwrap();
    let addr = server.local_addr().unwrap();
    let task = async_engine::launch(server.serve(move |_| {
        let receiver = receiver.lock().unwrap().take().unwrap();
        async move {
            Response::event_stream(
                receiver.into_stream_with(|result| result.ok().map(Ok)),
                Duration::from_secs(1),
            )
            .unwrap()
        }
    }));
    let mut socket = tokio::net::TcpStream::connect(addr).await.unwrap();
    socket
        .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    sender.send("first\nsecond".into()).unwrap();
    let mut received = String::new();
    async_engine::timeout(Duration::from_secs(2), async {
        while !received.contains("data: first\ndata: second\n\n") {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            received.push_str(std::str::from_utf8(&buffer[..count]).unwrap());
        }
    })
    .await
    .unwrap();
    assert!(received.contains("text/event-stream"));
    drop(sender);
    async_engine::timeout(Duration::from_secs(2), socket.read_to_string(&mut received))
        .await
        .unwrap()
        .unwrap();
    drop(task);
}
