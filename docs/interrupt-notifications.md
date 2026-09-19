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

Launched `'static` work that cannot borrow its `Runtime` (a daemon's shutdown
watcher) uses the ambient forms on the currently entered runtime instead:
`async_engine::wait_for_interrupt()` registers for Ctrl+C when first polled,
and `async_engine::TerminationSignal::new()` registers eagerly for the host's
graceful-termination request: Unix SIGTERM, or Windows console CTRL_BREAK,
close and shutdown. Both return `Unsupported` with no entered runtime; a
runtime without its signal driver panics. `TerminationSignal::recv` returns
`None` once the native receiver closes and is cancellation-safe. Windows ends
the process shortly after a close or shutdown event regardless of the
handler; that notification is the window to drain, not a veto. Logoff is not
observed, because services receive every user's logoff.

Registration affects process-wide signal handling. Dropping the listener does
not restore native default handlers. None of these replace the existing daemon
shutdown-request contract. Do not combine competing signal
installers without explicitly coordinating ownership. Applications decide exit
codes, repeated-interrupt policy and cleanup. Abrupt termination cannot run Drop.

Issue #187 is coordinated with zackees/fastled-wasm#242. Native delivery tests
run only in isolated child processes, with a fresh console on Windows and a
parent-enforced deadline. Publication and exact client adoption remain required.
