# Architecture

This document is the canonical dependency and ownership design for
`kernal-api`. Earlier proposals that placed `running-process` above the facade
or required a particular crate-packaging scheme are superseded by this design.

## Dependency direction

```text
zccache / soldr / fbuild
          |
          v
      kernal-api
          |
          v
   running-process
          |
          v
 Tokio and native operating-system APIs
```

`running-process` is the trusted native substrate. It already owns substantial
cross-platform process, lifecycle, and broker implementation code, and it must
have an async runtime to do that work. It must never depend on `kernal-api`.

`kernal-api` is the higher semantic facade used by applications. In the target
architecture it depends on `running-process`, selects shared implementation
versions, adds facilities such as hashing and profiling, and turns backend
behavior into stable application contracts. The current implementation has not
yet landed that dependency; phase 1 below establishes it. First-party
applications eventually depend on `kernal-api` only.

This one-way graph resolves the async/process cycle without creating a smaller
"base" facade that would merely move the same boundary elsewhere.

## Boundary rules

- `running-process` owns low-level process, OS, and current broker mechanisms.
- `kernal-api` owns public semantic types, policies, defaults, and capability
  composition. It may adapt `running-process` privately, but does not publicly
  re-export its types.
- Applications own product policy and product protocols. zccache, for example,
  keeps its cache payload schema, protocol identifiers, and deployment policy.
- Applications may not directly depend on `running-process`, Tokio, or another
  implementation for a capability that `kernal-api` provides.
- New facade operations bake in safe defaults: bounded resources, cancellation,
  connection and progress timeouts, child cleanup, and diagnostic visibility.

The public API describes intent rather than backend vocabulary. This allows a
backend to be trimmed, vendored, or rewritten without changing every client.

## Migration sequence

1. Fill the facade gaps needed by zccache: cancellation/runtime handles,
   process lifecycle and bounded execution, BLAKE3 hashing, and broker adapters.
2. Rebase zccache's embedded API on facade-owned async and cancellation types.
3. Move zccache process launch, probing, detached deployment, identity, and
   hashing behind `kernal-api`.
4. Move broker access through a compatibility facade while preserving the
   existing frame bytes, version negotiation, endpoint behavior, and number of
   process-to-process round trips.
5. Remove every direct zccache `running-process` dependency and import, then
   enable the strict boundary Dylint for the whole workspace.
6. Apply the proven migration to Soldr and fbuild.

During migration, each capability lands in `kernal-api` before the corresponding
client ban is enabled. There is no permanent legacy fallback in release builds.

## Broker direction

The broker daemon implementation remains in `running-process` during the first
migrations. This avoids combining an architectural cleanup with a wire or
lifecycle rewrite.

After zccache, Soldr, and fbuild consume the facade successfully, the generic
broker-daemon pattern should move up into `kernal-api` as a managed-service
capability. That later project must preserve compatibility and keep application
payloads outside the generic layer. It is explicitly not a phase-1 prerequisite.

## Source and package organization

One supported public API does not require one Rust compilation unit. Public
surface stability and physical crate boundaries are separate decisions.

Start with cohesive, feature-gated modules. Split private implementation crates
only when reproducible `soldr cargo` timings show a material win in clean build,
incremental rebuild, cache reuse, or feature isolation. A release amalgamation
is also an optimization to prove, not a permanent architectural requirement.
Optional features must omit their dependency trees when disabled.

Do not create speculative abstraction crates or traits such as a generic
`kernal-api-data` layer without multiple real implementations. Initially,
hashing, mapped files, file locks, databases, networking, HTTP services, and GUI
hosting belong in capability modules with facade-owned concrete contracts.
