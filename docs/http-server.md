# Native HTTP server migration draft

Issue #176 tracks the complete capability and FastLED adoption. This branch's
initial transport is a local development checkpoint, not a release-ready server
replacement. Do not resolve #176 or adopt it in FastLED on the basis of the
initial transport tests alone.

## Implemented foundation

- Optional `http-server` feature; owned `Server`, `Request`, `Response`, `Limits`.
- Native HTTP/1 on the caller's runtime using the existing exact Hyper parser.
- A bounded connection task set. At capacity, acceptance waits; no new task is
  created for a queued connection. The OS owns its listen backlog.
- Header buffer/count limits, request-body collection limit, response-body
  acceptance limit, header/body/handler timeouts and absolute connection lifetime.
- Request/response types expose no HTTP implementation types. Framing response
  headers are rejected. Applications select addresses, methods, targets and
  payloads; authentication and routing are not transport responsibilities.
- Cancelling the serving future drops the listener and aborts its owned task
  set, closing incomplete client connections.
- Pull-driven file bodies preserve an optional prefix and stream bounded frames
  from the file's current position to its original length. Growth is ignored;
  premature EOF is an error. Prefix transmission does not read ahead into the
  file. There is no whole-file collection or producer queue.
- Pull-driven SSE bodies encode multiline data safely, emit keepalive comments,
  bound individual payloads and frames, and do not prefetch the next event while
  encoded bytes remain pending.
- Socket write-progress deadlines release stalled connections. Idle SSE sources
  do not start this deadline until an actual socket write or flush stalls.
- Application response-header acceptance has separate count and byte limits.
  Duplicate fields count separately; byte accounting includes each field's
  name, value and four framing bytes. Transport-generated fields and the status
  line are excluded. Zero permits no application fields.
- Constructors reject nonempty bodies for 204, 205 and 304, per
  [HTTP semantics](https://httpwg.org/specs/rfc9110.html). Connection-specific
  headers remain private, including keep-alive, proxy-connection and TE.
- A cloneable diagnostics handle exposes fixed-size saturating counters for
  accepted/completed connections, connection errors, absolute lifetime expiry,
  failed tasks (including handler panics), body and handler deadlines, and
  request/response acceptance failures. It survives server shutdown without
  retaining request contents or allocating an event queue. Snapshots are not
  transactional. Header/write/stream failures share the connection-error count;
  shutdown cancellation is not reported as a task failure.
- The `fs` feature provides `platform::fs::AsyncFileIo` for bounded screenshot
  persistence mechanics: shared fail-fast concurrency admission, per-write byte
  acceptance, parent directory creation, and a deadline. Cancellation signals
  between native calls; an already-running OS call keeps its permit until it
  ends. Writes follow trusted caller-selected paths and are not atomic or
  crash-durable. A timeout can leave partial output or finish an OS effect after
  return. FastLED still owns PNG validation, authorization, path selection and
  success/failure events; adoption remains pending.

The body limits are acceptance limits, not precise process-memory guarantees.
Request collection currently copies the bounded collected body into a vector;
application response construction may allocate before the server sees it.
The header byte setting is the private parser's read-buffer bound, not a
universal RSS or application-response-header allocation bound.
File metadata/position inspection remains synchronous; cancellation cannot
forcibly interrupt an OS file read already running in the private runtime's
blocking I/O pool. SSE encoding adds bounded overhead to the payload limit.

## Required before the server migration can land

- Adopt bounded filesystem effects for screenshot persistence and retain product
  error reporting; ensure file-open/read behavior fits the streamed routes.
- Request path/query handling and application-owned response-header/CORS policy,
  including behavior for transport-generated errors and HEAD/OPTIONS requests.
- Product route migration and parity checks before removing Axum, Tower HTTP and
  direct Tokio file/listener calls from FastLED.
- Native CI, review, merge, release and exact published application adoption.

## Local evidence

The foundation's focused test initially failed to import the missing module.
Fifteen integration tests cover the draft with `http-server,event-stream` enabled:
request/response round trip, request rejection, cancellation cleanup,
invalid limits, connection-capacity waiting, body/handler deadlines,
header/connection deadlines, response limit/framing validation, a streamed file
larger than the memory-body limit, SSE delivery before source closure, and a
non-reading client releasing its connection slot, duplicate/header-byte limits,
bodyless-status/connection-header rejection, and diagnostic counters for
rejections, handler panics, protocol failures and deadlines. Five focused unit tests cover
bounded file reads, growth/truncation, SSE backpressure/encoding/keepalives, and
write-timeout activation. The full kernel suite with `http-server,event-stream`,
strict all-target Clippy, formatting, and dependency-isolation RED -> GREEN
checks pass locally. The suite uses the Linux build-ID flag required by process
identity tests; an initial Soldr relay failure passed on retry.
These are draft transport results, not proof of the remaining requirements or
cross-platform validation of the new streaming paths.

The async file-write addition passes three unit tests: parent creation and
pre-effect byte rejection, timeout permit retention, and cancellation stop
signaling with permit retention. The full local suite and strict all-target
Clippy also pass with `fs,http-server,event-stream` enabled.
