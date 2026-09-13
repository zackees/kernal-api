use super::Limits;
use bytes::Bytes;
use futures_core::Stream;
use hyper::body::{Body, Frame, SizeHint};
use std::{
    future::Future,
    io::{self, Seek},
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, ReadBuf};

enum Source {
    Bytes(Bytes),
    File(FileSource),
    Events(EventSource),
}

struct FileSource {
    file: tokio::fs::File,
    remaining: u64,
    prefix: Bytes,
    buffer: Vec<u8>,
}

struct EventSource {
    stream: Pin<Box<dyn Stream<Item = io::Result<String>> + Send>>,
    keepalive: Duration,
    timer: Pin<Box<tokio::time::Sleep>>,
    pending: Bytes,
    max_event: usize,
}

pub(super) struct ServerBody {
    source: Source,
    chunk: usize,
    done: bool,
}

impl std::fmt::Debug for ServerBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerBody")
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl ServerBody {
    pub(super) fn bytes(bytes: Bytes) -> Self {
        Self {
            source: Source::Bytes(bytes),
            chunk: 65536,
            done: false,
        }
    }

    pub(super) fn file(mut file: std::fs::File, prefix: Vec<u8>) -> io::Result<Self> {
        let metadata = file.metadata()?;
        if !metadata.is_file() || prefix.len() > 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "file response requires a regular file and bounded prefix",
            ));
        }
        let remaining = metadata.len().saturating_sub(file.stream_position()?);
        let mut file = tokio::fs::File::from_std(file);
        file.set_max_buf_size(65536);
        Ok(Self {
            source: Source::File(FileSource {
                file,
                remaining,
                prefix: prefix.into(),
                buffer: Vec::new(),
            }),
            chunk: 65536,
            done: false,
        })
    }

    pub(super) fn events<S>(events: S, keepalive: Duration) -> io::Result<Self>
    where
        S: Stream<Item = io::Result<String>> + Send + 'static,
    {
        if keepalive.is_zero() || keepalive > Duration::from_secs(365 * 24 * 3600) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid SSE keepalive",
            ));
        }
        crate::async_engine::RuntimeHandle::current().map_err(io::Error::other)?;
        Ok(Self {
            source: Source::Events(EventSource {
                stream: Box::pin(events),
                keepalive,
                timer: Box::pin(tokio::time::sleep(keepalive)),
                pending: Bytes::new(),
                max_event: 65536,
            }),
            chunk: 65536,
            done: false,
        })
    }

    pub(super) fn configure(&mut self, limits: Limits) -> io::Result<()> {
        self.chunk = limits.max_stream_chunk_bytes;
        let accepted = match &mut self.source {
            Source::Bytes(bytes) => bytes.len() <= limits.max_response_body_bytes,
            Source::File(file) => {
                file.file.set_max_buf_size(self.chunk);
                file.prefix.len() <= limits.max_response_body_bytes
                    && file
                        .remaining
                        .checked_add(file.prefix.len() as u64)
                        .is_some_and(|n| n <= limits.max_file_bytes)
            }
            Source::Events(events) => {
                events.max_event = limits.max_event_bytes;
                true
            }
        };
        if accepted {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "HTTP response exceeds configured limit",
            ))
        }
    }

    fn poll_data(&mut self, cx: &mut Context<'_>) -> Poll<Option<io::Result<Bytes>>> {
        match &mut self.source {
            Source::Bytes(bytes) => {
                if bytes.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(bytes.split_to(bytes.len().min(self.chunk)))))
                }
            }
            Source::File(file) => {
                if !file.prefix.is_empty() {
                    return Poll::Ready(Some(Ok(file
                        .prefix
                        .split_to(file.prefix.len().min(self.chunk)))));
                }
                if file.remaining == 0 {
                    return Poll::Ready(None);
                }
                let size = file.remaining.min(self.chunk as u64) as usize;
                file.buffer.resize(size, 0);
                let mut buffer = ReadBuf::new(&mut file.buffer);
                match std::task::ready!(Pin::new(&mut file.file).poll_read(cx, &mut buffer)) {
                    Err(error) => Poll::Ready(Some(Err(error))),
                    Ok(()) if buffer.filled().is_empty() => Poll::Ready(Some(Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "response file was truncated",
                    )))),
                    Ok(()) => {
                        file.remaining -= buffer.filled().len() as u64;
                        Poll::Ready(Some(Ok(Bytes::copy_from_slice(buffer.filled()))))
                    }
                }
            }
            Source::Events(events) => {
                if events.pending.is_empty() {
                    match events.stream.as_mut().poll_next(cx) {
                        Poll::Ready(None) => return Poll::Ready(None),
                        Poll::Ready(Some(Err(error))) => return Poll::Ready(Some(Err(error))),
                        Poll::Ready(Some(Ok(data))) => {
                            if data.len() > events.max_event {
                                return Poll::Ready(Some(Err(io::Error::new(
                                    io::ErrorKind::InvalidData,
                                    "SSE event exceeds configured limit",
                                ))));
                            }
                            let normalized = data.replace("\r\n", "\n").replace('\r', "\n");
                            let mut encoded = String::new();
                            for line in normalized.split('\n') {
                                encoded.push_str("data: ");
                                encoded.push_str(line);
                                encoded.push('\n');
                            }
                            encoded.push('\n');
                            events.pending = encoded.into();
                        }
                        Poll::Pending => {
                            std::task::ready!(events.timer.as_mut().poll(cx));
                            events.pending = Bytes::from_static(b":\n\n");
                        }
                    }
                    events
                        .timer
                        .as_mut()
                        .reset(tokio::time::Instant::now() + events.keepalive);
                }
                Poll::Ready(Some(Ok(events
                    .pending
                    .split_to(events.pending.len().min(self.chunk)))))
            }
        }
    }
}

