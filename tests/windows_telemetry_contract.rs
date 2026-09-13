use kernal_api::platform::resources::{commit_charge_mb, process_table};

#[test]
fn telemetry_signatures_preserve_unavailability() {
    let _: fn() -> Option<Vec<(u32, String)>> = process_table;
    let _: fn() -> Option<(u64, u64)> = commit_charge_mb;
}

#[cfg(not(windows))]
#[test]
fn windows_telemetry_is_explicitly_unavailable_elsewhere() {
    assert_eq!(process_table(), None);
    assert_eq!(commit_charge_mb(), None);
}

#[cfg(windows)]
#[test]
fn windows_snapshot_includes_this_process_and_commit_is_coherent() {
    let rows = process_table().expect("ToolHelp snapshot");
    assert!(rows
        .iter()
        .any(|(pid, name)| *pid == std::process::id() && !name.is_empty()));
    let (used, limit) = commit_charge_mb().expect("commit status");
    assert!(limit > 0);
    assert!(used <= limit);
}
