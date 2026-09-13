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
