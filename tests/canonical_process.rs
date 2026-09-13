//! Compile-time identity checks for the reviewed synchronous migration set.

use kernal_api::process::{
    DaemonStdio, EnvironmentPolicy, SpawnStdio, StdioSource, SyncEnvironment,
};

fn backend_to_platform_console(
    value: running_process::ConsoleWindowInfo,
) -> kernal_api::platform::process::ConsoleWindowInfo {
    value
}
fn platform_to_backend_console(
    value: kernal_api::platform::process::ConsoleWindowInfo,
) -> running_process::ConsoleWindowInfo {
    value
}
fn backend_to_platform_capture_stream(
    value: running_process::StreamKind,
) -> kernal_api::platform::process::CaptureStream {
    value
}
fn platform_to_backend_capture_stream(
    value: kernal_api::platform::process::CaptureStream,
) -> running_process::StreamKind {
    value
}
fn backend_to_platform_inspect_error(
    value: running_process::ProcessInspectError,
) -> kernal_api::platform::process::ProcessInspectError {
    value
}
fn platform_to_backend_inspect_error(
    value: kernal_api::platform::process::ProcessInspectError,
) -> running_process::ProcessInspectError {
    value
}
fn backend_to_facade_liveness(
    value: running_process::ProcessLiveness,
) -> kernal_api::ProcessLiveness {
    value
}
fn facade_to_backend_liveness(
    value: kernal_api::ProcessLiveness,
) -> running_process::ProcessLiveness {
    value
}

fn backend_to_facade_environment(value: running_process::EnvironmentPolicy) -> EnvironmentPolicy {
    value
}
fn facade_to_backend_environment(value: EnvironmentPolicy) -> running_process::EnvironmentPolicy {
    value
}
fn backend_to_facade_sync_environment(value: running_process::SyncEnvironment) -> SyncEnvironment {
    value
}
fn facade_to_backend_sync_environment(value: SyncEnvironment) -> running_process::SyncEnvironment {
    value
}
fn backend_to_facade_daemon_stdio(
    value: running_process::DaemonStdio<'static>,
) -> DaemonStdio<'static> {
    value
}
fn facade_to_backend_daemon_stdio(
    value: DaemonStdio<'static>,
) -> running_process::DaemonStdio<'static> {
    value
}
fn backend_to_facade_stdio(value: running_process::SpawnStdio<'static>) -> SpawnStdio<'static> {
    value
}
fn facade_to_backend_stdio(value: SpawnStdio<'static>) -> running_process::SpawnStdio<'static> {
    value
}
fn backend_to_facade_source(value: running_process::StdioSource<'static>) -> StdioSource<'static> {
    value
}
fn facade_to_backend_source(value: StdioSource<'static>) -> running_process::StdioSource<'static> {
    value
}
fn platform_to_backend_stdio(
    value: kernal_api::platform::process::SpawnStdio<'static>,
) -> running_process::SpawnStdio<'static> {
    value
}
fn backend_to_platform_stdio(
    value: running_process::SpawnStdio<'static>,
) -> kernal_api::platform::process::SpawnStdio<'static> {
    value
}
fn platform_to_backend_daemon_stdio(
    value: kernal_api::platform::process::DaemonStdio<'static>,
) -> running_process::DaemonStdio<'static> {
    value
}
fn backend_to_platform_daemon_stdio(
    value: running_process::DaemonStdio<'static>,
) -> kernal_api::platform::process::DaemonStdio<'static> {
    value
}
fn platform_to_backend_daemon_child(
    value: kernal_api::platform::process::DaemonChild,
) -> running_process::DaemonChild {
    value
}
fn backend_to_platform_daemon_child(
    value: running_process::DaemonChild,
) -> kernal_api::platform::process::DaemonChild {
    value
}
fn platform_to_backend_spawned_child(
    value: kernal_api::platform::process::SpawnedChild,
) -> running_process::SpawnedChild {
    value
}
fn backend_to_platform_spawned_child(
    value: running_process::SpawnedChild,
) -> kernal_api::platform::process::SpawnedChild {
    value
}

