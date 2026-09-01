//! Small real Rust guest used to characterize the wasm32-wasip1-threads ABI.
//!
//! Keep this free of output, environment, filesystem, networking, clocks, and
//! randomness. It is an artifact-profile fixture, not an example application.

use dashmap::DashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::BuildHasherDefault;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

#[link(wasm_import_module = "kernal-api:v1")]
extern "C" {
    #[link_name = "kernel-yield"]
    fn kernel_yield();
}

/// Explicit result marker for public artifact inspection.
#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    let counter = Arc::new(AtomicU32::new(0));
    let totals = Arc::new(Mutex::new(0_u32));
    // Use an explicit deterministic hasher: the closed threaded P1 surface
    // intentionally owns no ambient `random_get` authority.
    let map = Arc::new(DashMap::<u32, u32, BuildHasherDefault<DefaultHasher>>::with_hasher(
        BuildHasherDefault::default(),
    ));
    let (tx, rx) = mpsc::channel();
    let mut workers = Vec::new();
    for key in 0..2_u32 {
        let counter = Arc::clone(&counter);
        let totals = Arc::clone(&totals);
        let map = Arc::clone(&map);
        let tx = tx.clone();
        workers.push(std::thread::spawn(move || {
            // Each native child crosses the kernel boundary too, proving the
            // supplied runtime handle is observed in every guest Store.
            unsafe { kernel_yield() };
            counter.fetch_add(1, Ordering::SeqCst);
            *totals.lock().expect("mutex") += 1;
            map.insert(key, 1_u32);
            tx.send(1_u32).expect("channel");
        }));
    }
    drop(tx);
    let joined = workers
        .into_iter()
        .map(|worker| u32::from(worker.join().is_ok()))
        .sum::<u32>();
    let channel_total: u32 = rx.iter().sum();
    let map_sum: u32 = map.iter().map(|entry| *entry.value()).sum();
    let mutex_total = *totals.lock().expect("mutex");
    unsafe { kernel_yield() };
    joined + counter.load(Ordering::SeqCst) + mutex_total + channel_total + map_sum
}

fn main() {
    let _ = kernal_api_run();
}
