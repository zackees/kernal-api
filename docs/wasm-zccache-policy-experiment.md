# zccache policy sketch experiment — issue #13

## Reproduced native dependency barrier

Inspected zccache revision `a7c84de53105ce41b2060bd9dd7730ef226e78a1`
(workspace version 1.13.22) in the sister checkout
`../kernal-api-extern/zccache`. No upstream source changes were made during
this initial reproduction; coordinated prerequisites are recorded below.
On the Linux x86-64 reference host, using its pinned Rust 1.95.0 toolchain:

```sh
soldr --no-cache rustup target add wasm32-unknown-unknown
soldr cargo tree --locked -p zccache-compiler --target wasm32-unknown-unknown -i mio
soldr cargo check --locked -p zccache-compiler --target wasm32-unknown-unknown -j 1
```

The dependency query succeeds and reports:

```text
mio v1.1.1
└── tokio v1.50.0
    └── zccache-platform v1.13.22
        ├── zccache-compiler v1.13.22
        └── zccache-core v1.13.22
            └── zccache-compiler v1.13.22
```

The check exits 101 in Mio, with the primary diagnostic:

```text
This wasm target is unsupported by mio. If using Tokio, disable the net feature.
```

There are 49 Mio compilation errors; the primary diagnostic is at
`mio-1.1.1/src/lib.rs:44`. The local Soldr build record is
`20260913T024540Z-home-niteris-dev-kernal-api-extern-zccache.xml` under
`/home/niteris/.soldr/logs/builds/`. This is the required native-policy-path
RED evidence, not a successful guest build or proof that removing one
dependency makes the compiler crate portable.

An earlier attempt failed with E0463 because the Wasm target was absent.
That attempt is excluded from the portability result. Installing the target
and rerunning reached Mio's explicit unsupported-target diagnostic.

## Source constraints for the extraction

The actual source, rather than the architecture summary, establishes these
requirements for native/Wasm fixture equivalence:

- `crates/zccache-compiler/src/detect.rs` performs string-based compiler-family
  classification, including Windows-style paths, clang-cl, Emscripten, and
  Dylint drivers. Reuse these cases rather than inventing a toy classifier.
- `crates/zccache-compiler/src/parse.rs` uses `NormalizedPath` and consults
  `platform::host::is_windows()` for output policy. A guest's own compilation
  target is not the host fact that this policy needs.
- `crates/zccache-core/src/path.rs` has host-dependent normalization and key
  semantics. Replacing it with the guest standard library's path behavior
  would not prove native/Wasm equality on Windows and macOS.
- `crates/zccache-hash/src/cache_key.rs` uses the domain
  `zccache-cache-key-v2\0`, preserves argument order, and sorts environment
  and dependency maps. The checkout's architecture guidance still describes
  sorted arguments and a v1 tag; that prose must not define the experiment's
  expected keys. Kernel hashing must preserve the actual byte-update sequence.

## Explicit Rustc host-policy prerequisite

