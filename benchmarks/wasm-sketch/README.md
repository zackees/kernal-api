# Sketch candidate measurement

Issue #13 requires both binding candidates, at least ten warm one-line edits
per candidate, and a correctness-qualified selection. No comparative result
or latency gate is claimed yet.

The private [Component Model guest probe](component-guest/README.md) and
[encoder](component-tools/README.md) establish a non-WASI Rust 1.95.0 build
and structural encoding path. The optional execution probe also verifies a
64 MiB stream, forced-producer-trap cleanup, and teardown after cancellation of
an observed pending host call, plus guest-issued read cancellation and
same-instance reuse and a fast-producer/slow-consumer pause-and-resume check
on Linux x86-64 through the
kernel executor. This is not yet the representative candidate behind the public
guest facade or a comparative measurement.

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

The first optimized-host build did not complete: Cranelift's compiler received
SIGTERM. Soldr's record
`20260913T025516Z-home-niteris-dev-kernal-api.xml` reports zero cgroup OOM kills;
the signal's sender is not established. The failed build also pruned intermediate
metadata, and a concurrent Clippy rerun failed on a missing `.rmeta` file.
These failed commands are not performance samples. The no-cache Clippy retry
in a separate target directory passed in 176.24s with warnings denied; no
optimized-host timing is available yet. All three example tests and the
single-reviewer check pass.

The separate no-cache optimized-host retry subsequently completed in 460.24s.
That is the build time of the native measurement executable and its dependencies,
not a guest cold-build or edit-loop sample. The optimized-host edit run is
recorded separately from the debug diagnostic below.

### Optional build-memory measurement

Pass `--gnu-time /usr/bin/time` (or the installed GNU time path) to retain
`<sample>-build.rss-kib` alongside each build log. The runner verifies GNU time,
uses its `%M` measurement, converts KiB to bytes, and rejects absent, malformed,
or zero readings. BSD time and shell builtins are not supported by this option;
without it, the runner remains usable without this additional dependency.

`build_peak_rss_bytes` and `build_memory.peak_rss_bytes` describe GNU time's
build-command high-water mark, not a sum of concurrently resident processes,
an isolated rustc allocation profile, or native admission-engine memory.
Detached daemon processes are outside GNU time's accounting; in particular,
do not infer that the number captures all Soldr broker/daemon memory.
The compiler-specific field remains null: this command-level diagnostic does
not by itself discharge the peak-compiler-memory acceptance requirement.
The timing wrapper is included in end-to-end wall time, so keep instrumented
results separate from the original uninstrumented diagnostics. Caching remains
explicitly disabled; no cache-hit percentage is inferred or fabricated.

## Isolated core edit runner

```sh
uv run --no-project benchmarks/wasm-sketch/measure_core.py \
  --output /absolute/new/result-directory \
  --admission /absolute/path/to/wasm-admission \
  --embedder /absolute/path/to/kernal-api-wasm-abi-generator
uv run --no-project -m unittest discover -s benchmarks/wasm-sketch -p 'test_*.py'
```

The output directory must not exist. The runner retains a `git archive HEAD`
source snapshot there, then measures a fresh guest target directory, a no-op,
and ten real one-line clock-argument edits. Uncommitted source changes are not
part of the snapshot. It never edits the original guest or deletes old results.
Each sample includes guest compilation through Soldr, metadata embedding, and
admission. Logs and module/source hashes are retained. A failed command aborts
without producing a complete summary. Python 3.10+, Git, tar, Soldr, and the
installed Rust Wasm target are prerequisites; both helper executables must be
built beforehand, and their hashes are recorded.

This first runner is explicitly diagnostic: it disables Soldr caching while
retaining Cargo incremental build state, does not collect compiler RSS or cache
hit rate, and does not run the Component Model candidate. Its `cold` label means
an empty guest target directory, not an empty toolchain/registry or cold host
compiler. A completed result therefore cannot by itself satisfy #13's comparison
or selection gate. Use an optimized admission executable for performance work;
a debug-host run only verifies the measurement pipeline.

