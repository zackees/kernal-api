# C++ function analysis

The optional `source-cpp` feature privately owns an exact-pinned C++ grammar and
parser. `source::analyze_cpp` returns owned `FunctionDefinition` records in source
order, with byte ranges and namespace/aggregate/explicit-linkage context flags.
It does not expose parser nodes, trees, grammar symbols, or backend traits.

The implementation retains provenance from FastLED/fbuild's source scanner at
`1e75ccf5a4ca922b4d922a6da286b965fac8832d`, via the FastLED WASM adapter. It uses
syntax nodes to remove function parameter defaults, rather than splitting on
commas or guessing expression nesting. Headers retain their source formatting,
including newlines terminating line comments. Do not flatten or trim those
newlines before appending a declaration terminator.
They are not guaranteed to be standalone declarations: this is syntax analysis,
not a C++ compiler, preprocessor, or name-resolution service.

Applications own record selection, deduplication, Arduino `setup`/`loop` policy,
tab order, generated includes, source maps, and editor publication. Definitions
inside another function body are not inventoried. Invalid or incomplete syntax
returns an error without partial records, allowing an editor to keep its last
good product output.

The source limit is 8 MiB, checked before parsing. The parser observes a
two-second cooperative deadline; this is not process containment or a hard CPU
deadline. Subsequent iterative traversal is limited to 131,072 node visits,
depth 256, and 16,384 output records. Signature traversal shares the visit budget.
These are traversal/output bounds, not independent parser allocation quotas.
Diagnostics do not contain source contents. No filesystem, runtime, or ambient
process state is created by analysis.

Issue #209 tracks contract expansion, platform verification, and FastLED
adoption. The initial implementation is not yet release-accepted.
