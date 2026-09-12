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
        #[link_name = "operation_cancel"]
        pub(super) fn __kernal_api_v1_import_operation_cancel(operation: i64) -> i32;
        #[link_name = "operation_poll"]
        pub(super) fn __kernal_api_v1_import_operation_poll(operation: i64) -> i64;
        #[link_name = "operation_submit"]
        pub(super) fn __kernal_api_v1_import_operation_submit(kind: i32, arg0: i64, arg1: i64) -> i64;
        #[link_name = "operation_yield"]
        pub(super) fn __kernal_api_v1_import_operation_yield(operation: i64) -> i32;
    }
}

pub mod imports {
    use super::{raw_imports, AbiError, i32_from_i32, u32_to_i32, u64_from_i64, u64_to_i64};

    pub fn kernel_yield() -> Result<(), AbiError> {
        unsafe { raw_imports::__kernal_api_v1_import_kernel_yield() };
        Ok(())
    }

    pub fn operation_cancel(operation: u64) -> Result<i32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_operation_cancel(u64_to_i64(operation)) };
        i32_from_i32(raw)
    }

    pub fn operation_poll(operation: u64) -> Result<u64, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_operation_poll(u64_to_i64(operation)) };
        u64_from_i64(raw)
    }

    pub fn operation_submit(kind: u32, arg0: u64, arg1: u64) -> Result<u64, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_operation_submit(u32_to_i32(kind), u64_to_i64(arg0), u64_to_i64(arg1)) };
        u64_from_i64(raw)
    }

    pub fn operation_yield(operation: u64) -> Result<i32, AbiError> {
        let raw = unsafe { raw_imports::__kernal_api_v1_import_operation_yield(u64_to_i64(operation)) };
        i32_from_i32(raw)
    }
}

pub struct KernalApiV1Exports {

}

static EXPORTS: OnceLock<KernalApiV1Exports> = OnceLock::new();

pub fn install_exports(exports: KernalApiV1Exports) -> Result<(), ExportInstallError> { EXPORTS.set(exports).map_err(|_| ExportInstallError::AlreadyInstalled) }

fn installed_exports() -> &'static KernalApiV1Exports { EXPORTS.get().expect("install KernalApiV1Exports before invoking guest exports") }
fn require_abi<T>(value: Result<T, AbiError>) -> T { value.expect("host passed an invalid scalar Core Wasm ABI value") }




#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationError { Rejected, Cancelled, Closed, Failed }
pub struct OperationFuture { operation: u64 }
impl OperationFuture {
    fn submit(kind: u32, arg0: u64, arg1: u64) -> Result<Self, OperationError> {
        let operation = imports::operation_submit(kind, arg0, arg1).map_err(|_| OperationError::Failed)?;
        if operation == 0 { Err(OperationError::Rejected) } else { Ok(Self { operation }) }
    }
    pub fn poll(&self) -> Result<Option<u64>, OperationError> {
        let packed = imports::operation_poll(self.operation).map_err(|_| OperationError::Failed)?;
        match packed as u8 { 0 => Ok(None), 1 => Ok(Some(packed >> 8)), 2 => Err(OperationError::Cancelled), 6 => Err(OperationError::Closed), _ => Err(OperationError::Failed) }
    }
    pub fn yield_now(&self) -> Result<(), OperationError> { if imports::operation_yield(self.operation).map_err(|_| OperationError::Failed)? != 0 { Ok(()) } else { Err(OperationError::Failed) } }
    pub fn cancel(&self) { let _ = imports::operation_cancel(self.operation); }
}
#[derive(Clone, Copy)]
pub struct SyntheticResource { token: u64 }
impl SyntheticResource {
    pub fn create(shareable: bool) -> Result<OperationFuture, OperationError> { OperationFuture::submit(2, u64::from(shareable), 1) }
    pub fn from_create_payload(token: u64) -> Self { Self { token } }
    pub fn use_(&self) -> Result<OperationFuture, OperationError> { OperationFuture::submit(3, self.token, 1) }
    pub fn close(&self) -> Result<OperationFuture, OperationError> { OperationFuture::submit(4, self.token, 0) }
}
pub fn synthetic_yield() -> Result<OperationFuture, OperationError> { OperationFuture::submit(1, 0, 0) }

/// Opaque host-owned bulk resource. No buffer or native path is carried here.
pub struct BlobHandle { token: u64 }
impl BlobHandle {
    pub fn create() -> Result<OperationFuture, OperationError> { OperationFuture::submit(5, 0, 0) }
    pub fn from_create_payload(token: u64) -> Self { Self { token } }
    pub fn close(&self) -> Result<OperationFuture, OperationError> { OperationFuture::submit(4, self.token, 0) }
    /// The host copies this bounded slice before returning the future.
    /// No guest pointer or borrow is retained while waiting for capacity.
    pub fn write_chunk(&self, bytes: &[u8]) -> Result<OperationFuture, OperationError> {
        let length = u32::try_from(bytes.len()).map_err(|_| OperationError::Rejected)?;
        let pointer = u32::try_from(bytes.as_ptr() as usize).map_err(|_| OperationError::Rejected)?;
        OperationFuture::submit(6, self.token, (u64::from(length) << 32) | u64::from(pointer))
    }
}