The first complete diagnostic is retained in
[results/core-debug-diagnostic.json](results/core-debug-diagnostic.json): all ten
edits changed both source and admitted-module hashes and passed actual admission.
End-to-end edit p50 was 5.072s and p95 5.488s with a debug admission host, while
an unrelated optimized admission-host build was active. This is not a quiet
reference-host benchmark, does not pass the <=2s gate, and does not select a
binding candidate. Full logs and the isolated source snapshot remain at the
`evidence_directory` recorded in the result.

The optimized-host run is in
[results/core-release-diagnostic.json](results/core-release-diagnostic.json).
All ten edits produced distinct source and module hashes and passed admission.
End-to-end edit p50 was 1.380s and p95 1.415s; the fresh guest target-directory
sample was 17.198s and the no-op 0.608s. The measured edit p50 is below 2s,
but this remains diagnostic evidence on a shared development host, with caching
disabled and compiler RSS unmeasured. It does not establish the full #13 gate,
the Component Model comparison, fixture correctness, or a go/no-go selection.

The separate instrumented run is retained in
[results/core-release-rss-diagnostic.json](results/core-release-rss-diagnostic.json).
Ten distinct edits passed admission, with p50 1.372s and p95 1.391s.
The largest GNU time build-command reading was 264,003,584 bytes (251.77 MiB),
on the fresh-target sample. This is the scoped diagnostic described above,
not complete compiler/process-tree memory accounting or binding selection.

## Isolated component edit runner

Build `component-tools` with only `--features engine-probe` (not
`execution-probe`), then run:

```sh
uv run --no-project benchmarks/wasm-sketch/measure_component.py \
  --output /absolute/new/component-result-directory \
  --encoder /absolute/path/to/kernal-component-tools
```

This runner takes a committed source snapshot and an empty guest target
directory, then measures cold/no-op/ten one-line edits. Each edit changes the
consumer's runtime byte ceiling by one MiB, starting at 64 MiB; it is not a
comment-only edit. Both source and resulting component hashes must differ
across all ten edits. Every sample includes Soldr guest compilation, component
encoding, structural/import validation, and actual Wasmtime 45 compilation.
The encoder's two success lines and exact output size are checked, so an
encoding-only executable cannot silently count as engine validation.

The runner retains each component and command log and refuses an existing
output directory. As with the core runner, cold means a fresh guest target,
not a fresh registry/toolchain. Python 3.10+, Git, tar, Soldr, and the installed
`wasm32-unknown-unknown` target are prerequisites. Build the encoder in release
mode for performance conclusions; debug mode only diagnoses the pipeline.

This is still the private Component Model probe, not the same public facade
as the core candidate. It regenerates bindings during compilation and does
not measure instantiation, execution, cache hit rate, or compiler memory.
Do not treat these diagnostic timings as the controlled reference-host gate
or a binding selection. The existing execution tests separately cover the
unchanged 64 MiB transfer workload.

The first complete run is retained in
[results/component-debug-diagnostic.json](results/component-debug-diagnostic.json).
Ten distinct compiled components measured 2.058s p50 and 2.359s p95 end-to-end;
the fresh-target sample was 28.344s and the no-op 1.339s. Every component was
71,378 bytes. This used a debug encoder/engine host on the shared development
machine: it neither meets the <=2s timing threshold in this run nor establishes
a release-mode or reference-host result. Raw logs, components, and the isolated
source remain at `/tmp/kernal-component-measure-c1df497`.

The optimized-host run is retained in
[results/component-release-diagnostic.json](results/component-release-diagnostic.json).
All ten edits produced distinct, validated 71,378-byte components: p50 1.157s,
p95 1.200s, fresh-target 27.897s, and no-op 0.446s. The encoder was built with
`--release --features engine-probe`; the guest snapshot was `ff7965e`.
A native combined-feature check was active concurrently. This improves the
diagnostic latency evidence but is still not a quiet reference-host run,
same-facade comparison, full correctness gate, or candidate selection.
Raw evidence remains at `/tmp/kernal-component-release-measure-ff7965e`.
