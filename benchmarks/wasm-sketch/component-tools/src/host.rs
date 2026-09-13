//! Private generated-world host adapter for the Component Model experiment.
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
    table: ResourceTable,
    granted: bool,
    live: Arc<AtomicUsize>,
    live_blobs: Arc<AtomicUsize>,
    produced: Arc<AtomicUsize>,
    force_trap: bool,
    limits: wasmtime::StoreLimits,
}
struct Host;
impl HasData for Host {
    type Data<'a> = &'a mut State;
}
impl bindings::kernal::probe::blobs::HostBlob for State {}
impl bindings::kernal::probe::blobs::Host for State {}

impl bindings::kernal::probe::blobs::HostWithStore for Host {
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
            let force_trap = access.get().force_trap;
            live.fetch_add(1, Ordering::SeqCst);
            StreamReader::new(
                &mut access,
                Producer {
                    remaining: 64 * 1024 * 1024,
                    live,
                    produced,
                    force_trap,
                },
            )
        })
    }
}

struct Producer {
    remaining: usize,
    live: Arc<AtomicUsize>,
    produced: Arc<AtomicUsize>,
    force_trap: bool,
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
            return Poll::Ready(Ok(StreamResult::Cancelled));
        }
        if self.remaining == 0 {
            return Poll::Ready(Ok(StreamResult::Dropped));
        }
        if self.force_trap && self.remaining < 64 * 1024 * 1024 {
            return Poll::Ready(Err(wasmtime::format_err!(
                "intentional probe producer trap"
            )));
        }
        let count = destination
            .remaining(&mut store)
            .unwrap_or(64 * 1024)
            .min(64 * 1024)
            .min(self.remaining);
        destination.set_buffer(vec![0x5a; count].into());
        self.remaining -= count;
        self.produced.fetch_add(count, Ordering::SeqCst);
        Poll::Ready(Ok(StreamResult::Completed))
    }
}

pub(super) fn execute(bytes: &[u8]) -> wasmtime::Result<()> {
    execute_case(bytes, false)?;
    execute_case(bytes, true)?;
    println!("executed component: 64 MiB through <=64 KiB host chunks; normal and forced-trap teardown returned blob/producer counts to zero");
    Ok(())
}

fn execute_case(bytes: &[u8], force_trap: bool) -> wasmtime::Result<()> {
    let mut config = wasmtime::Config::new();
    config
        .wasm_component_model_async(true)
        .concurrency_support(true)
        .consume_fuel(true);
    let engine = wasmtime::Engine::new(&config)?;
    let component = wasmtime::component::Component::new(&engine, bytes)?;
    let mut linker = Linker::new(&engine);
    bindings::Sketch::add_to_linker::<State, Host>(&mut linker, |state| state)?;
    let mut store = Store::new(
        &engine,
        State {
            force_trap,
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(8 * 1024 * 1024)
                .build(),
            ..State::default()
        },
    );
    store.limiter(|state| &mut state.limits);
    store.set_fuel(100_000_000)?;
    let live = store.data().live.clone();
    let live_blobs = store.data().live_blobs.clone();
    let produced = store.data().produced.clone();
    let runtime = kernal_api::async_engine::RuntimeBuilder::current_thread()
        .enable_all()
        .build()?;
    let result = runtime.run(kernal_api::async_engine::timeout(
        std::time::Duration::from_secs(30),
        async {
            let sketch =
                bindings::Sketch::instantiate_async(&mut store, &component, &linker).await?;
            let count = store
                .run_concurrent(async |accessor| sketch.call_run(accessor).await)
                .await??;
            wasmtime::ensure!(
                count == Ok(64 * 1024 * 1024),
                "guest did not count exactly 64 MiB"
            );
            wasmtime::ensure!(store.data().table.is_empty(), "guest blob resource leaked");
            wasmtime::ensure!(
                store.data().produced.load(Ordering::SeqCst) == 64 * 1024 * 1024,
                "host did not supply exactly 64 MiB"
            );
            Ok::<_, wasmtime::Error>(())
        },
    ));
    drop(store);
    wasmtime::ensure!(
        live.load(Ordering::SeqCst) == 0,
        "host stream producer leaked"
    );
    wasmtime::ensure!(
        live_blobs.load(Ordering::SeqCst) == 0,
        "host blob leaked after teardown"
    );
    let outcome = result?;
    if force_trap {
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
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires the separately built async component in KERNAL_COMPONENT_PROBE"]
    fn actual_component_stream_and_trap_cleanup() {
        let path = std::env::var_os("KERNAL_COMPONENT_PROBE")
            .expect("set KERNAL_COMPONENT_PROBE to the encoded async component");
        let bytes = std::fs::read(path).expect("read the actual component fixture");
        super::execute(&bytes).expect("64 MiB stream and forced-trap cleanup");
    }
}
