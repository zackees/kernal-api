// Generated scalar Core Wasm guest bindings for `kernal-api:v1`.
// The public API uses semantic Rust scalar types; raw ABI values stay private.

use std::sync::OnceLock;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AbiError {
    InvalidBoolean(i32),
    OutOfRange { ty: &'static str, value: i32 },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExportInstallError {
    AlreadyInstalled,
}

fn bool_from_i32(value: i32) -> Result<bool, AbiError> { match value { 0 => Ok(false), 1 => Ok(true), value => Err(AbiError::InvalidBoolean(value)) } }
fn i8_from_i32(value: i32) -> Result<i8, AbiError> { value.try_into().map_err(|_| AbiError::OutOfRange { ty: "i8", value }) }
fn i16_from_i32(value: i32) -> Result<i16, AbiError> { value.try_into().map_err(|_| AbiError::OutOfRange { ty: "i16", value }) }
fn u8_from_i32(value: i32) -> Result<u8, AbiError> { value.try_into().map_err(|_| AbiError::OutOfRange { ty: "u8", value }) }
fn u16_from_i32(value: i32) -> Result<u16, AbiError> { value.try_into().map_err(|_| AbiError::OutOfRange { ty: "u16", value }) }

fn i32_from_i32(value: i32) -> Result<i32, AbiError> { Ok(value) }
fn u32_from_i32(value: i32) -> Result<u32, AbiError> { Ok(value as u32) }
fn i64_from_i64(value: i64) -> Result<i64, AbiError> { Ok(value) }
fn u64_from_i64(value: i64) -> Result<u64, AbiError> { Ok(value as u64) }
fn f32_from_f32(value: f32) -> Result<f32, AbiError> { Ok(value) }
fn f64_from_f64(value: f64) -> Result<f64, AbiError> { Ok(value) }
fn bool_to_i32(value: bool) -> i32 { i32::from(value) }
fn i8_to_i32(value: i8) -> i32 { value as i32 }
fn i16_to_i32(value: i16) -> i32 { value as i32 }
fn i32_to_i32(value: i32) -> i32 { value }
fn u8_to_i32(value: u8) -> i32 { value as i32 }
fn u16_to_i32(value: u16) -> i32 { value as i32 }
fn u32_to_i32(value: u32) -> i32 { value as i32 }
fn i64_to_i64(value: i64) -> i64 { value }
fn u64_to_i64(value: u64) -> i64 { value as i64 }
fn f32_to_f32(value: f32) -> f32 { value }
fn f64_to_f64(value: f64) -> f64 { value }

mod raw_imports {
    #[link(wasm_import_module = "kernal-api:v1")]
    extern "C" {
        #[link_name = "abi_version"]
        pub(super) fn __kernal_api_v1_import_abi_version() -> i32;
        #[link_name = "cancel"]
        pub(super) fn __kernal_api_v1_import_cancel(request: i64) -> i32;
        #[link_name = "capability_bits"]
        pub(super) fn __kernal_api_v1_import_capability_bits() -> i64;
        #[link_name = "completion_word"]
        pub(super) fn __kernal_api_v1_import_completion_word(request: i64, field: i32) -> i64;
        #[link_name = "poll"]
        pub(super) fn __kernal_api_v1_import_poll(request: i64) -> i32;
        #[link_name = "release"]
        pub(super) fn __kernal_api_v1_import_release(request: i64) -> i32;
        #[link_name = "submit"]
        pub(super) fn __kernal_api_v1_import_submit(operation: i32, argument0: i64, argument1: i64, argument2: i64) -> i64;
        #[link_name = "yield_now"]
        pub(super) fn __kernal_api_v1_import_yield_now() -> i32;
    }
}

pub mod imports {
    use super::*;

    pub fn abi_version() -> Result<u32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_abi_version() };
        u32_from_i32(raw)
    }

    pub fn cancel(request: u64) -> Result<u32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_cancel(u64_to_i64(request)) };
        u32_from_i32(raw)
    }

    pub fn capability_bits() -> Result<u64, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_capability_bits() };
        u64_from_i64(raw)
    }

    pub fn completion_word(request: u64, field: u32) -> Result<u64, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_completion_word(u64_to_i64(request), u32_to_i32(field)) };
        u64_from_i64(raw)
    }

    pub fn poll(request: u64) -> Result<u32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_poll(u64_to_i64(request)) };
        u32_from_i32(raw)
    }

    pub fn release(request: u64) -> Result<u32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_release(u64_to_i64(request)) };
        u32_from_i32(raw)
    }

    pub fn submit(operation: u32, argument0: u64, argument1: u64, argument2: u64) -> Result<u64, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_submit(u32_to_i32(operation), u64_to_i64(argument0), u64_to_i64(argument1), u64_to_i64(argument2)) };
        u64_from_i64(raw)
    }

    pub fn yield_now() -> Result<u32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_yield_now() };
        u32_from_i32(raw)
    }
}

pub struct KernalApiV1Exports {

}

static EXPORTS: OnceLock<KernalApiV1Exports> = OnceLock::new();

pub fn install_exports(exports: KernalApiV1Exports) -> Result<(), ExportInstallError> { EXPORTS.set(exports).map_err(|_| ExportInstallError::AlreadyInstalled) }

fn installed_exports() -> &'static KernalApiV1Exports { EXPORTS.get().expect("install KernalApiV1Exports before invoking guest exports") }
fn require_abi<T>(value: Result<T, AbiError>) -> T { value.expect("host passed an invalid scalar Core Wasm ABI value") }
