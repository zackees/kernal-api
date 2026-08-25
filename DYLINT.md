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
