//! Process spawning, sessions, identity, exit observation, and signals.
//!
//! One linked test binary per category, not per file: each module below was
//! its own top-level integration test, and each one statically linked this
//! crate's whole graph. Test IDs are now `process::<module>::<test>`.
//! See AGENTS.md for the rule.

mod foreground_command;
mod interrupt_notification;
mod posix_arguments;
mod process_exit_observation;
mod process_identity;
mod process_owner_death_contract;
mod process_session;
mod process_session_control;
mod process_session_streaming;
mod process_target;
mod spawn_mode_facade;
mod windows_spawn_cleanup;
