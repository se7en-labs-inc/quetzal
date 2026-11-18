# Startup migration implementation plan

Reuse feature002 conversion logic in a library in quetzal-migrate. Keep CLI parsing
in its binary and expose typed network/import options to callers. quetzal-api calls
the library in spawn_blocking, before binding sockets or opening WalletService.
The library's synchronous node runtime lives on that blocking thread; no nested
Tokio runtime, external migration executable or duplicated converter is needed.

Add a layout resolver for fresh, direct sled, SQLite and managed child states.
Use the existing staged, flushed, reopened conversion and no-replace publication
for the managed child. Allow this specifically validated child destination inside
the source directory while retaining standalone overlap rejection. Canonicalize
paths, reject ambiguous/conflicting layouts and symlinks. Detect direct sled by
its sled configuration marker; do not open/create a database during discovery.
Published child requires its receipt and sled marker before reuse.

Keep SQL isolated to the imported source reader and Nix-provided sqlite3; sled is
the only live store. Preserve existing network/genesis validation and encrypted
credential handling. This satisfies constitution financial safety, era awareness,
API compatibility, reproducible Nix builds, layering and storage requirements.
The API depends on the migration library, which depends only on lower layers.

Validate resolver/failure/restart behavior with unit tests and a real serve-driven
funded native migration integration. Reuse existing fixtures and qualification
artifacts without modifying the independent conformance checkout. Record exact
scope and any blocker; do not claim full preview replay performance qualification.
