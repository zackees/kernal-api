# kernal-api

`kernal-api` is the future shared async systems foundation for Soldr,
zccache, running-process, and fbuild. It will provide a portable OS/process
HAL together with allocator instrumentation, memory snapshots, pprof capture,
symbolization, and Tokio runtime diagnostics for Rust and Python consumers.

## Reservation release

Version `0.0.0` exists only to reserve the `kernal-api` name on crates.io and
PyPI. It contains no usable API and is yanked from dependency resolution.
Do not depend on it. The first usable release will have an explicit platform,
MSRV, Python-version, async-runtime, and feature compatibility contract.

## License

Licensed under either Apache License, Version 2.0 or the MIT license, at your
option.

