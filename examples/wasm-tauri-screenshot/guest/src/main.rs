//! The screenshot application lives here, inside the actual Wasm module.
//! No URL, native path, image bytes, host runtime, or platform API enters it.

use kernal_api_v1_bindings::{self as kernel, OperationError, OutputFile, WebviewUrl};

async fn screenshot() -> Result<(), OperationError> {
    let url = WebviewUrl::granted()?.ok_or(OperationError::Rejected)?;
    let output = OutputFile::granted()?.ok_or(OperationError::Rejected)?;
    let view = url.open().await?;
    view.wait_until_loaded().await?;
    // Submitted only after the matching top-level load completion. The host
    // monotonic timer owns the wait; there is no guest clock import.
    kernel::clock_sleep(5_000)?.wait().await?;
    let snapshot = view.capture_visible_png().await?;
    output.write_blob(&snapshot)?.wait().await?;
    snapshot.close()?.wait().await?;
    view.close().await?;
    Ok(())
}

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    match kernel::run(screenshot()) {
        Ok(()) => 0,
        Err(OperationError::Rejected) => 1,
        Err(OperationError::Cancelled) => 2,
        Err(OperationError::Closed) => 3,
        Err(OperationError::Failed) => 4,
    }
}

fn main() {
    if kernal_api_run() != 0 {
        // Fail the threaded command entry as well as the diagnostic export.
        // Do not print through WASI or report success after a rejected step.
        std::arch::wasm32::unreachable();
    }
}
