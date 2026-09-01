//! Small real Rust guest used to characterize the wasm32-wasip1-threads ABI.
//!
//! Keep this free of output, environment, filesystem, networking, clocks, and
//! randomness. It is an artifact-profile fixture, not an example application.

use dashmap::DashMap;
use std::cell::Cell;
use std::collections::hash_map::DefaultHasher;
use std::hash::BuildHasherDefault;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};

const REPORT_MAGIC: u32 = 0x4b52_5331; // "KRS1"
const REPORT_VERSION: u32 = 1;
const REPORT_BYTES: u32 = 48;

/// A fixed, atomics-only result record in the imported shared linear memory.
///
/// It is deliberately not exported: the closed threaded command ABI remains
/// unchanged. The diagnostic host locates this initialized record by its
/// versioned header and observes it only with atomic loads after `ready`.
#[repr(C)]
struct ResultRecord {
    magic: AtomicU32,
    version: AtomicU32,
    bytes: AtomicU32,
    ready: AtomicU32,
    joined_workers: AtomicU32,
    atomic_counter: AtomicU32,
    mutex_total: AtomicU32,
    channel_total: AtomicU32,
    map_total: AtomicU32,
    tls_total: AtomicU32,
    result: AtomicU32,
    reserved: AtomicU32,
}

impl ResultRecord {
    const fn new() -> Self {
        Self {
            magic: AtomicU32::new(REPORT_MAGIC),
            version: AtomicU32::new(REPORT_VERSION),
            bytes: AtomicU32::new(REPORT_BYTES),
            ready: AtomicU32::new(0),
            joined_workers: AtomicU32::new(0),
            atomic_counter: AtomicU32::new(0),
            mutex_total: AtomicU32::new(0),
            channel_total: AtomicU32::new(0),
            map_total: AtomicU32::new(0),
            tls_total: AtomicU32::new(0),
            result: AtomicU32::new(0),
            reserved: AtomicU32::new(0),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn publish(
        &self,
        joined_workers: u32,
        atomic_counter: u32,
        mutex_total: u32,
        channel_total: u32,
        map_total: u32,
        tls_total: u32,
        result: u32,
    ) {
        self.ready.store(0, Ordering::Relaxed);
        // Shared-memory linker initialization is a once-only module-start
        // concern. Rewrite the fixed header here as well, so the completed
        // record is self-identifying even when the host observes only after
        // root command execution.
        self.magic.store(REPORT_MAGIC, Ordering::Relaxed);
        self.version.store(REPORT_VERSION, Ordering::Relaxed);
        self.bytes.store(REPORT_BYTES, Ordering::Relaxed);
        self.joined_workers.store(joined_workers, Ordering::Relaxed);
        self.atomic_counter.store(atomic_counter, Ordering::Relaxed);
        self.mutex_total.store(mutex_total, Ordering::Relaxed);
        self.channel_total.store(channel_total, Ordering::Relaxed);
        self.map_total.store(map_total, Ordering::Relaxed);
        self.tls_total.store(tls_total, Ordering::Relaxed);
        self.result.store(result, Ordering::Relaxed);
        self.reserved.store(0, Ordering::Relaxed);
        // The host's acquire load synchronizes every prior record field.
        self.ready.store(1, Ordering::Release);
    }
}

static RESULT_RECORD: ResultRecord = ResultRecord::new();

std::thread_local! {
    // Each guest child must observe an independent Wasm TLS instance. If a
    // Store/Instance/TLS is accidentally reused, the second worker panics
    // instead of producing a false-green report.
    static CHILD_TLS: Cell<u32> = const { Cell::new(0) };
}

#[repr(C)]
struct Iovec {
    base: *const u8,
    len: u32,
}

#[link(wasm_import_module = "kernal-api:v1")]
extern "C" {
    #[link_name = "kernel-yield"]
    fn kernel_yield();
}

#[link(wasm_import_module = "wasi_snapshot_preview1")]
extern "C" {
    #[link_name = "fd_write"]
    fn fd_write(fd: i32, iovecs: *const Iovec, iovecs_len: i32, written: *mut u32) -> i32;
}

fn publish_report_to_host() {
    let iovec = Iovec {
        base: std::ptr::from_ref(&RESULT_RECORD).cast(),
        len: REPORT_BYTES,
    };
    let mut written = 0_u32;
    // The closed host validates both the iovec and the payload before its
    // discard-only write. Its test-only observer atomically snapshots this
    // exact bounded record; no guest export or extra authority is involved.
    unsafe {
        let _ = fd_write(3, &iovec, 1, &mut written);
    }
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
            let tls_value = CHILD_TLS.with(|tls| {
                assert_eq!(tls.replace(1), 0, "child must begin with fresh TLS");
                tls.get()
            });
            tx.send((1_u32, tls_value)).expect("channel");
        }));
    }
    drop(tx);
    let joined = workers
        .into_iter()
        .map(|worker| u32::from(worker.join().is_ok()))
        .sum::<u32>();
    let (channel_total, tls_total) = rx
        .iter()
        .fold((0_u32, 0_u32), |(messages, tls), (message, tls_value)| {
            (messages + message, tls + tls_value)
        });
    let map_sum: u32 = map.iter().map(|entry| *entry.value()).sum();
    let mutex_total = *totals.lock().expect("mutex");
    unsafe { kernel_yield() };
    let result = joined
        + counter.load(Ordering::SeqCst)
        + mutex_total
        + channel_total
        + map_sum
        + tls_total;
    RESULT_RECORD.publish(
        joined,
        counter.load(Ordering::SeqCst),
        mutex_total,
        channel_total,
        map_sum,
        tls_total,
        result,
    );
    publish_report_to_host();
    result
}

fn main() {
    let _ = kernal_api_run();
}
