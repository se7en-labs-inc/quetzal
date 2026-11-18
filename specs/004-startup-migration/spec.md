# Automatic startup migration

## User stories

### US1: Open existing wallets with the existing launcher (P1)
Given `serve --database DIR` points to a stopped cardano-wallet database directory,
when Quetzal starts, it converts supported SQLite wallets automatically and makes
their complete imported state available through the existing API. No separate
migration command, recovery phrase, password or launcher option is required.

### US2: Restart and recover without losing wallets (P1)
Given an import completed, later startups reuse the published sled store without
reimporting SQLite or replacing subsequent Quetzal changes. Given import fails or
is interrupted before publication, source SQLite remains intact and startup does
not serve an empty wallet list; the next startup can retry.

## Requirements

- FR001 Detect wallet `.sqlite` files directly in the persistent database directory;
  exclude `stake-pools.sqlite`. Fresh and existing direct sled layouts remain usable.
- FR002 Use the validated migration engine, including native network/checkpoint
  checks, all supported families, encrypted credentials and reopen verification.
- FR003 Publish imported sled at `DIR/quetzal.sled` atomically without replacement;
  preserve SQLite and WAL/SHM files. Reuse this store on subsequent startups even
  if SQLite sources later disappear. Reject conflicting direct/nested sled layouts.
- FR004 Complete migration before API listeners and wallet workers start. Errors
  stop startup with actionable context; never silently initialize fresh state on
  migration failure or unusable existing managed state.
- FR005 Reuse pinned SQLite reader only for conversion. Runtime storage stays sled;
  no cardano-wallet or cardano-cli executable dependency is introduced.
- FR006 Concurrent imports may not overwrite a completed destination. Existing
  standalone inspection/check/import commands retain their behavior.

## Edge cases and scope

Temporary databases remain temporary. A directory containing only the pool cache
has no wallets to migrate. Unsupported schema, wrong network, stale checkpoint,
corrupt wallet, destination conflict and missing node fail explicitly. The source
backend must be stopped as before; this is a one-way migration, not synchronization.
The startup directory is the configured SQLite directory, not an arbitrary parent
or recursively discovered directory. Existing legacy direct sled plus SQLite is
rejected as ambiguous; startup must not replace either store automatically.

## Success criteria

Verified migration occurs through `quetzal serve`, all five imported API families
retain their snapshots, and restart preserves post-import changes without reading
SQLite again. Failure and concurrent-publication tests preserve source/destination.
Formatting, all-target strict Clippy and workspace tests pass through Nix.
