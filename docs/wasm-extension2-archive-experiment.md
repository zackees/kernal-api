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
