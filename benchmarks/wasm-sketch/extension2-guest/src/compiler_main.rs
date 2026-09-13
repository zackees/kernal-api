//! Core entrypoint for the shared compiler capability proof.
#[path = "../../shared/compiler_policy.rs"]
mod compiler_policy;

#[export_name = "kernal-api-run"]
pub extern "C" fn kernal_api_run() -> u32 {
    u32::from(kernal_api::guest::run(compiler_policy::proof()).is_err())
}

fn main() {
    if kernal_api_run() != 0 {
        std::process::exit(1);
    }
}
