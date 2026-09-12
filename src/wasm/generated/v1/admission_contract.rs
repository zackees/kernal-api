// Generated from kernal-api-v1.abi.toml; do not edit.
#[cfg(test)]
pub(crate) const SCHEMA: &str = "fp-bindgen.core-wasm-abi";
#[cfg(test)]
pub(crate) const SCHEMA_REVISION: u8 = 1;
#[cfg(test)]
pub(crate) const GENERATOR_REVISION: u8 = 1;
#[cfg(test)]
pub(crate) const ABI_VERSION: u8 = 1;
pub(crate) const NAMESPACE: &str = "kernal-api:v1";
pub(crate) const KERNEL_YIELD: &str = "kernel_yield";
pub(crate) const KERNEL_YIELD_PARAMS: &[wasmparser::ValType] = &[];
pub(crate) const KERNEL_YIELD_RESULTS: &[wasmparser::ValType] = &[];
pub(crate) const METADATA: &[u8] = "capabilities=0\nschema = \"fp-bindgen.core-wasm-abi\"\nschema_revision = 1\ngenerator_revision = 1\nabi_version = 1\nnamespace = \"kernal-api:v1\"\n\n[[imports]]\nnamespace = \"kernal-api:v1\"\nname = \"kernel_yield\"\ndirection = \"guest-to-host\"\nparams = []\nresults = [{ semantic = \"()\", abi = \"unit\" }]\n\n".as_bytes();
