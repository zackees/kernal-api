# zccache policy sketch experiment — issue #13

## Reproduced native dependency barrier

Inspected zccache revision `a7c84de53105ce41b2060bd9dd7730ef226e78a1`
(workspace version 1.13.22) in the sister checkout
`../kernal-api-extern/zccache`. No upstream source changes were made.
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
