# Repository guidance

Read [AGENTS.md](AGENTS.md) first; it is the authoritative working agreement.

For a native-host capability, also read
[docs/platform-boundary.md](docs/platform-boundary.md). Keep the structured
host selector in `src/lib.rs`, keep concrete OS details private, and expose a
neutral facade. Do not use this file to create a second policy: #152 tracks
the remaining Dylint enforcement work.
