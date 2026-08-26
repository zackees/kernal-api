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

The lint checks both the client manifest and resolved Rust code. An unused,
aliased, target-specific, build, or test dependency on a facade-owned backend
is rejected before it can create a duplicate compile unit; method calls,
function items, imports, and public type references are also resolved to the
underlying crate and rejected. Client APIs must expose `kernal_api`-owned
facade types instead.

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
