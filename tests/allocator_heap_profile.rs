//! Proves `kernal_api::allocator::dump_to_vec` against a real sampled
//! allocation, the way zccache's `heap_profile_test.rs` proved
//! `mimalloc_pprof::prof::dump_proto_to_vec` before this facade existed.
//!
//! An integration test is a separate final executable, so its allocator
//! declaration also guards the documented downstream linkage pattern: the
//! embedding binary declares `#[global_allocator]`, this crate supplies the
//! type and the profiler controls.

#![cfg(feature = "allocator")]

use kernal_api::allocator::{dump_to_vec, is_enabled, live_sample_count, start, stop, Allocator};

#[global_allocator]
static GLOBAL: Allocator = Allocator::new();

#[test]
fn dump_to_vec_captures_a_sampled_allocation() {
    if is_enabled() {
        stop();
    }
    assert!(start(1), "heap profiler should start exactly once");

    let retained = vec![0x5a_u8; 1024 * 1024];
    std::hint::black_box(&retained);

    assert!(
        live_sample_count() > 0,
        "retained allocation was not sampled"
    );

    let snapshot = dump_to_vec();
    stop();

    assert!(
        !snapshot.is_empty(),
        "pprof snapshot must contain the sampled allocation"
    );
}
