// Test-only copy of fp-bindgen-support/src/common/mem.rs at
// 4e44d9e5408653e3c428ee3f855cc194d53f60b0. See LICENSE-MIT in this directory.
// Keep this executable evidence independent from our generated ABI.
#[doc(hidden)]
pub type FatPtr = u64;

#[doc(hidden)]
pub fn to_fat_ptr(ptr: *const u8, len: u32) -> FatPtr {
    (ptr as FatPtr) << 32 | (len as FatPtr)
}

#[doc(hidden)]
pub fn from_fat_ptr(ptr: FatPtr) -> (*const u8, u32) {
    ((ptr >> 32) as *const u8, (ptr & 0xffffffff) as u32)
}
