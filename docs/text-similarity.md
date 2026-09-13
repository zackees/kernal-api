# Bounded name similarity

Enable `text-similarity` and call `kernal_api::text::name_similarity(left, right)`.
The facade privately pins strsim 0.11.1 for Jaro-Winkler scoring. It returns a
finite `f64` from 0 to 1, or a facade-owned resource error. No backend types are
public. This feature does not activate GUI, HTTP or sketch-host facilities.

The fixed limits are 4096 UTF-8 bytes per input and a product of Unicode scalar
counts no greater than 1,048,576. Byte length is checked first, then scalar work,
before backend scoring or allocation. Even equal strings must satisfy these
limits. Limits are not configurable, so there is no invalid-configuration state.
This bounds comparison work and scratch storage, not a wall-clock deadline.
The function is synchronous and needs no runtime or cancellation mechanism.

Scoring is case-sensitive and does not normalize Unicode or combine grapheme
clusters. Two empty strings score 1; exactly one empty string scores 0. The
algorithm gives matching prefixes a boost. Applications keep case folding,
substring preference, candidate enumeration, tie tolerance and user prompts.
Oversized input must be reported explicitly; do not truncate or substitute a
different score after a resource error.

Issue #184 tracks FastLED adoption and exact published-release verification.
Removing a direct application dependency does not itself remove transitive
compilation or demonstrate a build-speed improvement.
