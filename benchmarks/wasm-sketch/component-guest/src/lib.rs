//! Private Component Model toolchain probe for issue #13, not a public guest API.
mod bindings {
    use super::Sketch;
    wit_bindgen::generate!({ path: "wit", world: "sketch" });
    export!(Sketch);
}

struct Sketch;

impl bindings::Guest for Sketch {
    async fn cancel_read() -> Result<u64, ()> {
        use std::{future::Future, task::Poll};
        let blob = bindings::kernal::probe::blobs::granted().await.ok_or(())?;
        let mut stream = blob.read().await;
        let (status, mut buffer) = stream.read(Vec::with_capacity(64 * 1024)).await;
        if status != wit_bindgen::StreamResult::Complete(64 * 1024) || buffer.len() != 64 * 1024 {
            return Err(());
        }
        buffer.clear();
        let mut read = Box::pin(stream.read(buffer));
        std::future::poll_fn(|cx| match read.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(Ok(())),
            Poll::Ready(_) => Poll::Ready(Err(())),
        })
        .await?;
        bindings::kernal::probe::blobs::await_pending_read().await;
        let (status, buffer) = read.as_mut().cancel();
        drop(read);
        if status != wit_bindgen::StreamResult::Cancelled || !buffer.is_empty() {
            return Err(());
        }
        drop(stream);
        drop(blob);
        Ok(64 * 1024)
    }

    async fn run() -> Result<u64, ()> {
        Self::consume(false).await
    }

    async fn slow_consumer() -> Result<u64, ()> {
        Self::consume(true).await
    }
}

impl Sketch {
    async fn consume(slow: bool) -> Result<u64, ()> {
        let blob = bindings::kernal::probe::blobs::granted().await.ok_or(())?;
        let mut stream = blob.read().await;
        let mut buffer = Vec::with_capacity(64 * 1024);
        let mut total = 0_u64;
        loop {
            let (status, received) = stream.read(buffer).await;
            buffer = received;
            match status {
                wit_bindgen::StreamResult::Complete(count) => {
                    if count == 0 || count != buffer.len() {
                        return Err(());
                    }
                    total = total.checked_add(count as u64).ok_or(())?;
                    if total > 64 * 1024 * 1024 {
                        return Err(());
                    }
                    buffer.clear();
                    if slow && total == 64 * 1024 {
                        bindings::kernal::probe::blobs::pause_consumer().await;
                    }
                }
                wit_bindgen::StreamResult::Dropped => return Ok(total),
                wit_bindgen::StreamResult::Cancelled => return Err(()),
            }
        }
    }
}
