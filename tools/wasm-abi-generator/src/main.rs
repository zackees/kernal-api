//! Explicit generation of the private core-Wasm contract; never a build script.
use fp_bindgen::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

const CAPABILITIES: u32 = 0;
// Revision 3 adds authentication and scoped future/archive abandonment (23-25).
// Bump when operation meaning changes, even if scalar signatures do not.
const OPERATION_PROTOCOL_REVISION: u32 = 3;
const METADATA_SECTION: &str = "kernal-api.abi";

#[test]
fn packaged_bindings_match_the_standalone_generated_fixture() {
    // Nested Cargo packages are excluded from the facade archive. Its private
    // support file must nevertheless contain exactly the same generated ABI.
    assert_eq!(
        include_bytes!("../../../src/wasm/generated/v1/guest_bindings.rs").as_slice(),
        include_bytes!("../../../src/wasm/generated/v1/guest/src/lib.rs").as_slice(),
    );
}

fp_import! {
    fn kernel_yield();
    fn operation_submit(kind: u32, arg0: u64, arg1: u64) -> u64;
    fn operation_poll(operation: u64) -> u64;
    fn operation_yield(operation: u64) -> i32;
    fn operation_cancel(operation: u64) -> i32;
}
fp_export! {}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = std::env::var_os("KERNAL_API_ABI_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../src/wasm/generated/v1")
        });
    let arguments: Vec<_> = std::env::args_os().skip(1).collect();
    if !arguments.is_empty() {
        if arguments.len() != 2 || arguments[0] != "--embed-threaded-metadata" {
            return Err("usage: wasm-abi-generator [--embed-threaded-metadata <artifact>]".into());
        }
        let manifest = fs::read_to_string(output.join("kernal-api-v1.abi.toml"))?;
        let contract = Contract::parse(&manifest)?;
        let path = Path::new(&arguments[1]);
        let original = fs::read(path)?;
        let embedded = embed_metadata(&original, contract.metadata.as_bytes())?;
        if embedded != original {
            fs::write(path, embedded)?;
        }
        return Ok(());
    }
    fp_bindgen!(BindingConfig {
        bindings_type: BindingsType::RustWasmtimeCoreWasm,
        path: output
            .to_str()
            .ok_or("generator output path is not UTF-8")?,
    });
    relocate_guest_package(&output)?;
    append_semantic_lifecycle(&output)?;
    // Cargo excludes nested packages when packaging the facade. Emit the
    // same private implementation outside the standalone fixture package so
    // the published facade never depends on an omitted source-tree file.
    fs::copy(
        output.join("guest/src/lib.rs"),
        output.join("guest_bindings.rs"),
    )?;
    make_operation_yield_async(&output)?;
    let manifest = fs::read_to_string(output.join("kernal-api-v1.abi.toml"))?;
    fs::write(
        output.join("admission_contract.rs"),
        Contract::parse(&manifest)?.render(),
    )?;
    Ok(())
}

fn make_operation_yield_async(output: &Path) -> std::io::Result<()> {
    let path = output.join("wasmtime45_host_linker.rs");
    let source = fs::read_to_string(&path)?;
    let source = source.replace(
        "fn operation_yield(&mut self, operation: u64) -> wasmtime::Result<i32>;",
        "fn operation_yield(&mut self, operation: u64) -> wasmtime::Result<std::sync::Arc<crate::async_engine::Notify>>;",
    );
    let old = r#"linker.func_wrap(
        "kernal-api:v1",
        "operation_yield",
        |mut caller: wasmtime::Caller<'_, T>,
         operation: i64|
         -> wasmtime::Result<i32> {
            let operation = u64_from_i64(operation)?;
            Ok(i32_to_i32(caller.data_mut().operation_yield(operation)?))
        },
    )?;"#;
    let new = r#"linker.func_wrap_async(
        "kernal-api:v1",
        "operation_yield",
        |mut caller: wasmtime::Caller<'_, T>, (operation,): (i64,)| {
            let waiter = caller.data_mut().operation_yield(operation as u64);
            Box::new(async move {
                match waiter { Ok(waiter) => { waiter.notified().await; 1_i32 }, Err(_) => -1_i32 }
            })
        },
    )?;"#;
    let source = source.replace(old, new);
    fs::write(path, format!("{}\n", source.trim_end()))
}

