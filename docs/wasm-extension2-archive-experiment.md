# Extension2 sealed archive experiment (#13): foundation only

The native `archive` feature now has independent aggregate output and
per-entry payload ceilings. Set `ExtractionLimits::max_entry_bytes` for the
entry policy; `max_output_bytes` still bounds aggregate extracted bytes.
The default per-entry ceiling is 64 GiB. Callers must set their product limits
explicitly (the #13 extension2 experiment calls for a 32 MiB XPI entry limit).
Zero allows empty payloads, and exact-limit entries are accepted.

ZIP checks advertised size before creating an entry and bounds actual copied
bytes as well. Tar checks effective payload size, including local PAX size,
in its physical-record guard before exposing the body to the backend. This
also covers entries skipped by selected-member extraction. Tar extension
records remain governed by metadata limits, not the per-entry payload limit.
Extraction can leave earlier output on error: callers still own exclusive
staging and cleanup, as documented by the native API.

## Focused evidence

`archive_entry_limit_is_independent_of_total_output` first failed to compile
because the field was absent. With the field present but unenforced, it failed
at runtime because an oversized entry extracted successfully. With enforcement,
the same regression passes for ZIP, gzip-tar, and zstd-tar, rejecting a five-byte
entry under a four-byte ceiling before creating that file and accepting two
five-byte entries under a five-byte entry / ten-byte aggregate ceiling.
Selected tar extraction cannot bypass the earlier oversized entry.

`entry_copy_limits_actual_bytes_even_without_size_metadata` checks the actual
copy boundary independently of archive size metadata, including zero, exact,
over-limit, and aggregate-budget cases.

```sh
soldr cargo test --locked --features archive --test archive_facade
soldr cargo test --locked --features archive --lib entry_budget_tests
soldr cargo clippy --locked --features archive --lib --test archive_facade -- -D warnings
```

## Still required

The next guest boundary is now an executable [RED contract](../benchmarks/wasm-sketch/extension2-guest/README.md).
Its original `guest-proof` Wasm build failed with E0432 because
`kernal_api::guest::EncryptedArchive` was absent (Soldr log
`20260913T060627Z-home-niteris-dev-kernal-api.xml`). The source requires
bounded original-header validation, authentication before archive authority,
inventory policy, and byte-for-byte 17 MiB entry reads through the public Blob
facade. Its separate native policy tests are not Wasm execution or unchanged
upstream-policy equivalence evidence. Generated operations, host dispatch,
and the encrypted fixture driver must make this same full contract GREEN.
The header-only control now executes grant, bounded original-header policy,
and revocation in a real Rust Wasm guest (valid, wrong identity, missing grant).
The header control uses an invalid sparse
ciphertext tail, not an encrypted ZIP; it is no authentication evidence.
The separate authentication control now executes a genuine encrypted ZIP,
accepts its valid tag, and rejects corrupted tags and wrong nonces. The full
build still lacks `next_entry`; no guest entry streaming is claimed.
Build and execution commands are in the linked README. Protocol revision 3
requires freshly rebuilt artifacts, not metadata relabeling of old binaries.

This is a reusable native prerequisite, not an extension2/Wasm GREEN claim.
The experiment still needs a real extension2 policy fixture, authenticated
AES-GCM staging with no readable plaintext before verification, seekable
opaque archive resources through the generated guest facade, streamed entries
larger than 16 MiB without guest whole-archive allocation, cancellation and
teardown proofs, and validation on all six native targets. Existing blob
`seal` means producer EOF; it must not be mistaken for cryptographic
authentication. Do not replace the native archive implementation with another
guest-only extractor or introduce ambient guest file paths.

## Actual extension2 source and RED build

Inspected the clean local extension2 checkout at
`f1e1173b3136a46d1ca778840a7b775411581096`, specifically
`crates/tw-orange-preview-source/src/implementation.rs` and its manifest.
No upstream source or content key was changed or copied into this repo.

The actual envelope is `TWPV1AES`, followed by a big-endian four-byte JSON
header length, a header of at most 16 KiB, and one AES-128-GCM ciphertext/tag.
The complete prefix and original header bytes are authenticated additional
data. Header identity binds schema 1, algorithm, version, commit, and key ID
to the release; the base64 nonce must decode to 12 bytes. The native caller
also checks encrypted size and SHA-256 against the release before decryption.
A replacement must preserve these bytes and checks, not reserialize the AAD
or substitute independently authenticated chunk records for the one message.
Use synthetic test keys for the experiment, not the private producer key.

The source currently decrypts into a whole `Vec<u8>` and retains extracted
entries in a `HashMap<String, Vec<u8>>`. Its archive policy is 512 MiB and
16,384 entries; the nested XPI policy is 128 MiB total, 32 MiB per file, and
4,096 entries. This policy does not become a streaming implementation merely
by compiling its existing decoder to Wasm. Metadata/feed identity, duplicate
names, safe relative paths, and XPI manifest policy must remain in the sketch,
while authentication, seekable storage, and bounded bulk copying remain host
capabilities. Native extractor path acceptance is not a replacement for the
stricter application inventory policy.

The following real check failed on Linux x86-64 with Rust 1.95.0, before the
preview source could compile, at `getrandom 0.2.17`'s unsupported
`wasm32-unknown-unknown` target diagnostic:

```sh
soldr --no-cache cargo check --locked \
  --manifest-path /home/niteris/dev/twp/extension2/Cargo.toml \
  -p tw-orange-preview-source --target wasm32-unknown-unknown \
  --target-dir /tmp/kernal-extension2-wasm-proof -j1
```

The inverse dependency tree confirms
`getrandom <- rand_core <- crypto-common <- aead <- aes-gcm <- tw-orange-preview-source`.
The build log is `20260913T045050Z-home-niteris-dev-kernal-api.xml` under
the local Soldr build logs. Enabling JavaScript randomness is not an acceptable
fix for this non-ambient guest. Removing that dependency edge alone also
would not satisfy streaming, authentication visibility, or archive authority.

The same command with `-p tw-orange-data-source` succeeds unchanged in
23.91 seconds using the same target directory and source revision. That is
a compile-only positive control for the portable metadata/feed policy crate,
not execution of the preview decoder or a Wasm archive fixture.

The upstream producer-contract test returns successfully without exercising
the producer bundle when `TW_ORANGE_PREVIEW_TEST_SITE` is unset. Its default
test result therefore must not be cited as encrypted fixture evidence.

## Required storage and authentication boundary

The current `ResourceValue::Blob` in `src/wasm/operations.rs` is a bounded
`VecDeque<u8>` with a producer-EOF flag. Reads consume its contents. It cannot
provide ZIP central-directory seeks, and increasing its byte quota to hold
the archive would defeat the independent-of-total-length memory requirement.

The next implementation must add private seekable backing under the existing
resource registry, with separate retained-memory and staged-storage quotas.
It must not turn the existing blob's EOF flag into an authentication verdict.
The required lifecycle is:

1. Reserve bounded staging capacity under the logical sketch before consuming
   ciphertext. Keep the staging handle private and unregistered as readable.
2. Validate the bounded envelope and stream ciphertext through a maintained
   AES-GCM implementation using the original AAD and one final tag. Write
   intermediate plaintext only to private staging, never to a guest-readable
   queue, output capability, archive parser, or progress callback.
3. Verify the final tag and release identity before making staging readable.
   On malformed input, authentication failure, cancellation, trap, or quota
   failure, close staging and release its storage reservation exactly once.
4. Register an authenticated seekable resource using the existing scoped,
   generation-safe token machinery. Archive inspection and entry reads then
   use bounded operations; the sketch receives bounded inventory records and
   applies extension2 policy, not a native path or whole ZIP.

The distinction between update output and authenticated output is explicit
in the maintained [OpenSSL authenticated-decryption example](https://github.com/openssl/openssl/blob/master/demos/cipher/aesgcm.c)
and [Rust OpenSSL Crypter API](https://docs.rs/openssl/0.10.81/openssl/symm/struct.Crypter.html).
The test-gated native prototype uses exact OpenSSL 0.10.81 with a vendored
backend under `archive-auth-test-support`; this does not select a Wasm binding
candidate or expose a production guest capability.
Do not hand-roll AES/GHASH to avoid a dependency, or use per-chunk AEAD records
that are incompatible with the existing single-message envelope.

The first integration regressions must pause before final authentication and
prove that no readable resource or output exists, then independently mutate
AAD, ciphertext, and tag and require the same cleanup. A successful synthetic
key fixture must exceed 16 MiB, preserve bytes through bounded reads and
central-directory seeks, and report peak memory, staged bytes, and zero live
resources after close/cancel/teardown. A native crypto-only test is not a
substitute for running that sequence through the actual guest ABI.

## Native authenticated-staging prototype

`src/archive/authenticated_staging.rs` is compiled only for tests with
`archive-auth-test-support`. Its pending type has no read/seek/path accessor.
Updates are capped at 64 KiB and write only to an anonymous temporary file.
Authentication consumes the pending state, verifies the expected length and
the single final GCM tag, then returns the seekable file with its storage
reservation. An update error permanently invalidates the pending state.
Drop, errors, and failed authentication close staging and release accounting;
successful authentication keeps the reservation charged until file drop.

This is synchronous native prototype work, not an async guest operation.
The initial staging tests construct their own accounting; the separate
test-gated registry integration below adds hub-owned accounting.
Accounting uses a private shared `StagingBudget`
whose maximum is fixed at construction, rather than accepting a separate
counter and caller-selected limit on every authentication. Budget clones
share the same ceiling and reservations keep that budget alive through file
drop. It is not yet a production sketch quota. The operation-table experiment
below adds cleanup of idle pending authentication on native cancellation;
guest execution, worker teardown, and progress deadlines remain unimplemented.
OS file cache/RSS and secure physical erasure of
temporary plaintext are not proven by byte accounting. No claim is made that
enabling the test-support feature implements the production archive contract.

```sh
soldr cargo test --locked --features archive-auth-test-support --lib authenticated_staging
```

On Linux x86-64 the initial vendored build took 223.30 seconds; all three
native tests passed in 0.06 seconds. Tests cover the standard zero-key GCM
vector, a 17 MiB single message with separate AAD/ciphertext/tag mutations,
incomplete input, per-update rejection, aggregate reservation denial, and
reservation release after success/failure/drop. The large case checks a seek
to the final chunk; it is not an encrypted ZIP or a guest execution test.
A temporary mutation that ignored failed final authentication caused the
known-vector regression to fail by returning an authenticated file for a bad
tag. Restoring error propagation returned all three tests to GREEN.

The next native integration, `encrypted_large_zip_reuses_bounded_extractor_only_after_authentication`,
adds a stored ZIP containing a 17 MiB entry. It creates, encrypts, decrypts,
and verifies with 64 KiB transfer buffers, authenticates before calling the
existing extractor, and verifies every extracted byte. Its ciphertext itself
exceeds 16 MiB (not merely its decompressed output). The private consuming
`Authenticated::extract` keeps the staging reservation alive until extraction
returns, including failure paths, and never reopens a source pathname.

The regression first failed because that consuming extraction method was
absent. It now passes success, bad-tag, per-entry ceiling, entry-count ceiling,
and escaping-path cases; no destination is created before authentication and
the storage counter returns to zero in every case. The four native staging
tests pass together in 0.56 seconds on Linux x86-64. This remains a synthetic
native fixture: real extension2 envelope parsing/identity policy, guest ABI
execution, asynchronous cancellation, and worker teardown are still required.

## Owned authenticated entry reader

The test-gated `Authenticated::into_reader` now consumes authenticated staging
into a private, owned ZIP reader. It shares the native extractor's ZIP opener,
including input, central-directory and entry-count preflight, and the existing
64 KiB bounded copy helper. One bounded semantic inventory record is returned
at a time, after name/path and per-entry validation. The initial streaming
seam accepts regular files only; directories, links and special entries are
rejected rather than exposed as ordinary file streams. Product inventory
decisions still belong in the guest.

The metadata-read control now uses an `Arc<AtomicU64>` instead of `Rc<Cell>` so
the parsed reader can move to a blocking worker. This is serialized access to
one non-cloneable reader, not a concurrent quota. The existing staged-storage
reservation remains inside the authenticated wrapper until its ZIP/file is
closed. A copy error permanently invalidates the reader: a partial failing
sink write must not allow a retry with reusable aggregate accounting.

The focused regressions first failed because `into_reader` was absent (Soldr
log `20260913T061359Z-home-niteris-dev-kernal-api.xml`). The large encrypted ZIP
test now hands the reader to a worker, validates its inventory, verifies every
byte of the 17 MiB entry through a bounded sink, and observes retained storage
during every write and zero storage after drop. Separate tests cover rejected
metadata/input/name/entry limits, unsafe paths, unsupported entry kinds, empty
files, aggregate-budget reuse, and a sink that fails after a partial write.
A temporary mutation disabling reader invalidation made the partial-sink
regression fail because a retry succeeded (Soldr log
`20260913T061836Z-home-niteris-dev-kernal-api.xml`); the guard was restored.

This is the native entry-streaming seam required by the guest contract, not
its completed implementation. Generated inventory/entry operations and their
real guest execution remain missing; header and authentication boundaries now
have actual guest controls. The original
synchronous `Write` sink tests do not prove asynchronous backpressure; the
native Blob bridge below adds that separate proof. Neither proves cancellation
of blocked filesystem operations.

## Native entry-to-Blob bridge

`src/wasm/authenticated_blob.rs` now connects the owned reader to the existing
capacity-awaited Blob writes under the test-only host experiment. A private,
non-cloneable producer creates a read-only consumer Blob. Ordinary write/seal
operations still require WRITE authority and reject that consumer handle.
There is no producer constructor accepting an arbitrary consumer token.

The blocking copy worker uses the caller's existing async-engine handle to
observe, suspend, and await each bounded write. It holds at most one pending
chunk and never waits with the operation-table mutex held. No second executor
or queue was added. A pending-write guard synchronously abandons uncollected
operations and their buffers on error/unwind. Only a successful complete
archive copy seals EOF; flush does not seal, and failure permanently invalidates
the producer. Unfinished producer drop closes the Blob instead of reporting EOF.

The bridge tests initially failed to compile because `NativeArchiveSink` was
absent (Soldr log `20260913T062645Z-home-niteris-dev-kernal-api.xml`). They now
stream and verify all 17 MiB through the same read-result collection machinery
used by guest Blob reads, stop at a 128 KiB Blob plus one pending 64 KiB write
while the consumer pauses, and resume after consumption. The hub's retained
transfer-capacity peak stays within its configured ceiling. This accounting
does not include the reader's fixed 64 KiB stack buffer or ZIP metadata budget,
and is not process RSS evidence.

Consumer drop, trap and timeout teardown wake the blocked producer, join the
copy worker, and leave zero storage, resources, operations and retained transfer
buffers. A separate test consumes a readable prefix before a write-submission
failure or producer panic, then requires closure rather than successful EOF.
Disabling the failed-producer guard made this regression fail because `finish`
accepted EOF after the failed write (Soldr log
`20260913T063059Z-home-niteris-dev-kernal-api.xml`); the guard was restored.

These are native integration tests, not execution of the guest archive
contract. The driver still needs generated operations, host grant/dispatch,
bounded entry-copy job accounting and draining, progress deadlines,
worker containment and actual guest/six-target validation. Operation-slot limits
alone do not bound native jobs that outlive a cancelled operation. The fixture's
owned reader is supplied directly by trusted native test code, not discovered
through guest archive authority.

## Tracked native encrypted-input authentication

The test-only `archive_input` driver now consumes an input grant into a tracked
job on the supplied kernel runtime's blocking lane. It reads ciphertext in
64 KiB chunks, passes the original prefix/header as AAD to the existing staged
authentication implementation, and publishes through the existing atomic
final-tag handoff. The old input token becomes stale at admission. No source
path is reopened, no Store is retained, and no whole archive is allocated.

Queued/running authentication jobs have a separate ceiling equal to the hub's
operation limit. Collecting a cancellation terminal does not release that job
slot. Root teardown revokes operations and awaits these jobs outside the hub
mutex; worker panics remain recorded even after finished handles are pruned.
An idle staging object can be reclaimed immediately on cancellation; an
in-flight staging write retains its own charge until the write returns.

The first driver test failed because submission/draining APIs were absent
(Soldr log `20260913T071734Z-home-niteris-dev-kernal-api.xml`). Native tests now
cover a real encrypted ZIP containing 17 MiB of byte-verified payload, bad-tag
rejection, foreign/consumed input rejection, cancellation with worker quota
retained after terminal collection, and panic reporting after handle pruning.
The complete authentication-related filter now passes 26 native tests on Linux
x86-64. The separate real authentication guest test passes success, bad-tag,
and wrong-nonce cases with a ZIP exceeding 16 MiB, and asserts zero staging
bytes, jobs, resources and operations after teardown. It is explicitly ignored
without a freshly built `auth-proof` artifact; commands are in the guest README.
Native pending and completed-uncollected abandonment tests cover the new
scoped authentication-future Drop operation. No plaintext crosses this guest
proof: it authenticates and closes an opaque archive without reading entries.

Generated inventory/entry operations, entry-copy job accounting, progress deadlines, killable
worker execution, and current six-native runs still remain. This does not
interrupt blocked filesystem calls or claim process RSS bounds.

## Native portability gate

The existing six-host `wasm-tauri-screenshot-native` CI matrix now runs
`Run native authenticated archive staging proofs` after verifying that the
Rust host matches the matrix target. It explicitly enables
`wasm-sketch-host,archive-auth-test-support` and executes the staging and
authenticated resource registry tests on Linux,
macOS, and Windows, each on x86-64 and ARM64. The independent `each-feature`
matrix also checks this feature without relying on `--all-features`.

Matrix configuration alone is not six-platform success evidence. Record the
successful per-target steps before claiming portability. The native vendored
crypto build, anonymous-file lifecycle, and authenticated ZIP extraction
must succeed on each host; a cross-compilation or screenshot-only result
does not substitute for this step. These native tests still do not exercise
the generated guest archive API or complete the extension2 acceptance gate.

### Verified native registry baseline

[Run 34740972933](https://github.com/zackees/kernal-api/actions/runs/34740972933)
completed successfully at `e0216d4b622049f1751dc72dc0aac2b713a7192b`.
Each native job's log reports all eight authenticated tests passing, including
`authenticated_archive_registry_extracts_large_zip_with_bounded_transfers`.
This extends the earlier staging-only baseline to immutable storage quotas,
scoped registry ownership and the 17 MiB registry extraction fixture.

| Target | Native job | Eight-test time |
| --- | --- | --- |
| Linux x86-64 | [103680505435](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505435) | 0.27 s |
| Linux ARM64 | [103680505322](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505322) | 0.37 s |
| macOS x86-64 | [103680505367](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505367) | 2.02 s |
| macOS ARM64 | [103680505258](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505258) | 0.29 s |
| Windows x86-64 | [103680505245](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505245) | 0.54 s |
| Windows ARM64 | [103680505383](https://github.com/zackees/kernal-api/actions/runs/34740972933/job/103680505383) | 2.00 s |

The later pending-authentication operation, atomic finalization, owned reader,
and guest contract changes are not covered by this revision. In particular,
these eight tests are not a six-target execution of the guest archive API.

### Earlier verified native staging baseline

CI run [34740154679](https://github.com/zackees/kernal-api/actions/runs/34740154679)
at `978afc4e1e1b1c711ee9183add74631db24218b1` executed all four original
`archive::authenticated_staging` tests on every native target. Each job log
contains the individual passing tests and `4 passed; 0 failed; 0 ignored`.
The durations below are test-harness elapsed times, not build timings.

| Native target | Test time | Verified job |
| --- | ---: | --- |
| Linux x86-64 | 0.23 s | [103678392539](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392539) |
| Linux ARM64 | 0.38 s | [103678392560](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392560) |
| macOS x86-64 | 1.67 s | [103678392530](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392530) |
| macOS ARM64 | 0.34 s | [103678392535](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392535) |
| Windows x86-64 | 0.57 s | [103678392524](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392524) |
| Windows ARM64 | 1.69 s | [103678392568](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392568) |

This proves native execution of the NIST vector, quota/update cleanup, large
single-message mutation cases, and authenticated large-ZIP extraction at that
revision, including the vendored crypto build and anonymous-file path. It does
not validate the later immutable-budget, registry, or pending-operation changes,
which were absent from this source revision. Those require fresh native runs.

The overall workflow is **not green**: the separate Windows `rust-native` job
[103678392556](https://github.com/zackees/kernal-api/actions/runs/34740154679/job/103678392556)
failed `profile::tests::a_handful_of_threads_fills_a_small_ring_long_before_the_window_ends`.
It reported 4,514,326,200 ns against a four-second assertion, with 631 tests
passing and one failing. The profiling cause remains unproven; do not waive
that failure or present these six passing native jobs as full-PR validation.

`shared_staging_budget_has_one_ceiling_under_contention` holds concurrent
reservations behind barriers: eight producers each request eight bytes from
one sixteen-byte budget, and exactly two may retain reservations. It also
checks reuse after drop and overflow rejection at `u64::MAX`. These are
accounting-only reservations, not attempts to allocate or stage `u64::MAX`
bytes. A guest must never be allowed to construct replacement budgets, which
would bypass the registry's ceiling.
Removing the ceiling check made this regression fail with 64 retained bytes
against a sixteen-byte budget. Restoring the check is required for GREEN;
the test does not rely on thread scheduling or sleep intervals.

## Test-gated authenticated resource ownership

With `wasm-sketch-host,archive-auth-test-support`, the existing `OperationHub`
owns a staging budget and accepts authenticated files into its existing
generation-safe resource table. Registration accepts only the consuming
authentication result, rejects files charged to another budget, applies the
ordinary resource-count ceiling, and does not share the token between stores.
This adds no second registry and no production ABI opcode.

The focused registry tests check foreign-store/foreign-hub rejection, stale
tokens after slot reuse, foreign-budget and full/closed-hub rejection, and
storage release on close, extraction failure, and hub cancellation/trap/owner
teardown. Extraction consumes the resource under the hub lock and runs the
existing extractor after releasing that lock; the owned file keeps its charge
until extraction returns. Its destination is still a trusted native test path,
not a guest-authorized output capability.

```sh
soldr cargo test --locked --features wasm-sketch-host,archive-auth-test-support --lib authenticated_archive_registry
```

This integration is compiled only in tests. In-flight extraction is not
interrupted by hub teardown. The small NIST fixture exercises
ownership and extraction failure. The additional
`authenticated_archive_registry_extracts_large_zip_with_bounded_transfers`
fixture creates a stored ZIP with a 17 MiB entry, streams encryption and
authentication with 64 KiB buffers, registers the authenticated resource, and
extracts through its opaque token. It verifies every output byte, no live
resource/output before authentication, denial of another staging reservation
while the registered archive holds the full budget, and rejection of token
reuse after consumption. Bad-tag and per-entry-limit cases finish with zero
live resources and zero charged storage as well.

This is a synthetic native registry proof, not guest execution or a measured
RSS bound. Async cancellation, generated guest calls, real extension2 policy,
and six-native-target execution remain required.
Removing the budget-identity guard caused the rejection regression to return
a live token instead of `WrongRights` for a foreign-budget file. The guard
was restored before final validation. The six-native CI step now enables the
host feature and filters `authenticated_` to include both native test modules.

## Pending authentication operation ownership

The test-gated hub now reserves an ordinary operation slot before constructing
pending authentication, and stores that state in the existing operation table.
Failed construction removes the unpublished operation. A chunk update takes
exclusive ownership of pending state, performs crypto/file I/O outside the
hub mutex, and returns state only if the operation is still active. No readable
archive resource is created by beginning or updating authentication.

Native cancellation and terminal transitions drop idle pending state before
notifying the waiter. Hub trap, owner-exit, and timeout teardown do the same.
The focused tests check foreign-owner rejection, cancellation after a partial
update, failed update/construction cleanup, zero storage/resource counts, and
terminal collection. Omitting cancellation cleanup made its regression retain
sixteen bytes instead of zero; restoring cleanup fixes that failure.

This is not yet an end-to-end async guest capability. An in-flight native
write keeps its reservation until I/O returns, even
if cancellation wins meanwhile. No interruption of uninterruptible filesystem
work, worker-process cleanup, or progress-deadline execution is claimed.

## Final-tag operation handoff

Finalization now consumes pending state from the operation table and performs
tag verification, flush, and rewind outside the authority mutex. It then
rechecks that the operation remains active. Resource insertion, attachment to
the creating operation, activation, and terminal publication occur together
under the same lock, using the existing resource allocator and terminal logic.
Authentication or resource-quota failure publishes a resource-free rejection.
The consumer receives the archive token only by collecting the operation's
successful terminal result.

The 17 MiB registry ZIP fixture now uses this complete native operation path:
begin, bounded updates, final verification, terminal collection, and extraction.
The cancellation regression runs deterministically after successful verification
but before publication, both with and without collecting the cancelled terminal.
It starts with sixteen charged bytes and requires zero resources/storage after
the rejected handoff. A separate test verifies finalization's foreign-owner and
resource-quota checks, also with nonzero staged data.

All of this remains test-gated. It is not generated guest execution, an async
dispatcher, real extension2 policy, or proof that filesystem calls can be
interrupted. The latest implementation still needs its six-native-target runs.
