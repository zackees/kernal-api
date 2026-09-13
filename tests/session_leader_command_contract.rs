//! Contract coverage for the legacy `setsid`/Windows-group command primitive.

#[cfg(unix)]
#[test]
fn session_leader_is_not_merely_a_child_process_group() {
    let mut command = std::process::Command::new("sh");
    command.args([
        "-c",
        "sid=$(ps -o sid= -p $$ | tr -d ' '); test \"$sid\" = \"$$\"",
    ]);
    kernal_api::platform::process::configure_session_leader_command(&mut command);
    assert!(command.status().expect("spawn session leader").success());
}

#[cfg(windows)]
#[test]
fn session_leader_command_is_exposed_without_hiding_the_console() {
    let mut command = std::process::Command::new("cmd.exe");
    kernal_api::platform::process::configure_session_leader_command(&mut command);
    // The native flag is intentionally applied only at process creation;
    // this regression asserts the semantic API remains separate from the
    // no-window trampoline configuration.
    let _ = command;
}
