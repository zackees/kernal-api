//! Awaiting another process's exit (Windows).

use std::io;
use std::ptr;
use std::sync::Arc;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetExitCodeProcess, OpenProcess, SetEvent, WaitForMultipleObjects, INFINITE,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::platform::process::{
    ProcessExitObservation, ProcessId, ProcessInspectError, ProcessInspectErrorKind,
    ProcessSessionExit,
};

/// The right to wait on a kernel object.
///
/// Spelled out here rather than imported: `windows-sys` files it under the
/// filesystem namespace, which is a generator artefact and would read at this
/// call site as though a file were involved.
const SYNCHRONIZE: u32 = 0x0010_0000;

/// `WaitForMultipleObjects` reports this when the wait itself failed.
const WAIT_FAILED: u32 = 0xFFFF_FFFF;

/// A subscription to one process's exit, held open until it arrives.
///
/// The open handle *is* the reuse safety. Windows will not reissue a PID
/// while any handle to that process remains open, so a handle taken once
/// keeps naming the same process -- including after it exits, when it becomes
/// a handle to a known-dead process rather than a stale number. Opening it
/// either succeeds against the process alive at that moment or fails, so the
/// watch cannot silently retarget.
///
/// This host is the generous one about status: `GetExitCodeProcess` answers
/// through any handle with query rights, parent or not, so the exit code is
/// reported where Linux and macOS have nothing to give.
pub struct ProcessExitWatch {
    pid: ProcessId,
    process: Arc<KernelHandle>,
}

impl std::fmt::Debug for ProcessExitWatch {
    /// Names the process, not the handle.
    ///
    /// The handle value is an artefact of this process's own table; printing
    /// it invites a reader to compare two values that were never comparable.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProcessExitWatch")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl ProcessExitWatch {
    /// Subscribe to `pid`'s exit, failing if it is not running now.
    ///
    /// Synchronous and runtime-free: acquisition is the part that must happen
    /// at a known moment, and making it `async` would put a scheduling delay
    /// between the caller's decision and the kernel pinning the target.
    ///
    /// `SYNCHRONIZE` is requested alongside query rights because a handle
    /// opened for queries alone cannot be waited on -- the access mask is
    /// checked when the wait starts, not when it would have completed.
    pub fn open(pid: ProcessId) -> Result<Self, ProcessInspectError> {
        // SAFETY: the call takes access flags, an inherit flag, and a PID by
        // value; the returned handle is checked before use.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE,
                0,
                pid.get(),
            )
        };
        if handle.is_null() {
            let source = io::Error::last_os_error();
            const ERROR_INVALID_PARAMETER: i32 = 87;
            let kind = match source.raw_os_error() {
                Some(ERROR_INVALID_PARAMETER) => ProcessInspectErrorKind::NotFound,
                _ => ProcessInspectErrorKind::Host,
            };
            return Err(ProcessInspectError { kind, source });
        }
        Ok(Self {
            pid,
            process: Arc::new(KernelHandle(handle)),
        })
    }

    /// The process this watch was opened for.
    #[must_use]
    pub fn pid(&self) -> ProcessId {
        self.pid
    }

    /// Wait until that process exits.
    ///
    /// This host has no readiness descriptor to hand a reactor, so the wait
    /// happens on a blocking worker. The second waited-on object is a cancel
    /// event signalled when this future is dropped, so giving up on the wait
    /// releases the worker instead of parking it until the target happens to
    /// exit.
    ///
    /// Waiting is not reaping: a `PlatformChild` or `std::process::Child` for
    /// the same process still waits successfully afterwards.
    pub async fn exited(&self) -> Result<ProcessExitObservation, ProcessInspectError> {
        let cancel = Arc::new(create_cancel_event()?);
        let process = Arc::clone(&self.process);
        let waiter = Arc::clone(&cancel);
        let release = CancelOnDrop(cancel);
        let outcome =
            crate::async_engine::launch_blocking(move || wait_for_exit(&process, &waiter))
                .await
                .map_err(|error| ProcessInspectError {
                    kind: ProcessInspectErrorKind::Host,
                    source: io::Error::other(error.to_string()),
                })?;
        drop(release);
        outcome
    }
}

