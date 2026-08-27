//! Small real Rust guest used to characterize the wasm32-wasip1-threads ABI.
//!
//! Keep this free of output, environment, filesystem, networking, clocks, and
//! randomness. It is an artifact-profile fixture, not an example application.

use dashmap::DashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[repr(C, align(4))]
struct ValidationRecord {
    words: [AtomicU32; 16],
}

static REPORT: ValidationRecord = ValidationRecord {
    words: [const { AtomicU32::new(0) }; 16],
};

// Retained verbatim as the validation-profile custom section. The ordinary
// threaded profile deliberately has no such extra metadata/export surface.
#[used]
#[link_section = "kernal-api.profile"]
static VALIDATION_PROFILE: [u8; 32] = *b"threaded-core-wasm-validation-v1";

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
    let totals = Arc::new(Mutex::new(0_u32));
    let map = Arc::new(DashMap::new());
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for key in 0..2_u32 {
        let counter = Arc::clone(&counter);
        let totals = Arc::clone(&totals);
        let map = Arc::clone(&map);
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            *totals.lock().expect("mutex") += 1;
            map.insert(key, 1_u32);
            tx.send(1_u32).expect("channel");
        }));
    }
    drop(tx);
    let joined = workers.into_iter().filter(|worker| worker.join().is_ok()).count() as u32;
    let channel_total: u32 = rx.iter().sum();
    let map_sum: u32 = map.iter().map(|entry| *entry.value()).sum();
    let mutex_total = *totals.lock().expect("mutex");
    let values = [0x4b_52_56_31, 1, 64, 0, 2, joined, counter.load(Ordering::SeqCst), mutex_total,
        channel_total, map.len() as u32, map_sum, 2, 0, 0, 0, 0];
    for (word, value) in REPORT.words.iter().zip(values) {
        word.store(value, Ordering::Relaxed);
    }
    REPORT.words[3].store(1, Ordering::Release);
    unsafe { kernel_yield() };
    joined
}

#[export_name = "kernal-api-threaded-validation-report-v1"]
pub extern "C" fn validation_report() -> i32 {
    REPORT.words.as_ptr() as usize as i32
}

fn main() {
    let _ = kernal_api_run();
}
