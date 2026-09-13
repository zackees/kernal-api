#![cfg(feature = "http-client")]

use kernal_api::http::{Client, Limits};
use std::io::{Read, Write};

fn fixture(response: &'static [u8]) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        let _ = socket.write_all(response);
    });
    (url, worker)
}

#[tokio::test]
async fn bounded_get_preserves_status_and_body() {
    let (url, worker) =
        fixture(b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope");
    let response = Client::new(Limits::default())
        .unwrap()
        .get(&url)
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    assert_eq!(response.into_bytes().await.unwrap(), b"nope");
    worker.join().unwrap();
}

#[tokio::test]
async fn declared_and_streamed_overflow_are_rejected() {
    for wire in [
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n12345".as_slice(),
        b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n12345".as_slice(),
    ] {
        let (url, worker) = fixture(wire);
        let client = Client::new(Limits {
            max_body_bytes: 4,
            ..Limits::default()
        })
        .unwrap();
        let result = match client.get(&url).await {
            Ok(response) => response.into_bytes().await.map(|_| ()),
            Err(error) => Err(error),
        };
        assert!(result.is_err());
        worker.join().unwrap();
    }
}

#[tokio::test]
async fn redirects_are_returned_without_following_and_headers_are_bounded() {
    let (url, worker) = fixture(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let response = Client::new(Limits::default())
        .unwrap()
        .get(&url)
        .await
        .unwrap();
    assert_eq!(response.status(), 302);
    assert!(response.into_bytes().await.unwrap().is_empty());
    worker.join().unwrap();
    let (url, worker) = fixture(b"HTTP/1.1 200 OK\r\nX-Large: abcdef\r\nContent-Length: 0\r\n\r\n");
    assert!(Client::new(Limits {
        max_header_bytes: 4,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .is_err());
    worker.join().unwrap();
}

#[tokio::test]
async fn unsafe_urls_and_unbounded_timeout_configuration_fail() {
    let client = Client::new(Limits::default()).unwrap();
    for url in [
        "file:///etc/passwd",
        "ftp://localhost/file",
        "http://user:secret@localhost/",
        "not a URL",
    ] {
        assert!(client.get(url).await.is_err());
    }
    assert!(Client::new(Limits {
        total_timeout: std::time::Duration::ZERO,
        ..Limits::default()
    })
    .is_err());
}

#[tokio::test]
async fn stalled_headers_and_body_hit_read_deadline() {
    for headers in [
        b"".as_slice(),
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n".as_slice(),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        let (release, held) = std::sync::mpsc::channel::<()>();
        let worker = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(2)))
                .unwrap();
            let mut request = [0; 4096];
            let _ = socket.read(&mut request).unwrap();
            socket.write_all(headers).unwrap();
            let _ = held.recv_timeout(std::time::Duration::from_secs(2));
        });
        let client = Client::new(Limits {
            read_timeout: std::time::Duration::from_millis(50),
            ..Limits::default()
        })
        .unwrap();
        let result = match client.get(&url).await {
            Ok(response) => response.into_bytes().await.map(|_| ()),
            Err(error) => Err(error),
        };
        let _ = release.send(());
        worker.join().unwrap();
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
    }
}
