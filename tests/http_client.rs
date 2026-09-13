#![cfg(feature = "http-client")]

use kernal_api::http::{Client, Limits, Method, Request};
use std::io::{Read, Write};

#[test]
fn blocking_adapter_streams_on_the_callers_runtime_and_rejects_nested_use() {
    use kernal_api::async_engine::RuntimeBuilder;
    use kernal_api::http::BlockingClient;
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = BlockingClient::new(&runtime, Limits::default()).unwrap();
    let (url, worker) =
        fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef");
    let mut response = client.get(&url).unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("Content-Length"), Some(b"6".as_slice()));
    let mut prefix = [0; 2];
    runtime.run(async {
        assert_eq!(
            response.read(&mut prefix).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    });
    response.read_exact(&mut prefix).unwrap();
    assert_eq!(&prefix, b"ab");
    assert_eq!(response.into_bytes().unwrap(), b"cdef");
    worker.join().unwrap();
    runtime.run(async {
        assert!(BlockingClient::new(&runtime, Limits::default()).is_err());
        assert!(client.get("http://127.0.0.1:1/").is_err());
    });
}

#[test]
fn blocking_stream_preserves_overflow_and_timeout_errors() {
    use kernal_api::async_engine::RuntimeBuilder;
    use kernal_api::http::BlockingClient;
    let runtime = RuntimeBuilder::current_thread()
        .enable_all()
        .build()
        .unwrap();
    let client = BlockingClient::new(
        &runtime,
        Limits {
            max_body_bytes: 3,
            ..Limits::default()
        },
    )
    .unwrap();
    let (url, worker) = fixture(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcdef");
    let mut response = client.get(&url).unwrap();
    assert_eq!(
        std::io::copy(&mut response, &mut std::io::sink())
            .unwrap_err()
            .kind(),
        std::io::ErrorKind::InvalidData
    );
    worker.join().unwrap();

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
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\n\r\n")
            .unwrap();
        let _ = held.recv_timeout(std::time::Duration::from_secs(2));
    });
    let client = BlockingClient::new(
        &runtime,
        Limits {
            read_timeout: std::time::Duration::from_millis(50),
            ..Limits::default()
        },
    )
    .unwrap();
    let response = client.get(&url).unwrap();
    let result = response.into_bytes();
    let _ = release.send(());
    worker.join().unwrap();
    assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
}

fn fixture(response: &[u8]) -> (String, std::thread::JoinHandle<()>) {
    let response = response.to_vec();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        let _ = socket.write_all(&response);
    });
    (url, worker)
}