impl Body for ServerBody {
    type Data = Bytes;
    type Error = io::Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, io::Error>>> {
        let this = self.get_mut();
        if this.done {
            return Poll::Ready(None);
        }
        match std::task::ready!(this.poll_data(cx)) {
            Some(Ok(bytes)) => Poll::Ready(Some(Ok(Frame::data(bytes)))),
            Some(Err(error)) => {
                this.done = true;
                Poll::Ready(Some(Err(error)))
            }
            None => {
                this.done = true;
                Poll::Ready(None)
            }
        }
    }

    fn is_end_stream(&self) -> bool {
        self.done
            || match &self.source {
                Source::Bytes(bytes) => bytes.is_empty(),
                Source::File(file) => file.prefix.is_empty() && file.remaining == 0,
                Source::Events(_) => false,
            }
    }

    fn size_hint(&self) -> SizeHint {
        if self.done {
            return SizeHint::with_exact(0);
        }
        match &self.source {
            Source::Bytes(bytes) => SizeHint::with_exact(bytes.len() as u64),
            Source::File(file) => {
                SizeHint::with_exact(file.remaining.saturating_add(file.prefix.len() as u64))
            }
            Source::Events(_) => SizeHint::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;
    use std::io::Write;

    #[tokio::test]
    async fn file_frames_bound_reads_and_prefix_does_not_read_ahead() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        file.write_all(&vec![b'x'; 8192]).unwrap();
        let source = std::fs::File::open(file.path()).unwrap();
        let mut observer = source.try_clone().unwrap();
        let mut body = ServerBody::file(source, b"prefix".to_vec()).unwrap();
        body.configure(Limits {
            max_stream_chunk_bytes: 1024,
            ..Limits::default()
        })
        .unwrap();
        assert_eq!(observer.stream_position().unwrap(), 0);
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            "prefix"
        );
        assert_eq!(observer.stream_position().unwrap(), 0);
        assert_eq!(
            body.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .len(),
            1024
        );
        assert_eq!(observer.stream_position().unwrap(), 1024);
        while let Some(frame) = body.frame().await {
            assert!(frame.unwrap().into_data().unwrap().len() <= 1024);
        }
    }

    #[tokio::test]
    async fn file_growth_is_ignored_and_truncation_is_sticky_failure() {
        let file = tempfile::NamedTempFile::new().unwrap();
        file.as_file().set_len(4).unwrap();
        let mut body =
            ServerBody::file(std::fs::File::open(file.path()).unwrap(), Vec::new()).unwrap();
        file.as_file().set_len(8).unwrap();
        assert_eq!(
            body.frame()
                .await
                .unwrap()
                .unwrap()
                .into_data()
                .unwrap()
                .len(),
            4
        );
        assert!(body.frame().await.is_none());
        let mut body =
            ServerBody::file(std::fs::File::open(file.path()).unwrap(), Vec::new()).unwrap();
        file.as_file().set_len(0).unwrap();
        assert_eq!(
            body.frame().await.unwrap().unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
        assert!(body.frame().await.is_none());
        assert!(body.is_end_stream());
    }

    struct Events {
        items: std::collections::VecDeque<String>,
        polls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Stream for Events {
        type Item = io::Result<String>;
        fn poll_next(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            self.polls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Poll::Ready(self.items.pop_front().map(Ok))
        }
    }

    #[tokio::test]
    async fn sse_chunks_do_not_prefetch_and_oversized_events_fail_stickily() {
        let polls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let events = Events {
            items: ["\r\nx".repeat(1000), "x".repeat(4001)].into(),
            polls: polls.clone(),
        };
        let mut body = ServerBody::events(events, Duration::from_secs(1)).unwrap();
        body.configure(Limits {
            max_stream_chunk_bytes: 1024,
            max_event_bytes: 4000,
            ..Limits::default()
        })
        .unwrap();
        assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 0);
        let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(frame.len(), 1024);
        let frame = body.frame().await.unwrap().unwrap().into_data().unwrap();
        assert_eq!(frame.len(), 1024);
        assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 1);
        loop {
            match body.frame().await.unwrap() {
                Ok(frame) => assert!(frame.into_data().unwrap().len() <= 1024),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
                    break;
                }
            }
        }
        assert!(body.frame().await.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn sse_keepalive_is_a_comment_and_carriage_returns_cannot_inject_fields() {
        struct Pending;
        impl Stream for Pending {
            type Item = io::Result<String>;
            fn poll_next(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Option<Self::Item>> {
                Poll::Pending
            }
        }
        let mut body = ServerBody::events(Pending, Duration::from_secs(10)).unwrap();
        let start = tokio::time::Instant::now();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            ":\n\n"
        );
        assert_eq!(start.elapsed(), Duration::from_secs(10));
        let events = Events {
            items: ["a\rb\r\nc".to_string()].into(),
            polls: Default::default(),
        };
        let mut body = ServerBody::events(events, Duration::from_secs(10)).unwrap();
        assert_eq!(
            body.frame().await.unwrap().unwrap().into_data().unwrap(),
            "data: a\ndata: b\ndata: c\n\n"
        );
    }
}
