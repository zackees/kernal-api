# Repository guidance

Read [AGENTS.md](AGENTS.md) first; it is the authoritative working agreement.

For a native-host capability, also read
[docs/platform-boundary.md](docs/platform-boundary.md). Keep the structured
host selector in `src/lib.rs`, keep concrete OS details private, and expose a
neutral facade. Do not use this file to create a second policy: #152 tracks
the remaining Dylint enforcement work.

# Coordinated dependency sub-git workflow

Use `_vender/<name>/` only for short-lived cross-repository integration.

1. Keep the nested checkout as its own Git repository; do not commit its files
   from `kernal-api`.
2. Point the local `kernal-api` dependency at the nested crate only while
   integrating an unpublished API. Treat this as intentionally release-blocked.
3. Implement and validate the dependency in its own repository, publish its
   public release, and wait until that exact version resolves from the registry.
4. Replace the local path with that exact public version, update the lockfile,
   remove the nested checkout, and then run the normal local/package validation.
5. Never retain a path patch, Git source, `0.0.0` reservation, or an unverified
   registry version on a release branch.
