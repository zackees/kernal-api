//! Canonical contained-process-group ownership primitives.
//!
//! These direct aliases preserve fbuild's established synchronous containment
//! and originator identity contract without recreating a second process group.

pub use running_process::{ContainedProcessGroup, SpawnedChild, ORIGINATOR_ENV_VAR};
