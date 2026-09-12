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

/// Drive the generated operation lifecycle. `operation_yield` is an async
/// Wasmtime host import: the host drops its Caller before waiting, then wakes
/// this guest only after the hub has published one terminal outcome.
fn complete_operation(
    operation: kernal_api_v1_bindings::OperationFuture,
) -> Result<u64, kernal_api_v1_bindings::OperationError> {
    assert!(
        operation.poll()?.is_none(),
        "synthetic operation begins pending"
    );
    operation.yield_now()?;
    Ok(operation
        .poll()?
        .expect("woken operation must have one terminal result"))
}

/// Explicit result marker for public artifact inspection.
#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    let blob = kernal_api_v1_bindings::BlobHandle::from_create_payload(
        complete_operation(
            kernal_api_v1_bindings::BlobHandle::create().expect("submit blob create"),
        )
        .expect("generated blob create"),
    );
    const CHUNK_BYTES: usize = 64 * 1024;
    assert!(blob.read_chunk(0).is_err(), "zero-length read is rejected");
    assert!(
        blob.read_chunk(64 * 1024 + 1).is_err(),
        "oversized read is rejected"
    );
    const CHUNKS: usize = 1024;
    let mut sent = [0_u8; CHUNK_BYTES];
    let mut received = [0_u8; CHUNK_BYTES];
    // The default per-blob capacity is 1 MiB. Pause consumption and prove
    // the seventeenth write cannot finish, even after a scheduler turn.
    for _ in 0..16 {
        assert!(blob.write_chunk(&sent).unwrap().poll().unwrap().is_some());
    }
    let blocked = blob.write_chunk(&sent).expect("capacity-awaited write");
    assert!(blocked.poll().unwrap().is_none());
    complete_operation(kernal_api_v1_bindings::synthetic_yield().unwrap()).unwrap();
    assert!(
        blocked.poll().unwrap().is_none(),
        "producer must wait for consumption"
    );
    let first = blob.read_chunk(CHUNK_BYTES as u32).unwrap();
    assert_eq!(first.poll_into(&mut received).unwrap(), Some(CHUNK_BYTES));
    assert!(
        blocked.poll().unwrap().is_some(),
        "bounded pull releases write capacity"
    );
    for _ in 0..16 {
        let read = blob.read_chunk(CHUNK_BYTES as u32).unwrap();
        assert_eq!(read.poll_into(&mut received).unwrap(), Some(CHUNK_BYTES));
        assert_eq!(received, sent);
    }
    for chunk in 0..CHUNKS {
        for (index, byte) in sent.iter_mut().enumerate() {
            *byte = (index as u8).wrapping_add(chunk as u8);
        }
        let write = blob.write_chunk(&sent).expect("submit bounded blob write");
        if write.poll().expect("poll blob write").is_none() {
            write.yield_now().expect("yield pending blob write");
            assert!(write.poll().expect("poll completed blob write").is_some());
        }
        let read = blob
            .read_chunk(CHUNK_BYTES as u32)
            .expect("submit bounded blob read");
        let count = loop {
            if let Some(count) = read.poll_into(&mut received).expect("collect blob read") {
                break count;
            }
            read.yield_now().expect("yield pending blob read");
        };
        assert_eq!(count, CHUNK_BYTES);
        assert_eq!(received, sent);
    }
    let eof = blob.read_chunk(1).expect("submit EOF observation");
    assert!(eof
        .poll_into(&mut received[..1])
        .expect("empty live blob")
        .is_none());
    let seal = blob.seal().expect("submit EOF");
    assert!(seal.poll().expect("completed seal").is_some());
    assert_eq!(
        eof.poll_into(&mut received[..1]).expect("sealed EOF"),
        Some(0)
    );
    complete_operation(blob.close().expect("submit blob close")).expect("generated blob close");
    if let Some(output) =
        kernal_api_v1_bindings::OutputFile::granted().expect("initial output grant")
    {
        let image = kernal_api_v1_bindings::BlobHandle::from_create_payload(
            complete_operation(kernal_api_v1_bindings::BlobHandle::create().unwrap()).unwrap(),
        );
        let write = image.write_chunk(b"guest exact output").unwrap();
        assert!(write.poll().unwrap().is_some());
        assert!(image.seal().unwrap().poll().unwrap().is_some());
        let commit = output.write_blob(&image).expect("submit exact output");
        while commit.poll().expect("commit result").is_none() {
            commit.yield_now().expect("wait for output commit");
        }
    }
    let counter = Arc::new(AtomicU32::new(0));
    let totals = Arc::new(Mutex::new(0_u32));
    // Use an explicit deterministic hasher: the closed threaded P1 surface
    // intentionally owns no ambient `random_get` authority.
    let map = Arc::new(
        DashMap::<u32, u32, BuildHasherDefault<DefaultHasher>>::with_hasher(
            BuildHasherDefault::default(),
        ),
    );
    let (tx, rx) = mpsc::channel();
    let resource = kernal_api_v1_bindings::SyntheticResource::from_create_payload(
        complete_operation(
            kernal_api_v1_bindings::SyntheticResource::create(true)
                .expect("submit generated resource create"),
        )
        .expect("generated resource create"),
    );
    let mut workers = Vec::new();
    for key in 0..2_u32 {
        let counter = Arc::clone(&counter);
        let totals = Arc::clone(&totals);
        let map = Arc::clone(&map);
        let tx = tx.clone();
        let resource = resource;
        workers.push(std::thread::spawn(move || {
            // Each native child crosses the kernel boundary too, proving the
            // supplied runtime handle is observed in every guest Store.
            kernal_api_v1_bindings::imports::kernel_yield().expect("generated kernel yield ABI");
            complete_operation(
                resource
                    .use_()
                    .expect("submit generated shared resource use"),
            )
            .expect("generated shared resource use");
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
    complete_operation(resource.close().expect("submit generated resource close"))
        .expect("generated resource close");
    kernal_api_v1_bindings::imports::kernel_yield().expect("generated kernel yield ABI");
    let result =
        joined + counter.load(Ordering::SeqCst) + mutex_total + channel_total + map_sum + tls_total;
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
