// Generated private Wasmtime 45 glue for scalar Core Wasm ABI `kernal-api:v1`.
// Host trait and invocation helpers use semantic Rust scalar types.

fn bool_from_i32(value: i32) -> wasmtime::Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        value => Err(wasmtime::Error::msg(format!(
            "invalid bool ABI value: {value}"
        ))),
    }
}
fn bool_to_i32(value: bool) -> i32 {
    i32::from(value)
}
fn i16_from_i32(value: i32) -> wasmtime::Result<i16> {
    value
        .try_into()
        .map_err(|_| wasmtime::Error::msg(format!("invalid i16 ABI value: {value}")))
}
fn i16_to_i32(value: i16) -> i32 {
    value as i32
}
fn u8_from_i32(value: i32) -> wasmtime::Result<u8> {
    value
        .try_into()
        .map_err(|_| wasmtime::Error::msg(format!("invalid u8 ABI value: {value}")))
}
fn u8_to_i32(value: u8) -> i32 {
    value as i32
}
fn u16_from_i32(value: i32) -> wasmtime::Result<u16> {
    value
        .try_into()
        .map_err(|_| wasmtime::Error::msg(format!("invalid u16 ABI value: {value}")))
}
fn u16_to_i32(value: u16) -> i32 {
    value as i32
}
fn u64_from_i64(value: i64) -> wasmtime::Result<u64> {
    Ok(value as u64)
}
fn u64_to_i64(value: u64) -> i64 {
    value as i64
}
fn f32_from_f32(value: f32) -> wasmtime::Result<f32> {
    Ok(value)
}
fn f32_to_f32(value: f32) -> f32 {
    value
}

pub(crate) trait KernalApiV1Imports {
    fn checked(&mut self, flag: bool, tiny: u8, total: u64) -> wasmtime::Result<bool>;
    fn reset(&mut self) -> wasmtime::Result<()>;
}

pub(crate) fn link_kernal_api_v1<T>(linker: &mut wasmtime::Linker<T>) -> wasmtime::Result<()>
where
    T: KernalApiV1Imports + Send + 'static,
{
    linker.func_wrap(
        "kernal-api:v1",
        "checked",
        |mut caller: wasmtime::Caller<'_, T>,
         flag: i32,
         tiny: i32,
         total: i64|
         -> wasmtime::Result<i32> {
            let flag = bool_from_i32(flag)?;
            let tiny = u8_from_i32(tiny)?;
            let total = u64_from_i64(total)?;
            Ok(bool_to_i32(caller.data_mut().checked(flag, tiny, total)?))
        },
    )?;
    linker.func_wrap(
        "kernal-api:v1",
        "reset",
        |mut caller: wasmtime::Caller<'_, T>| -> wasmtime::Result<()> {
            caller.data_mut().reset()?;
            Ok(())
        },
    )?;
    Ok(())
}

pub(crate) fn invoke_guest_measure<T>(
    store: &mut wasmtime::Store<T>,
    instance: &wasmtime::Instance,
    value: i16,
    ratio: f32,
) -> wasmtime::Result<u16> {
    let function = instance.get_typed_func::<(i32, f32), i32>(&mut *store, "guest_measure")?;
    let raw = function.call(&mut *store, (i16_to_i32(value), f32_to_f32(ratio)))?;
    u16_from_i32(raw)
}

pub(crate) fn invoke_guest_unit<T>(
    store: &mut wasmtime::Store<T>,
    instance: &wasmtime::Instance,
) -> wasmtime::Result<()> {
    let function = instance.get_typed_func::<(), ()>(&mut *store, "guest_unit")?;
    let raw = function.call(&mut *store, ())?;
    Ok(())
}
