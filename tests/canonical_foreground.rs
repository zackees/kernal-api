//! The foreground namespace must expose the producer functions unchanged.

#[test]
fn foreground_functions_retain_native_command_and_result_contracts() {
    type Status = fn(&mut std::process::Command) -> std::io::Result<std::process::ExitStatus>;
    type Output = fn(&mut std::process::Command) -> std::io::Result<std::process::Output>;
    let _: Status = kernal_api::foreground::status;
    let _: Status = running_process::foreground::status;
    let _: Output = kernal_api::foreground::output;
    let _: Output = running_process::foreground::output;
    // Function signatures alone also permit wrappers; retain the exact module
    // alias as a source-level policy contract, without unstable address checks.
    assert!(include_str!("../src/lib.rs").contains("pub use running_process::foreground;"));
}
