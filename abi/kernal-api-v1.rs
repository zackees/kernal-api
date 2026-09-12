// The single, versioned declaration source for the generated Core-Wasm ABI.
//
// The wire is deliberately all scalars. `operation` is a stable u32 namespace:
// 0 is `yield`, 1 is `resource-create`, 2 is `resource-use`, and 3 is
// `resource-close`; 0x8000_0000..=u32::MAX is reserved for incompatible
// future extensions. Each argument is an operation-defined u64 control word,
// never a pointer, length, serialized value, path, or platform object.
//
// `submit` returns a host-assigned opaque request u64. Guests cannot select a
// request identity, and scope is selected by host instance context, never by
// a guest argument. `poll` returns a u32 state/status code (0 pending, 1
// ready, 2 cancelled, 3 rejected). After terminal status, `completion_word`
// reads one of a fixed six u64 fields: 0 status/error, 1 value, 2 reserved,
// and 3..=5 resource scope/slot/generation. Thus a resource identity is the
// full host-validated triple rather than a lossy packed integer. A future
// lifecycle implementation must retain/release host-issued request tokens and
// reject forged, stale, or foreign resource triples without effects. #37
// supplies lifecycle semantics and #38 supplies the actual operations; this
// source reserves their complete bounded scalar representation now.

include!("constants.rs");

fp_import! {
    /// Returns the ABI version (1) and proves that the guest selected v1.
    fn abi_version() -> u32;
    /// Returns the host capability bitset; v1 defines only bit zero as reserved.
    fn capability_bits() -> u64;
    /// Begins a scalar operation and returns a host-assigned opaque request id.
    fn submit(operation: u32, argument0: u64, argument1: u64, argument2: u64) -> u64;
    /// Returns a terminal/pending state code for a host-assigned request id.
    fn poll(request: u64) -> u32;
    /// Reads one fixed completion word after a terminal poll result.
    fn completion_word(request: u64, field: u32) -> u64;
    /// Acknowledges a terminal request and releases its host token. Repeated,
    /// pending, forged, and foreign requests return their reserved status ids.
    fn release(request: u64) -> u32;
    /// Reserves cooperative yielding without an async transport.
    fn yield_now() -> u32;
    /// Reserves cancellation without defining cancellation behavior yet.
    fn cancel(request: u64) -> u32;
}

fp_export! {}
