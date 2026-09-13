# Ctrl+C notifications

`async_engine::Runtime::interrupt_signal` eagerly registers an owned listener
for Unix SIGINT or Windows console CTRL_C. The listener borrows the runtime;
use a runtime built with `enable_all`, and drive it while waiting. Requesting
this facility without enabled drivers returns `Unsupported`, without installing
a handler. Native registration failures propagate as I/O errors.

`InterruptSignal::wait` waits for one notification and reports a closed receiver
as `BrokenPipe`. Dropping a pending wait does not consume a later notification.
Signals can coalesce; delivery is not counting, and each registered listener is
notified. There is no event backlog proportional to the number of interrupts.
No new runtime, polling thread or dependency is introduced.

`Runtime::wait_for_interrupt` is a convenience future that registers when first
polled. Applications that change terminal modes before polling must use eager
registration instead. A wait intentionally has no timeout; compose it with a
deadline or cancellation where required by application policy.

Registration affects process-wide signal handling. Dropping the listener does
not restore native default handlers. This facility does not handle SIGTERM,
Windows CTRL_BREAK, console close, shutdown or logoff, and does not replace the
existing daemon shutdown-request contract. Do not combine competing signal
installers without explicitly coordinating ownership. Applications decide exit
codes, repeated-interrupt policy and cleanup. Abrupt termination cannot run Drop.

Issue #187 is coordinated with zackees/fastled-wasm#242. Native delivery tests
run only in isolated child processes, with a fresh console on Windows and a
parent-enforced deadline. Publication and exact client adoption remain required.
