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

This is synchronous native prototype work, not an async operation or resource
registry integration. The accounting is a private shared counter supplied by
the test harness, not the final authoritative sketch quota. Cancellation is
currently owner drop; guest cancellation/worker teardown and progress deadlines
are still unimplemented. OS file cache/RSS and secure physical erasure of
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
