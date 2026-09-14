# Private Core-Wasm ABI generator

`soldr cargo run --locked --manifest-path tools/wasm-abi-generator/Cargo.toml`
regenerates the checked-in scalar v1 guest crate, private Wasmtime linker, and
ABI manifest in `src/wasm/generated/v1` from the declaration in `src/main.rs`.
The guest package lives in its `guest/` subdirectory so Cargo includes the
private host files when packaging the facade.

`ci/check_wasm_abi_generation.sh` regenerates into a temporary directory and
compares every generated file. It is the required drift check and leaves the
working tree untouched.

The threaded build scripts copy Cargo's Wasm output and embed the generated
`kernal-api.abi` custom section in that copy. Its contents bind the complete
manifest and the declared capability set. Admission checks these bytes before
compilation. Embedding parses real section boundaries, leaves an already
matching artifact unchanged, and rejects mismatched or duplicate sections.
Changes to the ABI therefore require no manual edits to the guest artifact.

The metadata also binds `operation_protocol_revision=10`, independently of the
scalar import signatures. Revision 1 included opcodes 1–19 and scoped
transfer/blob abandonment. Revision 2 adds encrypted-input grant, bounded
header copy, and abandonment (20–22). Revision 3 adds authentication and scoped
future/archive abandonment (23–25). Revision 4 adds bounded inventory,
entry metadata, and entry abandonment (26–28), with opcode 24 also abandoning
inventory futures. Revision 5 adds entry-to-Blob open (29), whose future also
uses scoped abandonment (24); native input grants currently exist only
in the test-support experiment. Revision 6 adds BLAKE3 create, bounded update,
finalize, resource abandonment, and operation abandonment (30–34). Updates
copy at most 64 KiB per import into host-owned memory. Finalize validates the
complete 32-byte output range before consuming the hash. An abandoned update
revokes the hash rather than promising rollback of bytes already accepted.
Revision 7 adds host-granted compiler spawn, tagged bounded output collection,
exit observation, acknowledged close, and scoped abandonment (35–47). Command
paths and arguments remain host-owned. Output collection requires a 64-KiB
destination and writes directly to validated shared memory without another
host payload copy. EOF, abandonment, read failure, and exhaustion are distinct.
Revision 9 adds generated `resource_release_blob`: it is not an opcode, but a
Store-scoped owned-Blob release control delegated to the same `OperationHub`
registry as the existing blob-abandon path. Revision 10 applies the same
generated release model to encrypted-input, authenticated-archive, and
archive-entry authorities. Revision-9 guests must be rebuilt, not relabeled,
before they can import the archive controls.
Rebuild guest code before embedding the new
metadata; never relabel an older binary. Bump this revision when operation
semantics change even if the scalar function signatures remain identical.
Unversioned and unknown operation revisions are rejected before compilation;
there is no legacy-host fallback. This closes the gap where an older host could
accept a newer drop-capable guest because its import signatures still matched.

Rebuild artifacts from matching guest sources with the normal build scripts.
An already embedded legacy artifact is rejected by the embedder rather than
silently relabeled. Metadata is a compatibility declaration, not attestation
of guest source behavior; it never grants resource authority by itself.

Run `soldr cargo test --locked --manifest-path tools/wasm-abi-generator/Cargo.toml`
for malformed-manifest/metadata, idempotency, and historical transport checks.

The tool pins `zackees/fp-bindgen` at `df94a6988dac3ade189466adbaaa58be4524393e`.
It is a development tool outside the published package, so ordinary
`kernal-api` builds do not resolve the generator or its dependency graph.

That revision declares the generated `Blob`, `EncryptedArchive`,
`AuthenticatedArchive`, and `ArchiveEntry` guest wrappers as owned resources.
Each generated release control calls the existing `OperationHub` registry,
which atomically validates Store scope, kind, and generation before revoking
the resource. It does not introduce a second guest resource registry or a
raw-token public API.

The generated scalar contract provides `kernel_yield` and operation
submit/poll/yield/cancel imports. Semantic resource operations are closed
submit opcodes, not additional handwritten imports. Opcode 12 is
`clock_sleep(milliseconds: u32)`: it uses the supplied kernel runtime's
monotonic timer and the ordinary operation future. It has no resource payload
or guest clock import. Reserved arguments and oversized durations are rejected.
Cancellation releases semantic authority immediately, while queued timer tasks
retain bounded admission until they drain. Logical-root finalization revokes
and drains timers before returning.

The threaded smoke guest exercises a ten-millisecond generated sleep. The
five-second, post-load screenshot sequence remains part of #20; this primitive
alone does not establish that proof.

The screenshot guest contract reserves opcodes 13–17 for URL-grant discovery,
open, matching load wait, viewport capture, and close. These generated guest
methods exchange only opaque tokens. Their native host dispatch uses the same
logical-root hub. `OperationFuture::wait` and `run` provide composition for this
single generated command; the async host yield suspends the Wasm stack rather
than constructing a guest runtime. The driver rejects unrelated futures that
return `Pending`. See `examples/wasm-tauri-screenshot` for the actual Rust
application and the remaining native-runner evidence.

Opcode 18 is synchronous `transfer_abandon(operation_token, 0)`. It removes an
owned blob-read or blob-write operation, cancels any pending producer, and
discards an uncollected result, including completed read bytes. It allocates
no operation slot and returns 1 on success or 0 for invalid, foreign-store,
non-transfer tokens or nonzero reserved arguments. Repeated abandonment is
safe but returns 0. Explicit `operation_cancel` is unchanged: its terminal
result remains available to poll. Public transfer guards use abandonment on
drop. This additive opcode requires a matching guest-capable host release;
the exact pre-1.0 client pin must not pair these guards with an older host.

Opcode 19 is synchronous `blob_abandon(blob_token, 0)`. It remains the
registry's low-level scoped release path: it validates Store ownership and blob
kind under the same lock that revokes the generation, closes borrowing
operations, and releases stored bytes. The revision-9 generated guest wrapper
now calls `resource_release_blob`, whose host implementation delegates to that
same path and maps its success to the generated release status. Pending futures
retain typed terminal results until collection or their own abandonment.
