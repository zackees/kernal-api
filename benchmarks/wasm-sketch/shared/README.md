# Shared candidate policy

`hash_policy.rs` contains the async public-facade correctness policy, separate
from runtime-specific entrypoint glue. It checks empty input, two chunkings of
64 MiB, and resource Drop using only `kernal_api::guest` semantic types.

The Core hash guest currently includes this exact source. Component integration
is pending: its generated async export must await this function directly, not
use the Core runner or spin a noop waker. A shared source file alone is not
evidence of Component execution or runtime parity.

## Adapter requirements before comparison

Keep concrete adapters private behind the same facade-owned types; select the
experimental backend at build time, never through a production runtime fallback.
The Component host must use `kernal_api::hash::Blake3Hasher`, enforce the 64 KiB
update bound and resource quota itself, and reclaim uncertain hash state when
an update is abandoned. Dropping only a generated async future does not prove
that the host resource was revoked.

The next execution gate is this exact policy through the Component-selected
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