fn append_semantic_lifecycle(output: &Path) -> std::io::Result<()> {
    // This stays generator-owned: the semantic facade is derived from the
    // scalar declarations above and never introduces another guest ABI.
    const LIFECYCLE: &str = r#"

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
    pub fn authenticate(&self, nonce: &[u8; 12]) -> Result<ArchiveAuthentication, OperationError> {
        let pointer = u32::try_from(nonce.as_ptr() as usize).map_err(|_| OperationError::Rejected)?;
        Ok(ArchiveAuthentication { inner: OperationFuture::submit(23, self.token, u64::from(pointer))? })
    }
}
pub struct ArchiveAuthentication { inner: OperationFuture }
impl ArchiveAuthentication {
    pub async fn wait(self) -> Result<AuthenticatedArchive, OperationError> {
        loop {
            if let Some(token) = self.inner.poll()? {
                if token == 0 { return Err(OperationError::Failed); }
                return Ok(AuthenticatedArchive { token });
            }
            self.inner.yield_now()?;
        }
    }
}
impl Drop for ArchiveAuthentication {
    fn drop(&mut self) { let _ = imports::operation_submit(24, self.inner.operation, 0); }
}
pub struct AuthenticatedArchive { token: u64 }
impl AuthenticatedArchive {
    pub fn close(&self) -> Result<(), OperationError> {
        if imports::operation_submit(25, self.token, 0).map_err(|_| OperationError::Failed)? == 1 {
            Ok(())
        } else { Err(OperationError::Rejected) }
    }
    pub fn abandon(&self) { let _ = self.close(); }
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
"#;
    let path = output.join("guest/src/lib.rs");
    let source = fs::read_to_string(&path)?;
    // The pinned scalar generator emits its conversion helpers at crate scope
    // but its `imports` module only imports raw_imports/AbiError.  Primitive
    // argument imports therefore need these explicit lexical imports.
    let source = source.replace(
        "use super::{raw_imports, AbiError};",
        "use super::{raw_imports, AbiError, i32_from_i32, u32_to_i32, u64_from_i64, u64_to_i64};",
    );
    fs::write(&path, source)?;
    use std::io::Write as _;
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    file.write_all(LIFECYCLE.as_bytes())
}

fn relocate_guest_package(output: &Path) -> std::io::Result<()> {
    let guest = output.join("guest");
    fs::create_dir_all(guest.join("src"))?;
    // Replace only the two files owned by generation, preserving build output.
    fs::copy(output.join("Cargo.toml"), guest.join("Cargo.toml"))?;
    fs::copy(output.join("src/lib.rs"), guest.join("src/lib.rs"))?;
    // Keep the standalone generated guest check reproducible without letting
    // Cargo synthesize an untracked lockfile in the checked-in output tree.
    fs::write(
        guest.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\n# It is not intended for manual editing.\nversion = 4\n\n[[package]]\nname = \"kernal-api-v1-bindings\"\nversion = \"0.1.0\"\n",
    )?;
    fs::remove_file(output.join("Cargo.toml"))?;
    fs::remove_file(output.join("src/lib.rs"))?;
    fs::remove_dir(output.join("src"))?;
    Ok(())
}