#[tokio::test]
async fn opted_in_redirects_follow_and_enforce_hop_limit() {
    let (final_url, final_worker) =
        fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok");
    let redirect = format!("HTTP/1.1 302 Found\r\nLocation: {final_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (url, worker) = fixture(redirect.as_bytes());
    let response = Client::new(Limits {
        max_redirects: 1,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.into_bytes().await.unwrap(), b"ok");
    worker.join().unwrap();
    final_worker.join().unwrap();

    let (middle, middle_worker) = fixture(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let redirect = format!("HTTP/1.1 302 Found\r\nLocation: {middle}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let (url, worker) = fixture(redirect.as_bytes());
    let error = Client::new(Limits {
        max_redirects: 1,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    worker.join().unwrap();
    middle_worker.join().unwrap();
}

#[tokio::test]
async fn redirects_drop_cross_origin_headers_and_convert_post_to_get() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let target = format!("http://{}/next", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            socket.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            assert!(request.len() < 4096);
        }
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8(request).unwrap()
    });
    let (url, first) = fixture(format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
    let response = Client::new(Limits {
        max_redirects: 2,
        ..Limits::default()
    })
    .unwrap()
    .execute(Request {
        method: Method::Post,
        headers: &[("X-Secret", "private"), ("Content-Type", "text/plain")],
        body: b"ping",
        ..Request::get(&url)
    })
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    first.join().unwrap();
    let wire = worker.join().unwrap().to_ascii_lowercase();
    assert!(wire.starts_with("get /next http/1.1"));
    assert!(!wire.contains("x-secret"));
    assert!(!wire.contains("content-type"));
}

#[tokio::test]
async fn redirect_hops_reject_unsafe_urls_and_oversized_metadata() {
    for target in ["file:///secret", "http://user:secret@127.0.0.1:1/"] {
        let (url, worker) = fixture(format!("HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").as_bytes());
        let error = Client::new(Limits {
            max_redirects: 1,
            ..Limits::default()
        })
        .unwrap()
        .get(&url)
        .await
        .err()
        .unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        worker.join().unwrap();
    }
    let (url, worker) = fixture(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nX-Large: abcdef\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let error = Client::new(Limits {
        max_redirects: 1,
        max_header_bytes: 8,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .err()
    .unwrap();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    worker.join().unwrap();
}

#[tokio::test]
async fn total_deadline_applies_to_already_buffered_unread_body() {
    let (url, worker) =
        fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef");
    let mut response = Client::new(Limits {
        total_timeout: std::time::Duration::from_millis(100),
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .unwrap();
    assert_eq!(response.read(&mut [0; 1]).await.unwrap(), 1);
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    assert_eq!(
        response.read(&mut [0; 1]).await.unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
    worker.join().unwrap();
}

#[tokio::test]
async fn streaming_returns_before_body_completion_and_collection_keeps_only_unread_bytes() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let (release, held) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request).unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabc")
            .unwrap();
        held.recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        socket.write_all(b"def").unwrap();
    });
    let mut response = Client::new(Limits {
        max_body_bytes: 6,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .unwrap();
    let mut prefix = [0; 3];
    let mut offset = 0;
    while offset < prefix.len() {
        let n = response.read(&mut prefix[offset..]).await.unwrap();
        assert_ne!(n, 0);
        offset += n;
    }
    assert_eq!(&prefix, b"abc");
    release.send(()).unwrap();
    assert_eq!(response.into_bytes().await.unwrap(), b"def");
    worker.join().unwrap();
}

#[tokio::test]
async fn streaming_reads_respect_caller_buffers_and_stay_failed_after_overflow() {
    let (url, worker) =
        fixture(b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nConnection: close\r\n\r\nabcdef");
    let mut response = Client::new(Limits::default())
        .unwrap()
        .get(&url)
        .await
        .unwrap();
    assert_eq!(response.read(&mut []).await.unwrap(), 0);
    let mut buffer = [0; 2];
    let mut body = Vec::new();
    loop {
        let n = response.read(&mut buffer).await.unwrap();
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buffer[..n]);
    }
    assert_eq!(body, b"abcdef");
    assert_eq!(response.read(&mut buffer).await.unwrap(), 0);
    worker.join().unwrap();

    let (url, worker) = fixture(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nabcdef");
    let mut response = Client::new(Limits {
        max_body_bytes: 4,
        ..Limits::default()
    })
    .unwrap()
    .get(&url)
    .await
    .unwrap();
    loop {
        match response.read(&mut buffer).await {
            Ok(n) => assert_ne!(n, 0, "overflow must not become EOF"),
            Err(error) => {
                assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
                break;
            }
        }
    }
    assert!(response.read(&mut buffer).await.is_err());
    assert!(response.into_bytes().await.is_err());
    worker.join().unwrap();
}

#[tokio::test]
async fn head_preserves_headers_without_treating_entity_length_as_body() {
    let (url, worker) = fixture(
        b"HTTP/1.1 200 OK\r\nContent-Length: 1000000\r\nX-Test: value\r\nConnection: close\r\n\r\n",
    );
    let client = Client::new(Limits {
        max_body_bytes: 0,
        ..Limits::default()
    })
    .unwrap();
    let response = client
        .execute(Request {
            method: Method::Head,
            ..Request::get(&url)
        })
        .await
        .unwrap();
    assert_eq!(response.header("X-Test"), Some(b"value".as_slice()));
    assert!(response.into_bytes().await.unwrap().is_empty());
    worker.join().unwrap();
}

#[tokio::test]
async fn post_sends_application_headers_and_body() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/debug", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut wire = Vec::new();
        while !wire.ends_with(b"ping") {
            let mut bytes = [0; 256];
            let n = socket.read(&mut bytes).unwrap();
            assert_ne!(n, 0);
            wire.extend_from_slice(&bytes[..n]);
            assert!(wire.len() < 4096);
        }
        socket
            .write_all(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        wire
    });
    let client = Client::new(Limits::default()).unwrap();
    let response = client
        .execute(Request {
            method: Method::Post,
            headers: &[
                ("Content-Type", "application/json"),
                ("User-Agent", "fixture"),
            ],
            body: b"ping",
            ..Request::get(&url)
        })
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let wire = String::from_utf8(worker.join().unwrap()).unwrap();
    assert!(wire.starts_with("POST /debug HTTP/1.1\r\n"));
    assert!(wire
        .to_lowercase()
        .contains("content-type: application/json\r\n"));
    assert!(wire.ends_with("\r\n\r\nping"));
}

#[tokio::test]
async fn request_limits_and_invalid_headers_fail_before_connecting() {
    let client = Client::new(Limits {
        max_request_bytes: 4,
        ..Limits::default()
    })
    .unwrap();
    let url = "http://127.0.0.1:1/";
    for headers in [
        &[("bad name", "value")][..],
        &[("X-Test", "value\r\nInjected: yes")][..],
        &[("Content-Length", "100")][..],
        &[("Host", "elsewhere")][..],
    ] {
        let result = client
            .execute(Request {
                headers,
                ..Request::get(url)
            })
            .await;
        assert_eq!(
            result.err().unwrap().kind(),
            std::io::ErrorKind::InvalidData
        );
    }
    let result = client
        .execute(Request {
            method: Method::Post,
            body: b"12345",
            ..Request::get(url)
        })
        .await;
    assert_eq!(
        result.err().unwrap().kind(),
        std::io::ErrorKind::InvalidData
    );
    let headers = vec![("a", ""); 32769];
    let result = client
        .execute(Request {
            headers: &headers,
            ..Request::get(url)
        })
        .await;
    assert_eq!(
        result.err().unwrap().kind(),
        std::io::ErrorKind::InvalidData
    );
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

#[test]
fn unrepresentable_deadlines_are_rejected_when_building_the_client() {
    for limits in [
        Limits {
            connect_timeout: std::time::Duration::MAX,
            ..Limits::default()
        },
        Limits {
            total_timeout: std::time::Duration::MAX,
            ..Limits::default()
        },
        Limits {
            read_timeout: std::time::Duration::MAX,
            ..Limits::default()
        },
    ] {
        assert!(
            Client::new(limits).is_err(),
            "unrepresentable deadline accepted"
        );
    }
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
