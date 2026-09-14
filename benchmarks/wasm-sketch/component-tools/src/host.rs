//! Private generated-world host adapter for the Component Model experiment.
use std::future::Future;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use std::task::{Context, Poll};
use wasmtime::component::{
    Accessor, Destination, HasData, Linker, Resource, ResourceTable, StreamProducer, StreamReader,
    StreamResult, VecBuffer,
};
use wasmtime::{Store, StoreContextMut};

mod bindings {
    wasmtime::component::bindgen!({
        path: "../component-guest/wit",
        world: "sketch",
        imports: { default: async | store | trappable },
        with: { "kernal:probe/blobs.blob": super::BlobState },
    });
}

pub struct BlobState {
    read_started: bool,
    live: Arc<AtomicUsize>,
}
impl Drop for BlobState {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}
#[derive(Default)]
struct State {
    hash: super::hash_host::State,
    table: ResourceTable,
    granted: bool,
    live: Arc<AtomicUsize>,
    live_blobs: Arc<AtomicUsize>,
    produced: Arc<AtomicUsize>,
    mode: Mode,
    pending: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    pauses: usize,
    batches: Arc<AtomicUsize>,
    peak_batch: Arc<AtomicUsize>,
    limits: wasmtime::StoreLimits,
}
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    #[default]
    Transfer,
    Trap,
    CancelPending,
    GuestCancel,
    SlowConsumer,
}
struct Host;
impl HasData for Host {
    type Data<'a> = &'a mut State;
}
impl bindings::kernal::probe::blobs::HostBlob for State {}
impl bindings::kernal::probe::blobs::Host for State {}

impl bindings::kernal::probe::blobs::HostWithStore for Host {
    async fn pause_consumer<T: Send>(accessor: &Accessor<T, Self>) -> wasmtime::Result<()> {
        let (produced, batches) = accessor.with(|mut access| {
            let state = access.get();
            wasmtime::ensure!(
                state.mode == Mode::SlowConsumer,
                "pause outside slow-consumer probe"
            );
            wasmtime::ensure!(
                state.produced.load(Ordering::SeqCst) == 128 * 1024
                    && state.batches.load(Ordering::SeqCst) == 1,
                "expected one 128 KiB batch with 64 KiB still buffered"
            );
            state.pauses += 1;
            Ok::<_, wasmtime::Error>((state.produced.clone(), state.batches.clone()))
        })?;
        kernal_api::async_engine::sleep(std::time::Duration::from_millis(20)).await;
        wasmtime::ensure!(
            produced.load(Ordering::SeqCst) == 128 * 1024 && batches.load(Ordering::SeqCst) == 1,
            "producer advanced while the consumer was paused with buffered data"
        );
        Ok(())
    }

    async fn await_pending_read<T: Send>(accessor: &Accessor<T, Self>) -> wasmtime::Result<()> {
        loop {
            let ready = accessor.with(|mut access| {
                wasmtime::ensure!(
                    access.get().mode == Mode::GuestCancel,
                    "checkpoint outside cancellation probe"
                );
                Ok::<_, wasmtime::Error>(access.get().pending.load(Ordering::SeqCst) > 0)
            })?;
            if ready {
                return Ok(());
            }
            kernal_api::async_engine::yield_now().await;
        }
    }

    async fn granted<T: Send>(
        accessor: &Accessor<T, Self>,
    ) -> wasmtime::Result<Option<Resource<BlobState>>> {
        accessor.with(|mut access| {
            let state = access.get();
            if state.granted {
                return Ok(None);
            }
            state.granted = true;
            state.live_blobs.fetch_add(1, Ordering::SeqCst);
            Ok(Some(state.table.push(BlobState {
                read_started: false,
                live: state.live_blobs.clone(),
            })?))
        })
    }
}

impl bindings::kernal::probe::blobs::HostBlobWithStore for Host {
    async fn drop<T>(
        accessor: &Accessor<T, Self>,
        resource: Resource<BlobState>,
    ) -> wasmtime::Result<()> {
        accessor.with(|mut access| {
            access.get().table.delete(resource)?;
            Ok(())
        })
    }

