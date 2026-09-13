# Native terminal input groundwork

The `pty` feature exposes `TerminalInputSession`, an owned raw-terminal capture
session. `new` snapshots the input mode and Drop attempts to restore it. Unix
returns `Ok(None)` for non-terminal stdin; Windows currently reports an error
when stdin is not an attached console. Callers must not assume parity yet.

Capture and active terminal-graphics probes share exclusive admission within
one kernel instance. A second capture returns `WouldBlock`; a probe declines
while capture owns input. This cannot coordinate other libraries, linked kernel
copies, or processes reading the same terminal. Restore failures during Drop
are best-effort, not a guarantee that the terminal mode was restored.

Windows capture holds at most 256 translated events and 64 KiB of queued input.
Key releases are ignored; a key press with more than 1024 repetitions stops
capture before repeated text is allocated. Queue overflow and native read/wait
failure stop capture and are reported by the fallible wait APIs, including
`TerminalInputSession::read_chunk`. Previously delivered events cannot be
recalled. Optional trace I/O is outside the queue mutex, but may still delay
the worker and its shutdown; this API does not promise a shutdown deadline.

`TerminalInputState` no longer exposes its Windows queue for external mutation.
The new capture-failure enum variants require downstream exhaustive matches to
be updated. Legacy `next_event` and `drain_events` do not report failures; new
consumers should use fallible waits instead.

These are raw chunks, not decoded key events. In particular, scanning a chunk
for spaces or newlines is not safe key matching: terminal escape sequences and
pasted input can contain those bytes. Issue #178 still requires the bounded
key-polling and styling facade, FastLED policy adoption, native validation,
release, and exact published-version consumption. This groundwork does not
complete the Crossterm migration.