#[derive(Debug)]
struct Contract {
    schema: String,
    schema_revision: u8,
    generator_revision: u8,
    abi_version: u8,
    namespace: String,
    import_name: String,
    metadata: String,
}
impl Contract {
    fn parse(manifest: &str) -> Result<Self, String> {
        let value: toml::Value = manifest
            .parse()
            .map_err(|error| format!("invalid manifest: {error}"))?;
        let root = value.as_table().ok_or("manifest must be a table")?;
        let string = |key: &str| -> Result<String, String> {
            root.get(key)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("missing string field {key}"))
        };
        let revision = |key: &str| -> Result<u8, String> {
            root.get(key)
                .and_then(toml::Value::as_integer)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or_else(|| format!("invalid revision field {key}"))
        };
        let imports = root
            .get("imports")
            .and_then(toml::Value::as_array)
            .ok_or("missing imports")?;
        if imports.is_empty() {
            return Err("missing imports".into());
        }
        let namespace = string("namespace")?;
        let mut import_names = std::collections::BTreeSet::new();
        for import in imports {
            let import = import.as_table().ok_or("import must be a table")?;
            if import.get("namespace").and_then(toml::Value::as_str) != Some(namespace.as_str()) {
                return Err("import namespace disagrees with manifest namespace".into());
            }
            if import.get("direction").and_then(toml::Value::as_str) != Some("guest-to-host") {
                return Err("unexpected import direction".into());
            }
            let name = import
                .get("name")
                .and_then(toml::Value::as_str)
                .ok_or("missing import name")?;
            if !import_names.insert(name) {
                return Err("duplicate import name".into());
            }
            for field in ["params", "results"] {
                let values = import
                    .get(field)
                    .and_then(toml::Value::as_array)
                    .ok_or_else(|| format!("missing import {field}"))?;
                for value in values {
                    let value = value.as_table().ok_or("ABI value must be a table")?;
                    let semantic = value.get("semantic").and_then(toml::Value::as_str);
                    let abi = value.get("abi").and_then(toml::Value::as_str);
                    if !matches!(
                        (semantic, abi),
                        (Some("()"), Some("unit"))
                            | (Some("i32" | "u32"), Some("i32"))
                            | (Some("u64"), Some("i64"))
                    ) {
                        return Err("unsupported semantic/ABI value shape".into());
                    }
                }
            }
        }
        if root
            .get("exports")
            .is_some_and(|value| value.as_array().is_none_or(|values| !values.is_empty()))
        {
            return Err("this admission adapter does not yet support generated exports".into());
        }
        let import = imports[0].as_table().ok_or("import must be a table")?;
        if import.get("namespace").and_then(toml::Value::as_str) != Some(namespace.as_str()) {
            return Err("import namespace disagrees with manifest namespace".into());
        }
        let import_name = import
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or("missing import name")?;
        if import.get("direction").and_then(toml::Value::as_str) != Some("guest-to-host") {
            return Err("unexpected import direction".into());
        }
        Ok(Self {
            schema: string("schema")?,
            schema_revision: revision("schema_revision")?,
            generator_revision: revision("generator_revision")?,
            abi_version: revision("abi_version")?,
            namespace,
            import_name: import_name.to_owned(),
            // Exact bytes bind every signature/version, without a lossy fingerprint.
            metadata: format!("capabilities={CAPABILITIES}\noperation_protocol_revision={OPERATION_PROTOCOL_REVISION}\n{manifest}"),
        })
    }
    fn render(&self) -> String {
        format!(
            "// Generated from kernal-api-v1.abi.toml; do not edit.\n\
             #[cfg(test)]\npub(crate) const SCHEMA: &str = {schema:?};\n\
             #[cfg(test)]\npub(crate) const SCHEMA_REVISION: u8 = {schema_revision};\n\
             #[cfg(test)]\npub(crate) const GENERATOR_REVISION: u8 = {generator_revision};\n\
             #[cfg(test)]\npub(crate) const ABI_VERSION: u8 = {abi_version};\n\
             pub(crate) const NAMESPACE: &str = {namespace:?};\n\
             pub(crate) const KERNEL_YIELD: &str = {import_name:?};\n\
             pub(crate) const KERNEL_YIELD_PARAMS: &[wasmparser::ValType] = &[];\n\
             pub(crate) const KERNEL_YIELD_RESULTS: &[wasmparser::ValType] = &[];\n\
             pub(crate) const METADATA: &[u8] = {metadata:?}.as_bytes();\n",
            schema = self.schema,
            schema_revision = self.schema_revision,
            generator_revision = self.generator_revision,
            abi_version = self.abi_version,
            namespace = self.namespace,
            import_name = self.import_name,
            metadata = self.metadata
        )
    }
}