    async fn read<T: Send>(
        accessor: &Accessor<T, Self>,
        resource: Resource<BlobState>,
    ) -> wasmtime::Result<StreamReader<u8>> {
        accessor.with(|mut access| {
            let blob = access.get().table.get_mut(&resource)?;
            wasmtime::ensure!(!blob.read_started, "only one probe stream may be opened");
            blob.read_started = true;
            let live = access.get().live.clone();
            let produced = access.get().produced.clone();
            let mode = access.get().mode;
            let pending = access.get().pending.clone();
            let cancelled = access.get().cancelled.clone();
            let batches = access.get().batches.clone();
            let peak_batch = access.get().peak_batch.clone();
            live.fetch_add(1, Ordering::SeqCst);
            StreamReader::new(
                &mut access,
                Producer {
                    remaining: 64 * 1024 * 1024,
                    live,
                    produced,
                    mode,
                    pending,
                    cancelled,
                    batches,
                    peak_batch,
                },
            )
        })
    }
}

struct Producer {
    remaining: usize,
    live: Arc<AtomicUsize>,
    produced: Arc<AtomicUsize>,
    mode: Mode,
    pending: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
    batches: Arc<AtomicUsize>,
    peak_batch: Arc<AtomicUsize>,
}
impl Drop for Producer {
    fn drop(&mut self) {
        self.live.fetch_sub(1, Ordering::SeqCst);
    }
}
impl<T> StreamProducer<T> for Producer {
    type Item = u8;
    type Buffer = VecBuffer<u8>;
    fn poll_produce<'a>(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        mut store: StoreContextMut<'a, T>,
        mut destination: Destination<'a, u8, Self::Buffer>,
        finish: bool,
    ) -> Poll<wasmtime::Result<StreamResult>> {
        if finish {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if self.remaining == 0 {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        if self.mode == Mode::Trap && self.remaining < 64 * 1024 * 1024 {
            return Poll::Ready(Err(wasmtime::format_err!(
                "intentional probe producer trap"
            )));
        }
        if matches!(self.mode, Mode::CancelPending | Mode::GuestCancel)
            && self.remaining < 64 * 1024 * 1024
        {
            // Deliberately never ready: the observer cancels the enclosing host
            // call only after this poll actually returns Pending.
            self.pending.fetch_add(1, Ordering::SeqCst);
            return Poll::Pending;
        }
        let count = if self.mode == Mode::SlowConsumer {
            (128 * 1024).min(self.remaining)
        } else {
            destination
                .remaining(&mut store)
                .unwrap_or(64 * 1024)
                .min(64 * 1024)
                .min(self.remaining)
        };
        destination.set_buffer(vec![0x5a; count].into());
        self.remaining -= count;
        self.produced.fetch_add(count, Ordering::SeqCst);
        self.batches.fetch_add(1, Ordering::SeqCst);
        self.peak_batch.fetch_max(count, Ordering::SeqCst);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

pub(super) fn execute(bytes: &[u8]) -> wasmtime::Result<()> {
    execute_case(bytes, Mode::Transfer)?;
    execute_case(bytes, Mode::Trap)?;
    execute_case(bytes, Mode::CancelPending)?;
    execute_case(bytes, Mode::GuestCancel)?;
    execute_case(bytes, Mode::SlowConsumer)?;
    println!("executed component: shared public hash policy, failed-export hash cleanup, canonical oversize rejection, bounded transfer, trap cleanup, pending-call teardown, guest read cancellation with instance reuse, and slow-consumer backpressure passed");
    Ok(())
}

fn execute_case(bytes: &[u8], mode: Mode) -> wasmtime::Result<()> {
    let mut config = wasmtime::Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true)
        .consume_fuel(true);
    let engine = wasmtime::Engine::new(&config)?;
    let component = wasmtime::component::Component::new(&engine, bytes)?;
    let mut linker = Linker::new(&engine);
    bindings::Sketch::add_to_linker::<State, Host>(&mut linker, |state| state)?;
    super::hash_host::add_to_linker(&mut linker, |state: &mut State| &mut state.hash)?;
    let mut store = Store::new(
        &engine,
        State {
            mode,
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(8 * 1024 * 1024)
                .build(),
            ..State::default()
        },
    );
    store.limiter(|state| &mut state.limits);
    store.set_fuel(500_000_000)?;
    let live_hashes = store.data().hash.live.clone();
    let live = store.data().live.clone();
    let live_blobs = store.data().live_blobs.clone();
    let produced = store.data().produced.clone();
    let pending = store.data().pending.clone();
    let runtime = kernal_api::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()?;
    let result = runtime.run(kernal_api::async_engine::timeout(
        std::time::Duration::from_secs(30),
        async {
            let sketch =
                bindings::Sketch::instantiate_async(&mut store, &component, &linker).await?;
            if mode == Mode::Transfer {
                let result = store
                    .run_concurrent(async |accessor| sketch.call_hash_proof(accessor).await)
                    .await??;
                wasmtime::ensure!(result.is_ok(), "shared public hash policy failed");
                wasmtime::ensure!(
                    live_hashes.load(Ordering::SeqCst) == 0,
                    "hash resources leaked after export"
                );
                let failed = store
                    .run_concurrent(async |accessor| sketch.call_hash_fail(accessor).await)
                    .await??;
                wasmtime::ensure!(failed.is_err(), "expected intentional hash export failure");
                wasmtime::ensure!(
                    live_hashes.load(Ordering::SeqCst) == 0,
                    "failed export leaked hash"
                );
                let digest = store
                    .run_concurrent(async |accessor| {
                        sketch.call_hash_reject_overflow(accessor).await
                    })
                    .await??;
                wasmtime::ensure!(
                    digest
                        == Ok(kernal_api::hash::Blake3Hasher::new()
                            .finalize()
                            .as_bytes()
                            .to_vec()),
                    "oversized canonical update mutated hash"
                );
                wasmtime::ensure!(
                    live_hashes.load(Ordering::SeqCst) == 0,
                    "overflow rejection leaked hash"
                );
            }
            if mode == Mode::GuestCancel {
                let count = store
                    .run_concurrent(async |accessor| sketch.call_cancel_read(accessor).await)
                    .await??;
                wasmtime::ensure!(
                    count == Ok(64 * 1024),
                    "guest did not return after cancelling its second read"
                );
                wasmtime::ensure!(
                    store.data().pending.load(Ordering::SeqCst) > 0,
                    "guest cancellation never reached a pending producer"
                );
                wasmtime::ensure!(
                    store.data().cancelled.load(Ordering::SeqCst) > 0,
                    "producer did not receive cancellation"
                );
                wasmtime::ensure!(
                    store.data().table.is_empty()
                        && live.load(Ordering::SeqCst) == 0
                        && live_blobs.load(Ordering::SeqCst) == 0,
                    "guest cancellation left resources alive before store teardown"
                );
                wasmtime::ensure!(
                    produced.load(Ordering::SeqCst) == 64 * 1024,
                    "guest cancellation produced extra bytes"
                );
                // Explicitly grant a new blob; reuse the same Store and Sketch.
                store.data_mut().mode = Mode::Transfer;
                store.data_mut().granted = false;
                produced.store(0, Ordering::SeqCst);
            }
            if mode == Mode::CancelPending {
                let mut call = Box::pin(
                    store.run_concurrent(async |accessor| sketch.call_run(accessor).await),
                );
                std::future::poll_fn(|cx| {
                    let status = call.as_mut().poll(cx);
                    if status.is_ready() {
                        return Poll::Ready(Err(wasmtime::format_err!(
                            "call completed before a pending read"
                        )));
                    }
                    if pending.load(Ordering::SeqCst) != 0 {
                        return Poll::Ready(Ok(()));
                    }
                    Poll::Pending
                })
                .await?;
                drop(call);
                return Ok(());
            }
            let count = if mode == Mode::SlowConsumer {
                store
                    .run_concurrent(async |accessor| sketch.call_slow_consumer(accessor).await)
                    .await??
            } else {
                store
                    .run_concurrent(async |accessor| sketch.call_run(accessor).await)
                    .await??
            };
            wasmtime::ensure!(
                count == Ok(64 * 1024 * 1024),
                "guest did not count exactly 64 MiB"
            );
            wasmtime::ensure!(store.data().table.is_empty(), "guest blob resource leaked");
            if mode == Mode::SlowConsumer {
                wasmtime::ensure!(
                    store.data().pauses == 1,
                    "consumer never paused at buffered capacity"
                );
                wasmtime::ensure!(
                    store.data().peak_batch.load(Ordering::SeqCst) == 128 * 1024,
                    "unexpected producer batch bound"
                );
                wasmtime::ensure!(
                    store.data().batches.load(Ordering::SeqCst) == 512,
                    "production did not resume to exactly 64 MiB"
                );
            }
            wasmtime::ensure!(
                store.data().produced.load(Ordering::SeqCst) == 64 * 1024 * 1024,
                "host did not supply exactly 64 MiB"
            );
            Ok::<_, wasmtime::Error>(())
        },
    ));
    drop(store);
    wasmtime::ensure!(
        live_hashes.load(Ordering::SeqCst) == 0,
        "hash resources leaked after store teardown"
    );
    wasmtime::ensure!(
        live.load(Ordering::SeqCst) == 0,
        "host stream producer leaked"
    );
    wasmtime::ensure!(
        live_blobs.load(Ordering::SeqCst) == 0,
        "host blob leaked after teardown"
    );
    let outcome = result?;
    if mode == Mode::Trap {
        let error = outcome
            .err()
            .ok_or_else(|| wasmtime::format_err!("expected producer trap"))?;
        wasmtime::ensure!(
            format!("{error:#}").contains("intentional probe producer trap"),
            "unexpected failure: {error:#}"
        );
        wasmtime::ensure!(
            produced.load(Ordering::SeqCst) == 64 * 1024,
            "trap did not follow exactly one chunk"
        );
    } else {
        outcome?;
        if mode == Mode::CancelPending {
            wasmtime::ensure!(
                pending.load(Ordering::SeqCst) > 0,
                "no pending read observed"
            );
            wasmtime::ensure!(
                produced.load(Ordering::SeqCst) == 64 * 1024,
                "pending read was not after exactly one chunk"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires the freshly built shared-hash component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_shared_public_hash() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE").expect("set KERNAL_COMPONENT_PROBE");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute_case(&bytes, super::Mode::Transfer).expect(
            "shared public hash policy, failed export cleanup, and canonical oversize rejection",
        );
    }

    #[test]
    #[ignore = "requires the separately built async component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_slow_consumer_backpressure() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE").expect("set KERNAL_COMPONENT_PROBE");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute_case(&bytes, super::Mode::SlowConsumer)
            .expect("bounded fast producer pauses then resumes");
    }

    #[test]
    #[ignore = "requires the separately built async component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_guest_cancellation_and_reuse() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE").expect("set KERNAL_COMPONENT_PROBE");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute_case(&bytes, super::Mode::GuestCancel)
            .expect("guest cancellation then same-instance transfer");
    }

    #[test]
    #[ignore = "requires the separately built async component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_pending_call_cancellation() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE").expect("set KERNAL_COMPONENT_PROBE");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute_case(&bytes, super::Mode::CancelPending)
            .expect("cancel an observed pending read");
    }

    #[test]
    #[ignore = "requires the separately built async component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_stream_and_trap_cleanup() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE")
            .expect("set KERNAL_COMPONENT_PROBE to the encoded async component");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute(&bytes).expect("64 MiB stream and forced-trap cleanup");
    }
}
