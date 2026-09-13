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

The body limits are acceptance limits, not precise process-memory guarantees.
Request collection currently copies the bounded collected body into a vector;
application response construction may allocate before the server sees it.
The header byte setting is the private parser's read-buffer bound, not a
universal RSS or application-response-header allocation bound.

## Required before the server migration can land

- Pull-driven file and SSE response bodies with bounded chunks, no pump queue,
  and no whole-file collection. Preserve prefix injection for the test worker.
- Response write/progress deadlines in addition to absolute connection lifetime.
- Bounded response-header acceptance and status/body semantic validation.
- Observable connection/protocol/timeout/handler failures; the current draft
  isolates client failures but discards their outcomes.
- Bounded async filesystem effects needed for screenshot persistence.
- Request path/query handling and application-owned response-header/CORS policy,
  including behavior for transport-generated errors and HEAD/OPTIONS requests.
- Product route migration and parity checks before removing Axum, Tower HTTP and
  direct Tokio file/listener calls from FastLED.
- Native CI, review, merge, release and exact published application adoption.

## Initial evidence

The focused test initially failed to import the missing module. Eight tests now
pass: request/response round trip, request rejection, cancellation cleanup,
invalid limits, connection-capacity waiting, body/handler deadlines,
header/connection deadlines and response limit/framing validation. The
server-enabled kernel suite and strict all-target Clippy also pass locally.
These are transport-foundation results, not proof of the remaining requirements.
