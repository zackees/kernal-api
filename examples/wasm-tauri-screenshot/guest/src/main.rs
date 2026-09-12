//! The screenshot application lives here, inside the actual Wasm module.
//! No URL, native path, image bytes, host runtime, or platform API enters it.

use kernal_api_v1_bindings::{self as kernel, OperationError, OutputFile, WebviewUrl};
#[allow(dead_code)]
#[path = "../../status.rs"]
mod status;
use status::{Cause, Failure, Step};

async fn screenshot(step: &mut Step) -> Result<(), OperationError> {
    let url = WebviewUrl::granted()?.ok_or(OperationError::Rejected)?;
    *step = Step::OutputGrant;
    let output = OutputFile::granted()?.ok_or(OperationError::Rejected)?;
    *step = Step::Open;
    let view = url.open().await?;
    *step = Step::Load;
    view.wait_until_loaded().await?;
    // Submitted only after the matching top-level load completion. The host
    // monotonic timer owns the wait; there is no guest clock import.
    *step = Step::Sleep;
    kernel::clock_sleep(5_000)?.wait().await?;
    *step = Step::Capture;
    let snapshot = view.capture_visible_png().await?;
    // Acceptance-only fault in the real guest, after the native result has
    // crossed the generated ABI. No native replacement orchestration runs.
    if cfg!(feature = "proof-trap-after-capture") {
        std::arch::wasm32::unreachable();
    }
    *step = Step::Write;
    output.write_blob(&snapshot)?.wait().await?;
    // Successful exact-output commit consumes the snapshot and output grants.
    *step = Step::Close;
    view.close().await?;
    Ok(())
}

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    let mut step = Step::UrlGrant;
    match kernel::run(screenshot(&mut step)) {
        Ok(()) => 0,
        Err(error) => Failure {
            step,
            cause: match error {
                OperationError::Rejected => Cause::Rejected,
                OperationError::Cancelled => Cause::Cancelled,
                OperationError::Closed => Cause::Closed,
                OperationError::Failed => Cause::Failed,
                OperationError::TimedOut => Cause::TimedOut,
            },
        }
        .code(),
    }
}

fn main() {
    let status = kernal_api_run();
    if status != 0 {
        // The already-admitted command-exit boundary preserves this scalar;
        // actual Wasm traps remain distinct from semantic operation failures.
        std::process::exit(status as i32);
    }
}
