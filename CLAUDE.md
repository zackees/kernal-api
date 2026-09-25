# Repository guidance

Read [AGENTS.md](AGENTS.md) first; it is the authoritative working agreement.

For a native-host capability, also read
[docs/platform-boundary.md](docs/platform-boundary.md). Keep the structured
host selector in `src/lib.rs`, keep concrete OS details private, and expose a
neutral facade. The `kernal_api_platform_boundary` Dylint enforces that
boundary on this crate; do not use this file to create a second policy.
