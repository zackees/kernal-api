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
        #[link_name = "kernel_yield"]
        pub(super) fn __kernal_api_v1_import_kernel_yield() -> ();
    }
}

pub mod imports {
    use super::{raw_imports, AbiError};

    pub fn kernel_yield() -> Result<(), AbiError> {
        unsafe { raw_imports::__kernal_api_v1_import_kernel_yield() };
        Ok(())
    }
}

pub struct KernalApiV1Exports {

}

static EXPORTS: OnceLock<KernalApiV1Exports> = OnceLock::new();

pub fn install_exports(exports: KernalApiV1Exports) -> Result<(), ExportInstallError> { EXPORTS.set(exports).map_err(|_| ExportInstallError::AlreadyInstalled) }

fn installed_exports() -> &'static KernalApiV1Exports { EXPORTS.get().expect("install KernalApiV1Exports before invoking guest exports") }
fn require_abi<T>(value: Result<T, AbiError>) -> T { value.expect("host passed an invalid scalar Core Wasm ABI value") }


