# Sketch candidate measurement

Issue #13 requires both binding candidates, at least ten warm one-line edits
per candidate, and a correctness-qualified selection. No comparative result
or latency gate is claimed yet.

The native admission-only example gives the core-Wasm candidate a measurement
boundary without running the screenshot application or guest entry point:

```sh
soldr cargo build --locked --release --features wasm-sketch-host --example wasm-admission
target/release/examples/wasm-admission /absolute/path/to/guest.admitted.wasm
```

Use the executable path reported by Cargo if the target directory is overridden.
Input must already contain the generated ABI metadata. The example applies the
public `threaded_rust_v1` policy with a 32 MiB upload bound and the guest's
16,384-page maximum shared-memory contract. It reports one JSON record only
after admission succeeds. Malformed modules, policy violations, and compiler
failures exit unsuccessfully rather than contributing a timing sample.

`compiler_setup_ns` measures kernel compiler construction. `admission_ns`
measures public admission, including preflight and native engine compilation.
File reading and process startup are excluded from these internal measurements;
the end-to-end edit measurement must include its entire command wall time.
`module_bytes` is the actual admitted input length. `executed: false` means
the command does not instantiate or execute the module, not that the module
has passed lifecycle correctness tests.

Keep guest compilation, metadata embedding, admission, and execution timings
distinct. A successful admission measurement is not an execution proof, a
64 MiB stream measurement, or a Component Model result. Before selection, record
cold/no-op/edit wall times, p50/p95, cache evidence, peak compiler RSS, module
size, and the native/guest fixture equality and resource-lifecycle results
specified in #13. Do not infer cache hits from short elapsed times or count a
failed build as a fast sample.

Initial smoke check on the Linux x86-64 development host admitted the actual
threaded fixture (164,824 bytes), reporting 254,954 ns compiler setup and
3,590,573,627 ns admission. This was one debug-host run through `soldr cargo run`,
not an optimized reference-host benchmark or an edit-loop result. It establishes
that the command reaches real admission and emits a result; it does not establish
the <=2-second p50 gate. Both bounded-upload and invalid-input tests pass, as
do strict example Clippy and formatting checks.