#[test]
fn canonical_process_types_are_assignable_in_both_namespaces() {
    let _ = (
        backend_to_facade_environment,
        facade_to_backend_environment,
        backend_to_facade_sync_environment,
        facade_to_backend_sync_environment,
        backend_to_platform_console,
        platform_to_backend_console,
        backend_to_platform_capture_stream,
        platform_to_backend_capture_stream,
        backend_to_platform_inspect_error,
        platform_to_backend_inspect_error,
        backend_to_facade_liveness,
        facade_to_backend_liveness,
        backend_to_facade_daemon_stdio,
        facade_to_backend_daemon_stdio,
        backend_to_facade_stdio,
        facade_to_backend_stdio,
        backend_to_facade_source,
        facade_to_backend_source,
        platform_to_backend_stdio,
        backend_to_platform_stdio,
        platform_to_backend_daemon_stdio,
        backend_to_platform_daemon_stdio,
        platform_to_backend_daemon_child,
        backend_to_platform_daemon_child,
        platform_to_backend_spawned_child,
        backend_to_platform_spawned_child,
    );

    let _: fn(&running_process::ProcessLiveness) -> std::io::Result<bool> =
        running_process::ProcessLiveness::has_exited;
    let _: fn(&kernal_api::ProcessLiveness) -> std::io::Result<bool> =
        kernal_api::ProcessLiveness::has_exited;

    let _: fn(&mut std::process::Command) -> std::io::Result<kernal_api::process::DaemonChild> =
        kernal_api::process::spawn_daemon;
    let _: fn(&mut std::process::Command) -> std::io::Result<running_process::DaemonChild> =
        kernal_api::process::spawn_daemon;
    let _: fn(
        &mut std::process::Command,
        running_process::DaemonStdio<'_>,
        Vec<(std::ffi::OsString, std::ffi::OsString)>,
        bool,
    ) -> std::io::Result<running_process::DaemonChild> =
        kernal_api::process::spawn_daemon_with_explicit_environment;
    let _: fn(
        &mut std::process::Command,
        running_process::SpawnStdio<'_>,
        Vec<(std::ffi::OsString, std::ffi::OsString)>,
        Option<fn() -> std::time::Duration>,
    ) -> std::io::Result<running_process::SpawnedChild> =
        kernal_api::process::spawn_with_explicit_environment;
    let _: fn(
        &mut std::process::Command,
        running_process::DaemonStdio<'_>,
        running_process::SyncEnvironment,
        bool,
    ) -> std::io::Result<running_process::DaemonChild> =
        kernal_api::process::spawn_daemon_with_environment;
    let _: fn(
        &mut std::process::Command,
        running_process::SpawnStdio<'_>,
        running_process::SyncEnvironment,
        Option<fn() -> std::time::Duration>,
    ) -> std::io::Result<running_process::SpawnedChild> =
        kernal_api::process::spawn_with_environment;
    let _: fn(std::time::Duration) -> Vec<running_process::ConsoleWindowInfo> =
        kernal_api::monitor_console_windows;
    let _: fn(u32) -> std::io::Result<()> = kernal_api::platform::process::force_terminate_pid;
    let _: fn(u32) -> std::io::Result<()> =
        kernal_api::platform::process::force_terminate_process_group;
    let _: fn() = kernal_api::platform::process::detach_standard_streams;
    let _: fn(&std::path::Path) -> bool =
        kernal_api::platform::process::redirect_standard_streams_to_log;
    let _: fn() -> bool = kernal_api::platform::process::native_jobserver_supported;
    let _: fn(usize) -> std::io::Result<kernal_api::platform::process::NativeJobserver> =
        kernal_api::platform::process::NativeJobserver::create;
    let _: fn(&tokio::process::Child, kernal_api::ProcessPriority) -> std::io::Result<()> =
        kernal_api::platform::process::apply_priority_to_async_child;
    let _: fn(
        std::process::Command,
        Option<std::time::Duration>,
        usize,
    ) -> Result<kernal_api::process::RunOutput, kernal_api::process::ProcessError> =
        kernal_api::process::run_std_command_bounded;
}

#[test]
fn platform_spawned_child_delegates_drop_shutdown_exactly_once() {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    struct Control(Arc<AtomicUsize>);
    impl running_process::SpawnedChildControl for Control {
        fn kill(&mut self) -> std::io::Result<()> {
            Ok(())
        }
        fn wait(&mut self) -> std::io::Result<i32> {
            Ok(0)
        }
        fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
            Ok(Some(0))
        }
        fn shutdown(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    let shutdowns = Arc::new(AtomicUsize::new(0));
    let child = kernal_api::platform::process::SpawnedChild::from_parts(
        123,
        None,
        None,
        None,
        Box::new(Control(Arc::clone(&shutdowns))),
    );
    drop(child);
    assert_eq!(shutdowns.load(Ordering::SeqCst), 1);
}

#[test]
fn all_environment_launches_use_the_canonical_live_environment_entrypoints() {
    let source = include_str!("../src/sync_spawn_group.rs");
    assert!(
        source.contains("running_process::spawn_with_environment")
            && source.contains("Some(kill_drain_timeout)"),
        "Unix/macOS launch must delegate every environment policy and preserve its drop-time drain policy"
    );
    let windows = include_str!("../src/platform_win/sync_spawn.rs");
    assert!(
        windows.contains("running_process::spawn_with_environment(command, stdio, environment, None)"),
        "Windows launch must delegate every environment policy to its canonical Job-close implementation"
    );

    assert!(
        source.contains("running_process::spawn_daemon_with_environment(command, stdio, environment, breakaway)"),
        "Unix/macOS daemon launch must delegate every environment policy with stdio and breakaway"
    );
    assert!(
        windows.contains("running_process::spawn_daemon_with_environment(command, stdio, environment, breakaway)"),
        "Windows daemon launch must delegate every environment policy with stdio and breakaway"
    );
}

#[test]
fn platform_stdio_defaults_remain_the_canonical_safe_defaults() {
    let stdio = kernal_api::platform::process::SpawnStdio::default();
    assert!(matches!(
        stdio.stdin,
        kernal_api::platform::process::StdioSource::Null
    ));
    assert!(matches!(
        stdio.stdout,
        kernal_api::platform::process::StdioSource::Parent
    ));
    assert!(matches!(
        stdio.stderr,
        kernal_api::platform::process::StdioSource::Parent
    ));
    assert_eq!(stdio.drain_timeout, Some(std::time::Duration::from_secs(2)));
    assert!(!stdio.show_console);

    let daemon = kernal_api::platform::process::DaemonStdio::default();
    assert!(matches!(
        daemon.stdout,
        kernal_api::platform::process::DaemonStdioSource::Null
    ));
    assert!(matches!(
        daemon.stderr,
        kernal_api::platform::process::DaemonStdioSource::Null
    ));
}
