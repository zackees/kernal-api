# Native platform boundary

This is the authoring guide for native-host platform code in `kernal-api`.
It describes the intended boundary; the remaining mechanical Dylint
enforcement is tracked in [#152](https://github.com/zackees/kernal-api/issues/152).

## One structured selector

`src/lib.rs` owns the only native-host selector. Its `std::cfg_select!` block
maps Windows, Linux, and macOS to private concrete trees and gives the selected
tree the private `platform_imp` name. The crate root then exports semantic,
platform-neutral operations. Capability modules consume those operations; they
do not select an OS themselves.

```rust
// src/lib.rs: the one native-host selection point
std::cfg_select! {
    target_os = "windows" => { mod platform_win; pub(crate) use platform_win as platform_imp; }
    target_os = "linux" => { mod platform_linux; pub(crate) use platform_linux as platform_imp; }
    target_os = "macos" => { mod platform_macos; pub(crate) use platform_macos as platform_imp; }
}
```

Concrete OS implementation belongs under the selected platform trees. It may
use native APIs there, but it must export a semantic operation that works for
every supported host. If a capability is unsupported, model that explicitly
in the facade contract instead of spreading OS checks through callers.

## Authoring rules

- In ordinary `src/` modules, use the crate-root facade. Do not add
  `#[cfg(target_os = ...)]`, `cfg!(target_os = ...)`, `std::os::*`, native OS
  crates, `platform_imp`, or a concrete platform module.
- Host-neutral feature gates and test configuration remain normal Rust tools.
  They are not an exception for choosing a native implementation.
- WebAssembly/guest target selection is not native-host selection. Keep those
  branches conceptually and structurally separate.
- Add a native capability by defining its semantic facade operation,
  implementing it in each concrete platform tree, then exposing it once from
  the crate root. Preserve Linux, macOS, and Windows on x86-64 and ARM64.

For example, a capability module should call a neutral operation:

```rust
let shell = crate::shell_spec(command);
```

It should not select a concrete operating system:

```rust,ignore
#[cfg(target_os = "linux")]
use std::os::unix::process::CommandExt;
```

## Validation and current limitation

Run the repository lint through the Soldr front door:

```console
soldr cargo dylint --all --workspace -- --all-features --all-targets
```

The current `kernal_api_platform_boundary` Dylint protects clients but
intentionally bypasses the `kernal-api` package, and its CI execution is on
Linux. It is therefore not yet sufficient to prove owner-source compliance.
Issue #152 will extend the lint across active and inactive owner source with
AST/pre-expansion coverage, establish an explicit temporary occurrence
baseline, and reduce that baseline to zero. Until it lands, inspect changes
against this guide and the root selector manually.
