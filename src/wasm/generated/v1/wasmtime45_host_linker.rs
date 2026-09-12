// Generated private Wasmtime 45 glue for scalar Core Wasm ABI `kernal-api:v1`.
// Host trait and invocation helpers use semantic Rust scalar types.



pub(crate) trait KernalApiV1Imports {
    fn kernel_yield(&mut self) -> wasmtime::Result<()>;
}

pub(crate) fn link_kernal_api_v1<T>(linker: &mut wasmtime::Linker<T>) -> wasmtime::Result<()>
where
    T: KernalApiV1Imports + Send + 'static,
{
    linker.func_wrap(
        "kernal-api:v1",
        "kernel_yield",
        |mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<()> {
            caller.data_mut().kernel_yield()?;
            Ok(())
        },
    )?;
    Ok(())
}


