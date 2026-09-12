// Generated private Wasmtime 45 glue for scalar Core Wasm ABI `kernal-api:v1`.
// Host trait and invocation helpers use semantic Rust scalar types.

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
    fn abi_version(&mut self) -> wasmtime::Result<u32>;
    fn cancel(&mut self, request: u64) -> wasmtime::Result<u32>;
    fn capability_bits(&mut self) -> wasmtime::Result<u64>;
    fn completion_word(&mut self, request: u64, field: u32) -> wasmtime::Result<u64>;
    fn poll(&mut self, request: u64) -> wasmtime::Result<u32>;
    fn release(&mut self, request: u64) -> wasmtime::Result<u32>;
    fn submit(&mut self, operation: u32, argument0: u64, argument1: u64, argument2: u64) -> wasmtime::Result<u64>;
    fn yield_now(&mut self) -> wasmtime::Result<u32>;
}

pub(crate) fn link_kernal_api_v1<T>(linker: &mut wasmtime::Linker<T>) -> wasmtime::Result<()>
where
    T: KernalApiV1Imports + Send + 'static,
{
    linker.func_wrap(
        "kernal-api:v1",
        "abi_version",
        |mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<i32> {
            Ok(u32_to_i32(caller.data_mut().abi_version()?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "cancel",
        |mut caller: wasmtime::Caller<'_, T>,
         request: i64|
         -> wasmtime::Result<i32> {
            let request = u64_from_i64(request)?;
            Ok(u32_to_i32(caller.data_mut().cancel(request)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "capability_bits",
        |mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<i64> {
            Ok(u64_to_i64(caller.data_mut().capability_bits()?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "completion_word",
        |mut caller: wasmtime::Caller<'_, T>,
         request: i64,
         field: i32|
         -> wasmtime::Result<i64> {
            let request = u64_from_i64(request)?;
            let field = u32_from_i32(field)?;
            Ok(u64_to_i64(caller.data_mut().completion_word(request, field)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "poll",
        |mut caller: wasmtime::Caller<'_, T>,
         request: i64|
         -> wasmtime::Result<i32> {
            let request = u64_from_i64(request)?;
            Ok(u32_to_i32(caller.data_mut().poll(request)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "release",
        |mut caller: wasmtime::Caller<'_, T>,
         request: i64|
         -> wasmtime::Result<i32> {
            let request = u64_from_i64(request)?;
            Ok(u32_to_i32(caller.data_mut().release(request)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "submit",
        |mut caller: wasmtime::Caller<'_, T>,
         operation: i32,
         argument0: i64,
         argument1: i64,
         argument2: i64|
         -> wasmtime::Result<i64> {
            let operation = u32_from_i32(operation)?;
            let argument0 = u64_from_i64(argument0)?;
            let argument1 = u64_from_i64(argument1)?;
            let argument2 = u64_from_i64(argument2)?;
            Ok(u64_to_i64(caller.data_mut().submit(operation, argument0, argument1, argument2)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "yield_now",
        |mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<i32> {
            Ok(u32_to_i32(caller.data_mut().yield_now()?))
        },
    )?;
    Ok(())
}