/// A kernel handle this crate owns and closes exactly once.
struct KernelHandle(HANDLE);

// SAFETY: a process or event handle is a kernel object usable from any
// thread; the value is opaque here and never dereferenced.
unsafe impl Send for KernelHandle {}
unsafe impl Sync for KernelHandle {}

impl Drop for KernelHandle {
    fn drop(&mut self) {
        // SAFETY: `self.0` came from OpenProcess or CreateEventW and is
        // closed once.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Signals the cancel event when the awaiting future goes away.
struct CancelOnDrop(Arc<KernelHandle>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        // SAFETY: the event handle is live for as long as this Arc is.
        unsafe {
            SetEvent(self.0 .0);
        }
    }
}

/// A manual-reset event, so a cancellation set once stays set.
fn create_cancel_event() -> Result<KernelHandle, ProcessInspectError> {
    // SAFETY: default security, manual reset, initially unsignalled, unnamed;
    // the returned handle is checked before use.
    let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if handle.is_null() {
        return Err(ProcessInspectError::last_os_error(
            ProcessInspectErrorKind::Host,
        ));
    }
    Ok(KernelHandle(handle))
}

fn wait_for_exit(
    process: &KernelHandle,
    cancel: &KernelHandle,
) -> Result<ProcessExitObservation, ProcessInspectError> {
    let handles = [process.0, cancel.0];
    // SAFETY: two live handles are described, `bWaitAll` is false, and the
    // wait is released either by the process exiting or by cancellation.
    let waited = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
    if waited == WAIT_OBJECT_0 {
        return Ok(exit_status(process));
    }
    if waited == WAIT_FAILED {
        return Err(ProcessInspectError::last_os_error(
            ProcessInspectErrorKind::Host,
        ));
    }
    Err(ProcessInspectError::stated(
        ProcessInspectErrorKind::Host,
        "the exit wait was cancelled",
    ))
}

fn exit_status(process: &KernelHandle) -> ProcessExitObservation {
    let mut code = 0_u32;
    // SAFETY: the handle is live and the out-parameter is an initialised u32.
    let ok = unsafe { GetExitCodeProcess(process.0, &mut code) };
    if ok == 0 {
        return ProcessExitObservation::Unreported;
    }
    // The DWORD is kept whole in the native status; the signed convenience
    // code is the same bits read the way a caller expects to compare them.
    ProcessExitObservation::Reported(ProcessSessionExit::from_native(
        Some(code as i32),
        None,
        code,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A process that is not there cannot be subscribed to, which is the
    /// acquisition half of reuse safety: no handle, so nothing to retarget.
    ///
    /// Waiting is not enough on this host. A `Child` keeps the process handle
    /// open until it is dropped, and while any handle exists the process
    /// object does too -- which is exactly the property that makes a watch
    /// reuse-safe, and exactly why the child has to be let go of here before
    /// the PID counts as gone.
    #[test]
    fn a_reaped_process_cannot_be_watched() {
        let mut child = std::process::Command::new("cmd.exe")
            .args(["/C", "exit 0"])
            .spawn()
            .expect("spawn");
        let pid = ProcessId::new(child.id()).expect("child pid is in range");
        child.wait().expect("reap");
        drop(child);
        let error = ProcessExitWatch::open(pid).expect_err("a reaped pid is gone");
        assert_eq!(error.kind, ProcessInspectErrorKind::NotFound);
    }

    /// A watch names the process it was opened for, and this host will open
    /// one for any process it can query rather than only for its children.
    #[test]
    fn a_watch_can_be_opened_for_a_process_this_one_did_not_spawn() {
        let watch = ProcessExitWatch::open(ProcessId::current()).expect("open self");
        assert_eq!(watch.pid(), ProcessId::current());
    }
}
