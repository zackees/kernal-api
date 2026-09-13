# SQLite facade

Enable `kernal-api`'s `sqlite` feature for synchronous SQLite mechanics. The
facade is deliberately not a schema, migration, pool, or SQL-policy layer:
applications own those decisions and must execute a `Connection` on a kernel
blocking worker, never an async executor thread.

`Connection::open` creates a read-write database with WAL mode, foreign keys,
and a 100 ms busy timeout. `open_read_only` never creates the main database,
schema, or migrations, though SQLite may create or update adjacent WAL/shm
bookkeeping when its directory is writable. Callers that need a longer bounded wait may use the corresponding
`*_with_busy_timeout` constructor up to `MAX_BUSY_TIMEOUT` (five seconds).
Read-only SQLite connections may still observe WAL/shm bookkeeping files
created by writers; the facade itself performs no initialization for them.

Every statement is prepared and accepts facade-owned `Value` bindings. Queries
require `QueryLimits`; the default permits 1,000 rows and 1 MiB of logical
owned result memory. The byte budget includes facade row/value structs and
payload bytes before copying, but not `Vec` spare capacity, allocator
bookkeeping, or SQLite transient memory. A cap failure returns an error rather
than a truncated result.

`begin_immediate` has writer exclusion. A transaction rolls back on drop and
on a facade operation error; afterward it rejects further operations. Use
`commit` to retain its work. Busy/locked failures are classified as `Error::Busy`.

`checkpoint` and `integrity_check` expose maintenance mechanics. `backup_to`
uses SQLite's online backup API so WAL state is included. It permits a one-second
total retry window and at most ten consecutive busy/locked retries (successful
page batches reset that retry counter). It writes to a private
same-filesystem staging directory, syncs a closed standalone target, then
publishes with a no-replace hard link. The destination must not exist and must
support hard links; existing files, including source aliases, are never
overwritten. A relative backup destination is resolved relative to the current
working directory.

The exact bundled `rusqlite` 0.40.2 graph currently resolves `libsqlite3-sys`
0.38.2 and SQLite 3.53.2; the facade regression test requires a runtime SQLite
version at least 3.51.3, the WAL-reset corruption fix level.