The coordinated sister branch `feat/rustc-explicit-host-policy` contains
[zccache commit 4d37833](https://github.com/zackees/zccache/commit/4d378335db604ea340e204fa50d513197415009d).
It adds `RustcHost` and `parse_rustc_invocation_with_host` to the existing
compiler crate, without copying its parser into this benchmark. The native
entry point still resolves the existing host/configuration facts and delegates
to that implementation. Host-side proc-macro/Dylint names are independent of
the requested target; an explicit target still controls executable naming.

RED: the three new tests fail with unresolved imports before the seam exists
(Soldr record `20260913T085516Z-home-niteris-dev-kernal-api-extern-zccache.xml`).
GREEN on Linux x86-64: all three explicit-host tests pass, exercising all three
host families, Dylint/test-cache admission, and original argv preservation.
The complete compiler suite passes 383 tests with no failures or ignored tests;
strict package Clippy and an independent focused Astra review also pass:

```sh
soldr --no-cache cargo test --locked -p zccache-compiler -j1
soldr --no-cache cargo clippy --locked -p zccache-compiler --all-targets -j1 -- --deny warnings
```

This prerequisite is tracked in [zccache #1580](https://github.com/zackees/zccache/pull/1580)
and merged as `348e175aa4be339ebc666bb2be4dc3dd7993af60`. After integrating
the shared encoder prerequisite, the combined compiler suite again passed
383 tests and strict all-target compiler Clippy passed. It does not remove the native
dependency graph or change `NormalizedPath`/lexical path semantics. These
results are native parser evidence, not an actual Wasm parser/key/miss proof.

## Shared request-key encoder prerequisite

[zccache #1581](https://github.com/zackees/zccache/pull/1581), commit
`c54cf8aecd9a508cd090fa32ef9e1f5569eeaa05`, extracts the actual daemon
`zccache-request-v2` byte encoding into the existing hash crate. The native
daemon now consumes that same fallible emitter with lazy argument normalization.
It preserves ordered argv, detached remap handling, raw depfile salts, and
selected sorted environment entries without collecting a whole-key buffer.
This request-cache fingerprint is not the complete artifact key described above.

Literal-byte compatibility fixtures passed against the original daemon encoder.
The new emitter tests first failed with E0432, then passed after implementation.
Linux validation: 23 hash tests, 29 daemon fingerprint/path-policy tests, and
the full daemon suite (837 passed, 28 existing ignored integration tests,
139.73 seconds). Strict all-target Clippy for the changed crates passed with
`--no-deps`; broader dependency linting stopped on three existing
`double_must_use` diagnostics in untouched protocol code. Focused formatting
and independent Astra code/documentation review passed.

The PR merged as `e1dc9f27e931300f7ba17b76ee9ffa2073abe72d`; an authoritative
PR read confirmed the merge after the command returned a transient API error.
The sink boundary is ready for a kernel hash adapter, but neither
that adapter nor portable path normalization or the Wasm dependency graph is
implemented by this prerequisite. Neither PR changes a release version.

## Bounded public guest hash control

Operation protocol revision 6 adds a facade-owned `guest::Blake3Hasher`, backed
by the kernel's existing BLAKE3 implementation. Updates are bounded to 64 KiB,
resources are store-scoped, and operation capacity is reserved before mutation.
Dropping an uncollected create reclaims its resource; abandoning an update
revokes uncertain state without replay or rollback. Finalize consumes the hash
only after validating its complete fixed-size destination.

The `kernal-hash-guest-proof` binary in the extension2 benchmark workspace is
a separate capability control, not extension2 policy. On Linux x86-64, a freshly
compiled revision-6 guest checked the empty-input digest and hashed 64 MiB
of `0x5a` twice, in 65,536-byte and
4,093-byte chunks, through the public facade. Both digests matched the native
kernel and independent `b3sum` literal. The actual guest test passed in 4.57 s,
with zero live resources and pending operations after execution and execution
limits returned to their defaults after root closure. This is correctness
evidence, not a benchmark or aggregate-memory measurement.

The operation regression suite passed 79 tests; the independent focused hash
review run passed 12 tests. Strict host Clippy, generated-ABI drift checking,
and Astra lifecycle review passed. The hash proof is opt-in: build
`kernal-hash-guest-proof` with `guest-proof` for `wasm32-wasip1-threads`, copy
the freshly built module and embed metadata using the ABI generator, then set
`KERNAL_HASH_GUEST_WASM` to that admitted copy when running the ignored
`hash_actual_guest_streams_64_mib_through_public_facade` host test.

This control does not yet connect the shared zccache request encoder to the
guest, remove its native dependency graph, or establish the same-facade
Component comparison. Revision-5 archive and screenshot runs cannot establish
revision-6 compatibility; those artifacts must be rebuilt and rerun.

## Remaining proof

No extracted policy package or GREEN result is claimed yet. The next
implementation must reuse representative parser/key fixtures, supply host
facts explicitly, and route hashing and the controlled compiler miss through
public kernel capabilities with bounded stdout/stderr. Native and Wasm runs
must compare the same policy outputs. Daemon, watcher, IPC, and artifact
movement stay native.

This experiment does not replace the Component Model comparison, ten-edit
latency measurements, sealed extension2 archive proof, or six native target
acceptance required by #13.
