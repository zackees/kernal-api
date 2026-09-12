// Generated private Wasmtime 45 glue for scalar Core Wasm ABI `kernal-api:v1`.
// Host trait and invocation helpers use semantic Rust scalar types.

fn i32_from_i32(value: i32) -> wasmtime::Result<i32> {
    Ok(value)
}
fn i32_to_i32(value: i32) -> i32 {
    value
}
fn u32_from_i32(value: i32) -> wasmtime::Result<u32> {
    Ok(value as u32)
}
fn u32_to_i32(value: u32) -> i32 {
    value as i32
}
fn u64_from_i64(value: i64) -> wasmtime::Result<u64> {
    Ok(value as u64)
}
fn u64_to_i64(value: u64) -> i64 {
    value as i64
}

pub(crate) trait KernalApiV1Imports {
    fn kernel_yield(&mut self) -> wasmtime::Result<()>;
    fn operation_cancel(&mut self, operation: u64) -> wasmtime::Result<i32>;
    fn operation_poll(&mut self, operation: u64) -> wasmtime::Result<u64>;
    fn operation_submit(&mut self, kind: u32, arg0: u64, arg1: u64) -> wasmtime::Result<u64>;
    fn operation_yield(&mut self, operation: u64) -> wasmtime::Result<i32>;
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
    linker.func_wrap(
        "kernal-api:v1",
        "operation_cancel",
        |mut caller: wasmtime::Caller<'_, T>,
         operation: i64|
         -> wasmtime::Result<i32> {
            let operation = u64_from_i64(operation)?;
            Ok(i32_to_i32(caller.data_mut().operation_cancel(operation)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "operation_poll",
        |mut caller: wasmtime::Caller<'_, T>,
         operation: i64|
         -> wasmtime::Result<i64> {
            let operation = u64_from_i64(operation)?;
            Ok(u64_to_i64(caller.data_mut().operation_poll(operation)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "operation_submit",
        |mut caller: wasmtime::Caller<'_, T>,
         kind: i32,
         arg0: i64,
         arg1: i64|
         -> wasmtime::Result<i64> {
            let kind = u32_from_i32(kind)?;
            let arg0 = u64_from_i64(arg0)?;
            let arg1 = u64_from_i64(arg1)?;
            Ok(u64_to_i64(caller.data_mut().operation_submit(kind, arg0, arg1)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "operation_yield",
        |mut caller: wasmtime::Caller<'_, T>,
         operation: i64|
         -> wasmtime::Result<i32> {
            let operation = u64_from_i64(operation)?;
            Ok(i32_to_i32(caller.data_mut().operation_yield(operation)?))
        },
    )?;
    Ok(())
}


