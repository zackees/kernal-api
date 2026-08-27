//! Small real Rust guest used to characterize the wasm32-wasip1-threads ABI.
//!
//! Keep this free of output, environment, filesystem, networking, clocks, and
//! randomness. It is an artifact-profile fixture, not an example application.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[link(wasm_import_module = "kernal-api:v1")]
extern "C" {
    #[link_name = "kernel-yield"]
    fn kernel_yield();
}

/// Explicit result marker for artifact inspection. The current #25 admission
/// profile is expected to reject this real std-thread guest before compiling it.
#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    let counter = Arc::new(AtomicU32::new(0));
    let worker_counter = Arc::clone(&counter);
    let worker = std::thread::spawn(move || {
        worker_counter.fetch_add(1, Ordering::SeqCst);
    });
    let joined = worker.join().is_ok();
    unsafe { kernel_yield() };
    counter.load(Ordering::SeqCst) + u32::from(joined)
}

fn main() {
    let _ = kernal_api_run();
}
