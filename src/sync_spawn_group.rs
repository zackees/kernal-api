//! Unix facade policy only; native launch, containment and reaping live in RP.
use std::io;
use std::process::Command;
use std::time::Duration;

const DEFAULT_KILL_DRAIN_TIMEOUT: Duration = Duration::from_secs(2);
const KILL_DRAIN_TIMEOUT_ENV: &str = "KERNAL_API_KILL_DRAIN_TIMEOUT_MS";

fn kill_drain_timeout() -> Duration {
    std::env::var(KILL_DRAIN_TIMEOUT_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_KILL_DRAIN_TIMEOUT)
}

pub fn spawn_sync_daemon(
    command: &mut Command,
    stdio: crate::platform::process::DaemonStdio<'_>,
    environment: crate::platform::process::SyncEnvironment,
    breakaway: bool,
) -> io::Result<crate::platform::process::DaemonChild> {
    running_process::spawn_daemon_with_environment(command, stdio, environment, breakaway)
}

pub fn spawn_sync(
    command: &mut Command,
    stdio: crate::platform::process::SpawnStdio<'_>,
    environment: crate::platform::process::SyncEnvironment,
) -> io::Result<crate::platform::process::SpawnedChild> {
    // Evaluate the facade's environment knob at drop, not at spawn.
    running_process::spawn_with_environment(
        command, stdio, environment, Some(kill_drain_timeout),
    )
}
