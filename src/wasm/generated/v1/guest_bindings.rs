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
pub enum OperationError { Rejected, Cancelled, Closed, Failed, TimedOut }
pub struct OperationFuture { operation: u64 }
impl OperationFuture {
    fn submit(kind: u32, arg0: u64, arg1: u64) -> Result<Self, OperationError> {
        let operation = imports::operation_submit(kind, arg0, arg1).map_err(|_| OperationError::Failed)?;
        if operation == 0 { Err(OperationError::Rejected) } else { Ok(Self { operation }) }
    }
    pub fn poll(&self) -> Result<Option<u64>, OperationError> {
        let packed = imports::operation_poll(self.operation).map_err(|_| OperationError::Failed)?;
        match packed as u8 { 0 => Ok(None), 1 => Ok(Some(packed >> 8)), 2 => Err(OperationError::Cancelled), 3 => Err(OperationError::TimedOut), 6 => Err(OperationError::Closed), 7 => Err(OperationError::Rejected), _ => Err(OperationError::Failed) }
    }
    pub fn yield_now(&self) -> Result<(), OperationError> { if imports::operation_yield(self.operation).map_err(|_| OperationError::Failed)? == 1 { Ok(()) } else { Err(OperationError::Failed) } }
    pub fn cancel(&self) { let _ = imports::operation_cancel(self.operation); }
    /// Discard a blob transfer and its result without allocating another slot.
    /// Non-transfer operation tokens are rejected by the host.
    pub fn abandon_transfer(&self) { let _ = imports::operation_submit(18, self.operation, 0); }
    /// The async host import parks this Wasm stack until the operation wakes.
    /// No native thread blocks and no guest-side scheduler is constructed.
    pub async fn wait(self) -> Result<u64, OperationError> {
        loop {
            if let Some(payload) = self.poll()? { return Ok(payload); }
            self.yield_now()?;
        }
    }
}

/// Drive a command composed exclusively of generated kernel futures.
/// Suspension happens inside the generated async host import, not through a
/// second guest executor. A foreign future returning Pending is unsupported.
pub fn run<T>(future: impl std::future::Future<Output = Result<T, OperationError>>) -> Result<T, OperationError> {
    let mut future = std::pin::pin!(future);
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut context) {
        std::task::Poll::Ready(result) => result,
        std::task::Poll::Pending => Err(OperationError::Failed),
    }
}

