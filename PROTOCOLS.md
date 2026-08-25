# Protocol contract

All `kernal-api` communication that must be decoded across a process,
executable, or durable-version boundary uses Protocol Buffers. Messages keep
stable numeric field tags, reserve retired tags instead of reusing them, and
convert at the boundary to facade-owned domain types. Prost-generated types
are private implementation details.

Current protobuf boundaries include:

- capture-to-symbolizer worker requests and responses;
- symbol manifest discovery; and
- any future daemon, broker, profiling-control, or dump-control IPC.

JSON is not accepted as a control-plane or IPC fallback. It remains valid only
where an external human-facing tool defines JSON as its import format, such as
Firefox Profiler export.

## Crash journal exception

The first-stage crash handler writes a fixed-size, versioned binary journal.
This is intentional: fatal signal/exception context cannot allocate, lock, or
invoke a general protobuf encoder safely. The handler only copies bounded
primitive fields into a preallocated record using async-signal-safe writes.

Once normal execution resumes in the reporter process, the journal is parsed
into facade-owned types. Any cross-process or durable report emitted after
that safe boundary must use a tagged protobuf message. The raw crash journal
is never an IPC request/response protocol and has no JSON representation.
