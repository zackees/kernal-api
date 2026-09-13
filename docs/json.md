# JSON documents

Enable `json` for `kernal_api::json::{parse, encode, Value, Layout}`. The API
owns its values and errors; it exposes no Serde bounds, derives or backend
types. Applications retain schema validation, defaults, field names, unknown
field handling, HTTP status codes and trailing-newline policy.

Parsing accepts one UTF-8 JSON value with optional surrounding whitespace.
Duplicate object keys retain the last value. Objects encode in sorted key
order, arrays retain order, and pretty output uses two-space indentation with
no trailing newline. Signed and unsigned 64-bit extrema are exact. Positive
integers use the signed variant when representable; larger integers outside
both integer ranges can round to finite f64. This is not arbitrary precision.
Nonfinite caller-provided floats are rejected instead of silently becoming null.

Input is limited to 8 MiB before parsing. Decoded trees are limited to 262144
values (including containers, excluding object keys) and depth 64, root zero.
Those traversal bounds apply after the private parser constructs its tree;
they are not independent parser memory or CPU quotas. The private parser's
own recursion limit remains enabled.

Encoding validates count, depth and finite numbers before serialization,
borrows caller-owned values without constructing a second tree, and limits
output to 8 MiB including whitespace and escaping. Failures return no partial
document and diagnostics never echo source contents. Limits do not constrain
allocations performed by callers while constructing their values.

JSON is distinct from TOML configuration: null, unsigned integers and JSON
output semantics should not alter the configuration contract. The private
JSON backend is also used by the existing Firefox profile exporter.