fn embed_metadata(original: &[u8], metadata: &[u8]) -> Result<Vec<u8>, String> {
    if !original.starts_with(b"\0asm\x01\0\0\0") {
        return Err("invalid core-Wasm artifact header".into());
    }
    let mut found = false;
    for payload in wasmparser::Parser::new(0).parse_all(original) {
        let payload = payload.map_err(|error| format!("invalid Wasm artifact: {error}"))?;
        if let wasmparser::Payload::CustomSection(section) = payload {
            if section.name() == METADATA_SECTION {
                if found {
                    return Err("duplicate ABI metadata sections".into());
                }
                if section.data() != metadata {
                    return Err("mismatched ABI metadata section".into());
                }
                found = true;
            }
        }
    }
    let mut output = original.to_vec();
    if !found {
        let mut body = Vec::new();
        push_leb(METADATA_SECTION.len(), &mut body);
        body.extend_from_slice(METADATA_SECTION.as_bytes());
        body.extend_from_slice(metadata);
        output.push(0);
        push_leb(body.len(), &mut output);
        output.extend_from_slice(&body);
    }
    Ok(output)
}
fn push_leb(mut value: usize, output: &mut Vec<u8>) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[allow(dead_code)]
    mod generated_guest {
        include!("../../../src/wasm/generated/v1/guest/src/lib.rs");
    }
    static YIELD_RESPONSE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);
    std::thread_local! {
        static SUBMISSION: std::cell::Cell<(i32, i64, i64)> = const { std::cell::Cell::new((0, 0, 0)) };
        static POLL_RESPONSE: std::cell::Cell<i64> = const { std::cell::Cell::new((42 << 8) | 1) };
    }

    // Exercise the checked-in guest facade against the scalar import boundary,
    // including the negative failure sentinel emitted by the async host linker.
    #[no_mangle]
    extern "C" fn operation_submit(kind: i32, arg0: i64, arg1: i64) -> i64 {
        SUBMISSION.set((kind, arg0, arg1));
        1
    }
    #[no_mangle]
    extern "C" fn operation_yield(_operation: i64) -> i32 {
        YIELD_RESPONSE.load(std::sync::atomic::Ordering::SeqCst)
    }
    #[no_mangle]
    extern "C" fn operation_poll(_operation: i64) -> i64 {
        POLL_RESPONSE.get()
    }
    #[test]
    fn generated_blob_abandon_is_a_scoped_scalar_submission() {
        generated_guest::BlobHandle::from_create_payload(42).abandon();
        assert_eq!(SUBMISSION.get(), (19, 42, 0));
    }

    #[test]
    fn generated_encrypted_input_grant_and_abandon_are_scoped_scalar_submissions() {
        let input = generated_guest::EncryptedArchive::granted()
            .unwrap()
            .unwrap();
        assert_eq!(SUBMISSION.get(), (20, 0, 0));
        input.abandon();
        assert_eq!(SUBMISSION.get(), (22, 1, 0));
    }

    #[test]
    fn generated_poll_preserves_rejected_terminal_status() {
        let operation = generated_guest::synthetic_yield().unwrap();
        POLL_RESPONSE.set(7);
        assert_eq!(
            operation.poll(),
            Err(generated_guest::OperationError::Rejected)
        );
        POLL_RESPONSE.set(3);
        assert_eq!(
            operation.poll(),
            Err(generated_guest::OperationError::TimedOut)
        );
        POLL_RESPONSE.set(0x80);
        assert_eq!(
            operation.poll(),
            Err(generated_guest::OperationError::Failed)
        );
    }
    #[test]
    fn generated_webview_sequence_exchanges_only_opaque_scalars() {
        POLL_RESPONSE.set((42 << 8) | 1);
        generated_guest::run(async {
            let url = generated_guest::WebviewUrl::granted()?.unwrap();
            assert_eq!(SUBMISSION.get(), (13, 0, 0));
            let view = url.open().await?;
            assert_eq!(SUBMISSION.get(), (14, 1, 0));
            view.wait_until_loaded().await?;
            assert_eq!(SUBMISSION.get(), (15, 42, 0));
            generated_guest::clock_sleep(5_000)?.wait().await?;
            assert_eq!(SUBMISSION.get(), (12, 5_000, 0));
            let snapshot = view.capture_visible_png().await?;
            assert_eq!(SUBMISSION.get(), (16, 42, 0));
            generated_guest::OutputFile::from_granted_token(7)
                .write_blob(&snapshot)?
                .wait()
                .await?;
            assert_eq!(SUBMISSION.get(), (10, 42, 7));
            view.close().await?;
            assert_eq!(SUBMISSION.get(), (17, 42, 0));
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn generated_driver_rejects_foreign_pending_and_zero_resource_results() {
        assert_eq!(
            generated_guest::run(std::future::pending::<
                Result<(), generated_guest::OperationError>,
            >()),
            Err(generated_guest::OperationError::Failed),
        );
        POLL_RESPONSE.set(1);
        let url = generated_guest::WebviewUrl::granted().unwrap().unwrap();
        assert!(matches!(
            generated_guest::run(url.open()),
            Err(generated_guest::OperationError::Failed)
        ));
    }
    #[test]
    fn generated_guest_yield_accepts_only_host_success() {
        let operation = generated_guest::synthetic_yield().unwrap();
        for response in [1, -1, 0, 2, i32::MIN, i32::MAX] {
            YIELD_RESPONSE.store(response, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(
                operation.yield_now(),
                if response == 1 {
                    Ok(())
                } else {
                    Err(generated_guest::OperationError::Failed)
                },
                "host yield response {response}"
            );
        }
    }
    #[test]
    fn generated_clock_uses_only_the_scalar_operation_boundary() {
        for milliseconds in [0, 5_000, u32::MAX] {
            let _ = generated_guest::clock_sleep(milliseconds).unwrap();
            assert_eq!(SUBMISSION.get(), (12, i64::from(milliseconds), 0));
        }
    }
    const MANIFEST: &str = include_str!("../../../src/wasm/generated/v1/kernal-api-v1.abi.toml");
    const EMPTY: &[u8] = b"\0asm\x01\0\0\0";

    #[test]
    fn metadata_binds_the_entire_generated_contract() {
        let contract = Contract::parse(MANIFEST).unwrap();
        assert_eq!(
            contract.metadata,
            format!("capabilities=0\noperation_protocol_revision=3\n{MANIFEST}")
        );
        let changed = MANIFEST.replace("abi_version = 1", "abi_version = 2");
        assert_ne!(
            contract.metadata,
            Contract::parse(&changed).unwrap().metadata
        );
    }
    #[test]
    fn unsupported_shapes_cannot_borrow_another_imports_signature() {
        let mut value: toml::Value = MANIFEST.parse().unwrap();
        let imports = value.get_mut("imports").unwrap().as_array_mut().unwrap();
        let other = imports[0].clone();
        imports[0]["params"] = toml::Value::Array(vec![toml::Value::String("i32".into())]);
        imports.push(other);
        assert!(Contract::parse(&value.to_string()).is_err());
        value["imports"].as_array_mut().unwrap().pop();
        assert!(Contract::parse(&value.to_string()).is_err());
        assert!(Contract::parse(&MANIFEST.replace("abi = \"unit\"", "abi = \"i64\"")).is_err());
    }
    #[test]
    fn embedding_is_idempotent_and_rejects_mismatch_duplicate_and_malformed_sections() {
        let metadata = Contract::parse(MANIFEST).unwrap().metadata;
        let once = embed_metadata(EMPTY, metadata.as_bytes()).unwrap();
        assert_eq!(once, embed_metadata(&once, metadata.as_bytes()).unwrap());
        assert!(embed_metadata(&once, b"wrong contract").is_err());
        let mut duplicated = once.clone();
        duplicated.extend_from_slice(&once[EMPTY.len()..]);
        assert!(embed_metadata(&duplicated, metadata.as_bytes())
            .unwrap_err()
            .contains("duplicate"));
        assert!(embed_metadata(&once[..once.len() - 1], metadata.as_bytes()).is_err());
    }
    #[test]
    fn incidental_marker_and_metadata_bytes_do_not_count_as_an_abi_section() {
        let metadata = Contract::parse(MANIFEST).unwrap().metadata;
        let mut original = EMPTY.to_vec();
        let mut body = Vec::new();
        push_leb("debug".len(), &mut body);
        body.extend_from_slice(b"debug");
        body.extend_from_slice(METADATA_SECTION.as_bytes());
        body.extend_from_slice(metadata.as_bytes());
        original.push(0);
        push_leb(body.len(), &mut original);
        original.extend_from_slice(&body);
        let result = embed_metadata(&original, metadata.as_bytes()).unwrap();
        assert!(result.len() > original.len());
        assert_eq!(
            result,
            embed_metadata(&result, metadata.as_bytes()).unwrap()
        );
    }
}
