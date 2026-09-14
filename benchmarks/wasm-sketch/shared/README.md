# Shared candidate policy

`hash_policy.rs` contains the async public-facade correctness policy, separate
from runtime-specific entrypoint glue. It checks empty input, two chunkings of
64 MiB, and resource Drop using only `kernal_api::guest` semantic types.
It also runs actual upstream request-key encoding and includes `rustc_policy.rs`
for the pinned upstream compiler parser. The latter is compiled unchanged into
the native test and both guests. Current scope and evidence are recorded in
[the policy experiment](../../../docs/wasm-zccache-policy-experiment.md).

Both candidate guests include this exact source. The Component generated async
export awaits the function directly; the Core wrapper uses its suspending
runner. The temporary `wasm-component-hash-experiment` feature selects only the
private hash transport, not a second supported runtime. The actual Linux
Component hash test passed in 3.52 s, including the shared policy, an intentionally
failed export, oversized canonical input rejection, and zero live hashes after
each export and store teardown. This is hash correctness, not full runtime parity.

## Adapter requirements before comparison

Keep concrete adapters private behind the same facade-owned types; select the
experimental backend at build time, never through a production runtime fallback.
The Component host must use `kernal_api::hash::Blake3Hasher`, enforce the 64 KiB
update bound and resource quota itself, and reclaim uncertain hash state when
an update is abandoned. Dropping only a generated async future does not prove
that the host resource was revoked.

The execution gate uses this exact policy through the Component-selected
public facade, with identical native digest expectations and zero live hash
resources after success, explicit Drop, and failed execution. Hash parity
does not establish Blob parity: the latter also needs bounded pending reads
and writes, synchronous authority revocation on Drop, successful EOF distinct
from producer failure, and cancellation followed by same-instance reuse.

Current public pending-operation wait loops call Core's synchronous
`yield_now()`. A Component adapter must provide a real asynchronous wait beneath
the public async methods. Silently spinning, replacing cancellation with store
destruction, or treating a dropped producer as successful EOF changes semantics
and cannot satisfy the comparison.

The initial actual Component encoding failed on the Core facade's
`kernal-api:v1::operation_poll` import (Soldr record
`20260913T100423Z-home-niteris-dev-kernal-api.xml`). The Component hash binding
uses a separate typed WIT interface and its synchronous resource destructor;
bounded CPU work completes atomically inside each host import. The encoder
requires exactly the existing private blob interface and the hash interface,
so accidentally pulling in Core-only facade methods fails closed.

The experimental `list<u8>` transport checks the 64 KiB update limit after
canonical lifting, before hash mutation. This does **not** establish a 64 KiB
host-allocation bound for hostile canonical arguments. The 8 MiB guest memory
cap is not a substitute for pre-lift validation or aggregate concurrent-call
accounting. Hash-only resource admission is explicitly limited to 64 handles;
the older private blob probe does not yet share that table or budget.
