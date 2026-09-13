//! Core entrypoint glue for the shared public-facade hash policy.
#[path = "../../shared/hash_policy.rs"]
mod hash_policy;

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    u32::from(kernal_api::guest::run(hash_policy::proof()).is_err())
}

fn main() {
    if kernal_api_run() != 0 {
        std::process::exit(1);
    }
}
