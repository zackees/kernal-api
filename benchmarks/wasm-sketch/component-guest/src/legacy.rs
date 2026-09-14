//! Legacy blob and public hash comparison fixture for issue #13.
#[path = "../../shared/hash_policy.rs"]
mod hash_policy;

// Adversarial wire checks intentionally bypass facade-side validation.
mod hash_wire {
    wit_bindgen::generate!({
        path: "../../../src/guest_component_hash.wit",
        world: "hash-client",
        type_section_suffix: "adversarial-wire",
    });
}

mod bindings {
    use super::Sketch;
    wit_bindgen::generate!({ path: "wit", world: "sketch" });
    export!(Sketch);
}

struct Sketch;

impl bindings::Guest for Sketch {
    async fn hash_proof() -> Result<(), ()> {
        hash_policy::proof().await.map_err(|_| ())
    }

    async fn hash_fail() -> Result<(), ()> {
        let _hash = kernal_api::guest::Blake3Hasher::new()
            .await
            .map_err(|_| ())?;
        Err(())
    }

    async fn hash_reject_overflow() -> Result<Vec<u8>, ()> {
        use hash_wire::kernal::hash_experiment::hashes;
        let hash = hashes::create().map_err(|_| ())?;
        let chunk = |used, last| hashes::InputChunk {
            word_0: 0,
            word_1: 0,
            word_2: 0,
            word_3: 0,
            word_4: 0,
            word_5: 0,
            word_6: 0,
            word_7: 0,
            word_8: 0,
            word_9: 0,
            word_10: 0,
            word_11: 0,
            word_12: 0,
            word_13: 0,
            word_14: 0,
            word_15: 0,
            word_16: 0,
            word_17: 0,
            word_18: 0,
            word_19: 0,
            word_20: 0,
            word_21: 0,
            word_22: 0,
            word_23: 0,
            word_24: 0,
            word_25: 0,
            word_26: 0,
            word_27: 0,
            word_28: 0,
            word_29: 0,
            word_30: 0,
            word_31: 0,
            used,
            last,
        };
        let update_result = {
            let (mut writer, reader) = hash_wire::wit_stream::new::<hashes::InputChunk>();
            let mut update = std::pin::pin!(hash.update(reader));
            if std::future::poll_fn(|context| {
                std::task::Poll::Ready(match std::future::Future::poll(update.as_mut(), context) {
                    std::task::Poll::Pending => false,
                    std::task::Poll::Ready(_) => true,
                })
            })
            .await
            {
                return Err(());
            }
            let mut chunks: Vec<_> = (0..256).map(|_| chunk(256, false)).collect();
            chunks.push(chunk(1, true));
            // A dropped stream may report that all frames were accepted by the
            // transport before the host reports the semantic rejection. The
            // update future is therefore the authoritative result here.
            let _undelivered = writer.write_all(chunks).await;
            drop(writer);
            update.await
        };
        if update_result != Err(hashes::Error::Rejected) {
            return Err(());
        }
        let trailing_result = {
            let (mut writer, reader) = hash_wire::wit_stream::new::<hashes::InputChunk>();
            let mut update = std::pin::pin!(hash.update(reader));
            if std::future::poll_fn(|context| {
                std::task::Poll::Ready(match std::future::Future::poll(update.as_mut(), context) {
                    std::task::Poll::Pending => false,
                    std::task::Poll::Ready(_) => true,
                })
            })
            .await
            {
                return Err(());
            }
            let mut chunks: Vec<_> = (0..255).map(|_| chunk(256, false)).collect();
            chunks.push(chunk(256, true));
            // A terminal frame followed in the same write batch must reject
            // rather than commit the terminal frame's staged bytes.
            chunks.push(chunk(1, false));
            let _undelivered = writer.write_all(chunks).await;
            drop(writer);
            update.await
        };
        if trailing_result != Err(hashes::Error::Rejected) {
            return Err(());
        }
        hashes::finish(hash).map_err(|_| ())
    }

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