/// One pre-authorized native URL, never a guest URL string or network grant.
pub struct WebviewUrl { token: u64 }
/// Opaque native viewport authority owned by this logical sketch.
pub struct Webview { token: u64 }
impl WebviewUrl {
    pub fn granted() -> Result<Option<Self>, OperationError> {
        let token = imports::operation_submit(13, 0, 0).map_err(|_| OperationError::Failed)?;
        Ok(if token == 0 { None } else { Some(Self { token }) })
    }
    pub async fn open(&self) -> Result<Webview, OperationError> {
        let token = OperationFuture::submit(14, self.token, 0)?.wait().await?;
        if token == 0 { return Err(OperationError::Failed); }
        Ok(Webview { token })
    }
}
impl Webview {
    pub async fn wait_until_loaded(&self) -> Result<(), OperationError> {
        OperationFuture::submit(15, self.token, 0)?.wait().await?;
        Ok(())
    }
    pub async fn capture_visible_png(&self) -> Result<BlobHandle, OperationError> {
        let token = OperationFuture::submit(16, self.token, 0)?.wait().await?;
        if token == 0 { return Err(OperationError::Failed); }
        Ok(BlobHandle { token })
    }
    pub async fn close(self) -> Result<(), OperationError> {
        OperationFuture::submit(17, self.token, 0)?.wait().await?;
        Ok(())
    }
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

/// Sleep on the embedding kernel's monotonic timer. The guest imports no
/// clock and creates no runtime; yield parks this Wasm execution on the host.
pub fn clock_sleep(milliseconds: u32) -> Result<OperationFuture, OperationError> {
    OperationFuture::submit(12, u64::from(milliseconds), 0)
}

/// Opaque host-owned bulk resource. No buffer or native path is carried here.
pub struct BlobHandle { token: u64 }
/// Optional host-owned encrypted input. No source path or key crosses the ABI.
pub struct EncryptedArchive { token: u64 }
impl EncryptedArchive {
    pub fn granted() -> Result<Option<Self>, OperationError> {
        let token = imports::operation_submit(20, 0, 0).map_err(|_| OperationError::Failed)?;
        Ok(if token == 0 { None } else { Some(Self { token }) })
    }
    pub fn read_header(&self, destination: &mut [u8]) -> Result<usize, OperationError> {
        let length = u32::try_from(destination.len()).map_err(|_| OperationError::Rejected)?;
        let pointer = u32::try_from(destination.as_mut_ptr() as usize).map_err(|_| OperationError::Rejected)?;
        let result = imports::operation_submit(21, self.token, (u64::from(length) << 32) | u64::from(pointer)).map_err(|_| OperationError::Failed)?;
        let copied = (result >> 8) as usize;
        if result as u8 != 1 || copied > destination.len() || copied > 16 * 1024 + 12 { return Err(OperationError::Rejected); }
        Ok(copied)
    }
    pub fn abandon(&self) { let _ = imports::operation_submit(22, self.token, 0); }
}
/// One exact destination authorized by the embedding host; never a guest path.
pub struct OutputFile { token: u64 }
impl OutputFile {
    /// Discover the optional initial grant for the root Store.
    pub fn granted() -> Result<Option<Self>, OperationError> {
        let token = imports::operation_submit(11, 0, 0).map_err(|_| OperationError::Failed)?;
        Ok(if token == 0 { None } else { Some(Self { token }) })
    }
    /// Wrap the scoped token supplied by the host. Forging this value grants
    /// no authority: every commit checks the host's resource registry.
    pub fn from_granted_token(token: u64) -> Self { Self { token } }
    pub fn write_blob(&self, blob: &BlobHandle) -> Result<OperationFuture, OperationError> {
        OperationFuture::submit(10, blob.token, self.token)
    }
}
pub struct BlobReadFuture { operation: u64 }
impl BlobReadFuture {
    /// Poll and collect into a caller-owned bounded destination. No pointer
    /// is retained after this call, including when the result is pending.
    pub fn poll_into(&self, destination: &mut [u8]) -> Result<Option<usize>, OperationError> {
        let length = u32::try_from(destination.len()).map_err(|_| OperationError::Rejected)?;
        let pointer = u32::try_from(destination.as_mut_ptr() as usize).map_err(|_| OperationError::Rejected)?;
        let packed = imports::operation_submit(8, self.operation, (u64::from(length) << 32) | u64::from(pointer)).map_err(|_| OperationError::Failed)?;
        match packed as u8 {
            0 => Ok(None),
            1 => Ok(Some((packed >> 8) as usize)),
            2 => Err(OperationError::Cancelled),
            6 => Err(OperationError::Closed),
            _ => Err(OperationError::Failed),
        }
    }
    pub fn yield_now(&self) -> Result<(), OperationError> { OperationFuture { operation: self.operation }.yield_now() }
    pub fn cancel(&self) { OperationFuture { operation: self.operation }.cancel(); }
    pub fn abandon_transfer(&self) { OperationFuture { operation: self.operation }.abandon_transfer(); }
}
impl BlobHandle {
    /// Revoke this resource synchronously without allocating an operation.
    pub fn abandon(&self) { let _ = imports::operation_submit(19, self.token, 0); }
    pub fn create() -> Result<OperationFuture, OperationError> { OperationFuture::submit(5, 0, 0) }
    pub fn from_create_payload(token: u64) -> Self { Self { token } }
    pub fn read_chunk(&self, maximum_bytes: u32) -> Result<BlobReadFuture, OperationError> {
        let operation = OperationFuture::submit(7, self.token, u64::from(maximum_bytes))?;
        Ok(BlobReadFuture { operation: operation.operation })
    }
    /// Publish EOF. Call only after preceding writes have completed; pending
    /// writes are rejected when the producer seals its stream.
    pub fn seal(&self) -> Result<OperationFuture, OperationError> { OperationFuture::submit(9, self.token, 0) }
    pub fn close(&self) -> Result<OperationFuture, OperationError> { OperationFuture::submit(4, self.token, 0) }
    /// The host copies this bounded slice before returning the future.
    /// No guest pointer or borrow is retained while waiting for capacity.
    pub fn write_chunk(&self, bytes: &[u8]) -> Result<OperationFuture, OperationError> {
        let length = u32::try_from(bytes.len()).map_err(|_| OperationError::Rejected)?;
        let pointer = u32::try_from(bytes.as_ptr() as usize).map_err(|_| OperationError::Rejected)?;
        OperationFuture::submit(6, self.token, (u64::from(length) << 32) | u64::from(pointer))
    }
}
