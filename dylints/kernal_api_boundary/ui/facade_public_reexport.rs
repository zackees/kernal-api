// aux-build:running_process.rs
// aux-build:tokio.rs

// `running_process` is an owned backend like `tokio`: a public re-export from
// the facade owner leaks it whatever the public spelling. Compiled under the
// facade owner's crate name so the lint applies the owner rule.
#![allow(unused)]
#![crate_name = "kernal_api"]

extern crate running_process;
extern crate tokio;

pub use running_process::spawn;

/// The #258 shape: a renamed re-export is still the backend type.
pub use running_process::StreamKind as CaptureStream;

pub use tokio::sync::Mutex as Lock;

pub use tokio::task::*;

pub use running_process::{spawn as spawn_again, StreamKind as Stream};

pub mod process {
    pub use running_process::spawn as spawn_process;
}

// Internal imports, crate-private re-exports, and re-exports inside a module no
// client can reach publish nothing.
use running_process::spawn as internal_spawn;

pub(crate) use tokio::sync::Mutex as InternalLock;

mod private {
    pub use running_process::StreamKind;
}

fn main() {}
