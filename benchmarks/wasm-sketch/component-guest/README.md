# Private Component Model guest probe

Issue #13 requires a Component Model comparison before selecting a production
binding design. This unpublished, separate workspace tests the toolchain first;
it is not a second runtime backend or a public guest API. Neither this workspace
nor its tools are dependencies of `kernal-api`.

The pinned `wit-bindgen = 0.58.0` generates a private custom world with one
kernel interface: a granted blob resource exposes a typed `stream<u8>`.
The async export reads with one reusable 64 KiB buffer, counts up to 64 MiB,
and returns an error on cancellation or overflow. Resource and stream handles
are dropped on return. These are source properties, not a proven host lifecycle
contract: no component host has executed this probe yet.

From the repository root, on the pinned Rust 1.95.0 toolchain:

```sh
soldr --no-cache cargo build --locked \
  --manifest-path benchmarks/wasm-sketch/component-guest/Cargo.toml \
  --target wasm32-unknown-unknown --release \
  --target-dir benchmarks/wasm-sketch/component-guest/target -j1
```

The target must be installed using `soldr --no-cache rustup target add
wasm32-unknown-unknown`. The output is still a core module with component type
metadata; use the sibling [component-tools](../component-tools/README.md) to
encode and structurally validate the final component. No WASI adapter is used.

The [upstream Rust guide](https://component-model.bytecodealliance.org/language-support/creating-runnable-components/rust.html)
describes a newer linker requirement for its WASI-based async example. This
probe instead compiled successfully on Rust 1.95.0 for the non-ambient target
and encoded with matching `wit-component = 0.251.0`. This establishes that
the documented WASI linker issue does not prevent this separate encoding path;
it does not prove runtime execution compatibility. The sibling encoder's
optional `engine-probe` has subsequently compiled the actual component with
Wasmtime 45.0.0; instantiation and execution remain unproven.

Remaining work includes adapting the same public Rust guest facade, deterministic
checked-in bindings, host execution through the existing kernel executor,
bounded host production and cancellation/teardown tests, exact world/version
negotiation, ten edit measurements, and six native target validation. Do not
count this small toolchain probe as the representative application comparison.
