# Client boundary lint

The suite contains two lints:

- `kernal_api_boundary` rejects direct client use of implementation crates.
- `kernal_api_platform_boundary` rejects host `cfg` selection and native OS
  APIs outside the shared HAL, including code elided on the CI host.

Add the repository lints to a client workspace:

```toml
[workspace.metadata.dylint]
libraries = [
  { git = "https://github.com/zackees/kernal-api", pattern = "dylints/*" },
]
```

Then run it through the client's Soldr toolchain front door:

```console
soldr cargo dylint --all --workspace -- --all-targets
```

## What the repository's own lint job covers

A Dylint pass only sees code the compiler compiles, and `default = []` here.
The `dylints` job in `.github/workflows/ci.yml` therefore runs both lints as

```console
soldr cargo dylint --all --workspace -- --all-features --all-targets
```

`--all-features` puts every gated module in front of the lint -- `crash`,
`wasm`, `symbolize`, `profile`, `snapshot`, `fs`, `fs-watch`, `ipc`, `pty`,
`tokio-console`, the daemon slices -- and `--all-targets` adds the integration
tests and the two `required-features` worker binaries. A green `dylints` job
means the whole crate is clean, not just the ungated core. The job runs on
`ubuntu-latest`, so `cfg(windows)` and `cfg(target_os = "macos")` bodies remain
unlinted by it; `tests/facade_policy.rs` scans those as text regardless of
host. Narrowing the feature set is a coverage decision, not a knob: if it is
ever narrowed, say here exactly which features remain covered.

The lint checks both the client manifest and resolved Rust code. An unused,
aliased, target-specific, build, or test dependency on a facade-owned backend
is rejected before it can create a duplicate compile unit; method calls,
function items, imports, and public type references are also resolved to the
underlying crate and rejected. Client APIs must expose `kernal_api`-owned
facade types instead.

Inside `kernal-api` the same lint applies the complementary rule. A backend may
be used privately there, but it may not appear in a public type position: the
payload of a public enum variant, a public field, a function parameter or
return type, a type alias, a const or static, or a bound. A backend named in
one of those is vocabulary a client has to speak in order to match on or call
the item, whether or not it is re-exported, so a `pub use` grep does not see
it. A private field of a public newtype, a private item, and a trait
implementation that adapts a facade type into a backend one all remain legal.

The same coupling arrives from the other direction when this crate implements
a backend's trait for one of its own exported types. `impl
tokio::io::AsyncRead for IpcAsyncStream` names no backend in any signature,
yet a client cannot call through it without `use tokio::io::AsyncRead`, so the
backend is back in the application's import list. The rule splits that class
in two.

Implementing the async mirrors of `std::io::{Read, Write, Seek, BufRead}` --
`tokio::io::AsyncRead`, `AsyncWrite`, `AsyncSeek`, and `AsyncBufRead` -- is
allowed, and is what a usable stream facade is for. Those four are shared
byte-stream vocabulary rather than backend design: the impl says "this is a
byte stream", not "this is an interprocess socket", and the ecosystem's
combinators, codecs, and protocol crates are all written against them. A
facade-owned substitute would interoperate with none of them, so refusing the
impl would make the facade's stream types useless in the one position they
exist to fill. The facade still owes a caller an inherent method for the
ordinary operations, so that reaching a stream's `read` or `write_all` needs
no extension-trait import; the trait implementations are for handing the
stream to someone else's generic code.

Implementing any other owned-crate trait for an exported type is rejected.
Those are backend extension points -- `framehop::ModuleSectionInfo`,
`notify::Watcher` -- and implementing one for a type a client can name
publishes the backend's design as this crate's contract, which is the drift
the boundary exists to prevent. Two shapes stay outside the rule because they
publish nothing. A backend trait implemented for a private type is
unreachable, since no client can name the type it is attached to. A
facade-owned trait implemented for a backend type -- the adapter direction --
is reachable only by a caller that already holds the backend type, so it
imposes no vocabulary on one that does not.

`running-process` is classified as an owned implementation dependency for the
target architecture. It is allowed inside `kernal-api`; phase 1 will add the
private adapter. It is denied in each first-party application once that
application's required process and broker facade is available.
`running-process` must never depend in the opposite direction.

Adoption is a capability-by-capability ratchet, not a flag-day waiver. Land a
facade with behavior and compatibility tests, migrate the client call sites,
then enable the strict ban for that client workspace. Temporary migration
branches may carry both dependencies, but released code has no alternate
runtime, network stack, process layer, or broker path behind a fallback.

The `kernal_api::hash` BLAKE3 facade landed in #8 before any first-party
client has migrated its content-hashing call sites. `blake3` is therefore not
yet in the owned implementation set: enabling that direct-dependency ban now
would block a client before its migration branch can consume the facade. Add
it with normal, aliased, target, build, and test dependency fixtures when the
first such client migration is ready; its released branch must not retain the
legacy direct dependency.
