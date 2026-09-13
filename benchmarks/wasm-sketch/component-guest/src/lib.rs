//! Private Component Model toolchain probe for issue #13, not a public guest API.
mod bindings {
    use super::Sketch;
    wit_bindgen::generate!({ path: "wit", world: "sketch" });
    export!(Sketch);
}

struct Sketch;

impl bindings::Guest for Sketch {
    async fn run() -> Result<u64, ()> {
        let blob = bindings::kernal::probe::blobs::granted().ok_or(())?;
        let mut stream = blob.read();
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
                }
                wit_bindgen::StreamResult::Dropped => return Ok(total),
                wit_bindgen::StreamResult::Cancelled => return Err(()),
            }
        }
    }
}
