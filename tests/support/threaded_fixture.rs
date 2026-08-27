// Shared raw ThreadedRustV1 fixture builder.  Keeping the source fixture in
// one integration binary avoids production-only test APIs while allowing
// other integration binaries to exercise the same admitted artifact.
#[allow(dead_code)]
mod source {
    include!("../threaded_root_execution.rs");
}

pub fn looping_root_wasm() -> Vec<u8> {
    source::threaded_root_wasm(None, false, false, true)
}
