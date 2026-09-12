// Scalar protocol constants shared by the declaration frontend and generated
// private host support. This file defines no transport or public backend API.
pub const CORE_ABI_METADATA: &[u8] = b"fp-bindgen.core-wasm-abi@1;capabilities=0";
pub const ABI_VERSION: u32 = 1;
pub const CAPABILITIES: u64 = 0;
pub const INVALID_REQUEST: u64 = 0;
pub const POLL_PENDING: u32 = 0;
pub const POLL_READY: u32 = 1;
pub const POLL_CANCELLED: u32 = 2;
pub const POLL_REJECTED: u32 = 3;
pub const OP_YIELD: u32 = 0;
pub const OP_RESOURCE_CREATE: u32 = 1;
pub const OP_RESOURCE_USE: u32 = 2;
pub const OP_RESOURCE_CLOSE: u32 = 3;
pub const STATUS_PENDING: u16 = 0;
pub const STATUS_OK: u16 = 1;
pub const ERROR_STALE_OR_FORGED: u16 = 2;
pub const ERROR_FOREIGN_SCOPE: u16 = 3;
pub const ERROR_QUOTA: u16 = 4;
pub const ERROR_CANCELLED: u16 = 5;
pub const ERROR_TIMEOUT: u16 = 6;
pub const ERROR_UNSUPPORTED: u16 = 7;
pub const ERROR_WRONG_RIGHTS: u16 = 8;
pub const ERROR_WRONG_KIND: u16 = 9;
pub const ERROR_INVALID_COMPLETION_FIELD: u16 = 10;
