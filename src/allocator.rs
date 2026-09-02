//! One allocator and heap-profiler facade for every client process.
//!
//! The profiler is compiled in but dormant until [`start`] or
//! [`start_from_env`] is called. Applications still declare their own
//! `#[global_allocator]`; this crate supplies the exact shared allocator type
//! and dump implementation without attempting to install process-wide policy
//! from a library.

use std::alloc::{GlobalAlloc, Layout};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Facade-owned mimalloc allocator with dormant heap sampling support.
pub struct Allocator;

static INNER_ALLOCATOR: mimalloc_pprof::MiMalloc = mimalloc_pprof::MiMalloc;

impl Allocator {
    /// Construct the shared allocator for a `#[global_allocator]` static.
    pub const fn new() -> Self {
        Self
    }
}

impl Default for Allocator {
    fn default() -> Self {
        Self::new()
    }
}

// SAFETY: every operation delegates unchanged layouts and pointers to the one
// process-wide mimalloc implementation selected by this facade.
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { INNER_ALLOCATOR.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        unsafe { INNER_ALLOCATOR.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { INNER_ALLOCATOR.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        unsafe { INNER_ALLOCATOR.realloc(pointer, layout, size) }
    }
}

/// Default environment variable understood by [`start_from_env`].
pub const HEAP_PROFILE_ENV: &str = "KERNAL_API_HEAP_PROFILE";

/// mimalloc-pprof's low-overhead default sample rate (512 KiB).
pub const DEFAULT_SAMPLE_RATE: usize = 512 * 1024;

static DUMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Parse a heap-profile setting into a byte sampling interval.
pub fn sample_rate_from(value: &str) -> Option<usize> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "off" | "no" => None,
        "1" | "true" | "on" | "yes" => Some(DEFAULT_SAMPLE_RATE),
        other => other.parse::<usize>().ok().filter(|rate| *rate > 0),
    }
}

/// Start profiling from `env_var`, returning the selected sample rate.
pub fn start_from_named_env(env_var: &str) -> Option<usize> {
    let rate = sample_rate_from(&std::env::var(env_var).ok()?)?;
    mimalloc_pprof::prof::start(rate).then_some(rate)
}

/// Start profiling from [`HEAP_PROFILE_ENV`].
pub fn start_from_env() -> Option<usize> {
    start_from_named_env(HEAP_PROFILE_ENV)
}

/// Start profiling at the requested byte sampling interval.
pub fn start(sample_rate: usize) -> bool {
    sample_rate > 0 && mimalloc_pprof::prof::start(sample_rate)
}

/// Stop sampling. Retained samples remain dumpable.
pub fn stop() {
    mimalloc_pprof::prof::stop();
}

/// Whether the profiler is currently sampling.
pub fn is_enabled() -> bool {
    mimalloc_pprof::prof::is_enabled()
}

/// Number of currently live sampled allocations.
pub fn live_sample_count() -> usize {
    mimalloc_pprof::prof::stats().live_samples
}

/// A collision-resistant default filename for one heap snapshot.
pub fn next_dump_name() -> String {
    let sequence = DUMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or_default();
    format!("heap-{}-{millis}-{sequence}.pb", std::process::id())
}

/// Write the retained heap samples to an explicit pprof protobuf path.
pub fn dump_to(path: impl AsRef<Path>) -> std::io::Result<()> {
    mimalloc_pprof::prof::dump_proto_file(path.as_ref())
}

/// Serialize the retained heap samples to an in-memory pprof protobuf buffer.
///
/// Use this when a caller needs the snapshot bytes directly — for example to
/// hand them to an API response or an in-process test assertion — rather
/// than writing them to disk first. Returns an empty buffer if the profiler
/// failed to serialize the snapshot; it never returns an error.
pub fn dump_to_vec() -> Vec<u8> {
    mimalloc_pprof::prof::dump_proto_to_vec()
}

/// Create `directory` asynchronously and write a uniquely named pprof dump.
pub async fn dump_in(directory: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let directory = directory.as_ref();
    tokio::fs::create_dir_all(directory).await?;
    let path = directory.join(next_dump_name());
    let dump_path = path.clone();
    tokio::task::spawn_blocking(move || dump_to(&dump_path))
        .await
        .map_err(std::io::Error::other)??;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_are_explicit_and_zero_never_means_sample_everything() {
        for off in ["", "0", "00", "false", "off", "no"] {
            assert_eq!(sample_rate_from(off), None, "{off:?}");
        }
        for on in ["1", "true", "on", "yes"] {
            assert_eq!(sample_rate_from(on), Some(DEFAULT_SAMPLE_RATE), "{on:?}");
        }
        assert_eq!(sample_rate_from("65536"), Some(65_536));
    }

    #[test]
    fn dump_names_do_not_collide() {
        let first = next_dump_name();
        let second = next_dump_name();
        assert_ne!(first, second);
        assert!(first.ends_with(".pb"));
    }

    // `dump_to_vec` needs samples captured through mimalloc's allocator, which
    // means a `#[global_allocator]` declaration. This lib unit-test binary has
    // none (declaring one here would switch every other unit test in the
    // crate onto mimalloc too), so that behavior is exercised instead by the
    // dedicated `tests/allocator_heap_profile.rs` integration test, which
    // mirrors the real zccache consumer: a separate final executable that
    // installs `Allocator` as its global allocator before profiling.
}
