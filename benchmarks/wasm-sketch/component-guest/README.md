# Private Component Model guest probe

Issue #13 requires a Component Model comparison before selecting a production
binding design. This unpublished, separate workspace tests the toolchain first;
it is not a second runtime backend or a public guest API. Neither this workspace
nor its tools are dependencies of `kernal-api`.

The pinned `wit-bindgen = 0.58.0` generates a private custom world with one
kernel interface: a granted blob resource exposes a typed `stream<u8>`.
The async export reads with one reusable 64 KiB buffer, counts up to 64 MiB,
and returns an error on cancellation or overflow. Grant and read methods are
explicitly async in the WIT. Resource and stream handles are dropped on return.
The sibling tool's opt-in execution probe has exercised normal transfer and a
forced producer trap on Linux x86-64, plus store teardown after an observed
pending host-call cancellation. Guest-issued operation cancellation and the
full lifecycle contract are not yet proven.

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
Wasmtime 45.0.0. Its later `execution-probe` links the exact generated world and
executes the rebuilt async-import component through `kernal_api::async_engine`.

Remaining work includes adapting the same public Rust guest facade, deterministic
checked-in bindings, broader host execution through the existing kernel executor,
backpressure and cancellation/teardown tests, ABI/capability version
negotiation, ten edit measurements, and six native target validation. Do not
count this small toolchain probe as the representative application comparison.
