# Owned temporary directories

The `fs` feature provides `platform::fs::TemporaryDirectory`, backed privately
by the already pinned temporary-file implementation. It does not expose the
backend guard or builder. `new()` selects OS temporary storage;
`in_directory(parent, prefix)` places staging under a trusted existing parent
so callers can subsequently rename on the same filesystem.

Prefixes are limited to 128 UTF-8 bytes and exclude control characters,
separators, dot/parent components and Windows filename/drive metacharacters.
The generated suffix has 16 characters. The pinned backend limits randomized
creation to 65536 collision attempts and never overwrites a live directory.
Unix directories request owner-only 0700 permissions at creation (subject to
umask); Windows uses the backend's inherited access-control behavior.

Paths become absolute at creation, so changing working directories does not
redirect cleanup. Drop performs best-effort recursive removal. `close()` reports
errors; it does not retry automatically after a failure. Save the path first if
explicit recovery is needed. `persist()` transfers the path and all cleanup
responsibility to the caller; cache validation, replacement and publication
remain product policy.

Creation and cleanup are synchronous native filesystem operations, without a
wall-clock or cancellation guarantee. Avoid dropping large trees on async worker
threads. This is not a path-security sandbox: parent replacement, external temp
cleaners and hostile changes to owned paths can violate lifetime assumptions.
Close open child handles before Windows cleanup. Directory names are collision
avoidance, not authorization tokens.

Issue #182 tracks native checks, application migration, release and exact
published adoption. Generic lifetime/permission/prefix tests live upstream;
FastLED must retain its cache-publication and download-policy integration tests.
