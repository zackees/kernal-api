// Host-neutral configuration stays legal everywhere: feature, test, docs and
// debug predicates select no native host. Comments and strings that merely
// mention `cfg(windows)` or `libc::getpid` are not code, and an ordinary
// binding that happens to be called `windows` is not a path into the crate.

#[cfg(feature = "tokio-console")]
fn feature_gated() {}

#[cfg_attr(docsrs, doc = "documented")]
#[cfg(any(test, debug_assertions))]
fn debug_only() -> &'static str {
    "cfg(windows) and libc::getpid() in a string"
}

fn main() {
    let windows = [1_u8, 2];
    let _ = (windows, cfg!(debug_assertions));
}
