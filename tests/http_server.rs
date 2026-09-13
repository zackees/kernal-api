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
