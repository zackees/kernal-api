//! Off-process symbolization for native captures.
//!
//! # Why this is a separate process
//!
//! Symbol files are attacker-adjacent input: a PDB, DWARF section, or minidump
//! can be malformed in ways that crash a parser outright rather than returning
//! an error. Isolation here is therefore a **process** boundary, not a
//! `catch_unwind` — a parser that segfaults takes down only this PID, and the
//! daemon observes the exit status and carries on.
//!
//! Isolation is link-time as well as run-time. This library contains only the
//! stable wire schema and the worker client. Parser modules are compiled only
//! into the `kernal-symbolize` executable behind the `symbolize-worker`
//! feature; dependent applications neither compile nor link them.
//!
//! # Never fabricate a symbol
//!
//! Every degradation path preserves the module and offset and reports a
//! [`wire::FrameStatus`] saying how far resolution got. A wrong function name
//! is worse than no name: it sends whoever reads the report looking in the
//! wrong place, and nothing in the output would contradict them.

#![deny(missing_docs)]

mod client;
pub mod wire;

pub use client::{default_worker_path, SymbolizerWorker, WorkerError, SYMBOLIZER_WORKER_ENV};
pub use wire::{RawCapture, SymbolReport};
