# Native terminal input groundwork

The dependency-free `terminal-input` feature exposes key decoding and
`TerminalInputSession`, an owned raw-terminal capture session. The `pty` feature
includes it and additionally enables PTY process spawning through a private
backend. `new` snapshots the input mode and Drop attempts to restore it. Unix
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
pasted input can contain those bytes. Use the decoded API below for key policy.

## Decoded keys

`keys::TerminalKeys` reuses native capture in noncanonical, no-echo mode and returns one
facade-owned `KeyEvent` per `poll`. Calls accept a requested wait of at most
100 ms and examine at most 64 KiB of bytes. Remaining queued bytes survive
subsequent calls, including zero-wait calls. Drop uses the native session's
best-effort restoration. Existing signal-key behavior is preserved (Unix ISIG
and Windows processed input), as are Unix input translations and output flags.
With normal initial settings Ctrl+C therefore remains a native signal, not a
decoded event. Applications must register graceful signal handling before
opening capture so interruption can drop the owner and restore modes; abrupt
process termination cannot run Rust Drop. Opening still has the platform-specific
non-terminal behavior above. The raw `TerminalInputSession::new` API used for
PTY forwarding is unchanged.

`keys::KeyDecoder` is the same incremental decoder without native ownership;
callers feeding bytes directly own its timing and use `finish_pending` after
their inter-byte deadline. TerminalKeys checks incomplete-sequence age on polls
when no previously buffered bytes remain: after 250 ms a lone Escape becomes an
Escape event and other partial sequences fail. Poll timing is a requested OS
wait plus bounded parsing, not a hard real-time guarantee.

The small contract recognizes UTF-8 characters, Enter, Escape and legacy
control characters and unambiguous Alt characters. Alt+Space and reserved Alt
introducers overlap terminal control-sequence encodings; they are conservatively
parsed as control sequences, not text keys. An incomplete sequence then fails
closed. Consumers must not assume full legacy-modifier equivalence with Crossterm.
Shift and key-release information cannot be recovered
from legacy terminal bytes. Windows capture ignores releases and expands
bounded repeats. CR/LF mean Enter; their legacy control-letter aliases are not
distinguished. CSI/SS3 and intermediate escape sequences produce `Other`, as do
complete bracketed pastes and control strings. Plain unbracketed paste cannot
be distinguished from typed text. Unsupported X10 mouse payloads fail closed.
This is not a general terminal emulator or a Crossterm event-type wrapper.

Decoder buffering is at most 128 bytes. A control string or bracketed paste
may consume at most 64 KiB including its introducer and terminator, without
buffering the body. Invalid UTF-8, malformed or oversized sequences poison the
decoder: no trailing spaces are reinterpreted as keys after an error.

## Diagnostic formatting

The independent `terminal-style` feature adds no dependency. It provides a
borrowed `StyledText` formatter with a facade-owned 16-color foreground palette.
The caller chooses the color and whether it is enabled. The formatter neither
reads environment variables nor detects terminals. Disabled output preserves
plain text; enabled output wraps it in ANSI foreground selection and default
foreground reset. It preserves embedded escapes and is not a sanitizer. Writer
errors propagate; a reset cannot be guaranteed after an output failure.

`prepare_stderr_ansi` enables Windows console ANSI processing while preserving
other mode flags. It returns false for redirected/non-console stderr or consoles
that reject VT support, and errors for other native failures. Preparation is
idempotent but persistent: writers sharing the console buffer can observe the
mode change. On Unix it needs no native action and returns true, which is not
a TTY or terminal-capability assertion. Applications own `NO_COLOR`, `TERM`,
redirection, fallback and diagnostic-message policy.

Issue #178 still requires decoded-key/style native validation, FastLED policy
adoption, release and exact published-version consumption. This does not yet
complete the Crossterm migration.
