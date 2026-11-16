# Feature specification: cardano-wallet SQLite state migration

**Branch**: `002-sqlite-state-migration`
**Created**: 2026-11-10
**Status**: Implementation and native verification
**Input**: “I need a way to migrate cardano-wallet sqlite state to quetzal sled state.”

## User scenarios

### P1: Preserve an existing wallet without its recovery phrase

Given an offline cardano-wallet database directory or wallet SQLite file, when the
user inspects and imports it into a new Quetzal database, the wallet keeps its
identifier, family, name, creation/passphrase timestamps, encrypted spending
credentials, account/policy keys, known/allocated addresses and shared templates.
Existing spending passwords still work. Watch-only wallets remain watch-only.
No plaintext private key or password is required or printed.

### P1: Continue wallet state safely

The user requires complete usable state for immediate startup: balances/native
assets, transaction history, pending signed bytes and resubmission provenance,
checkpoint and staking state. Do not substitute a mnemonic restore or discard
pending submissions/history.

Imported snapshots must have an explicit rollback boundary. A rollback below
available imported undo history must trigger safe recovery rather than retain
spent or orphaned UTxOs. Never fabricate a ready wallet or an authenticated key.

### P1: Preserve the source and publish atomically

Given source files (including a WAL), inspection and failed imports leave the
original bytes unchanged. A successful import publishes a complete, flushed
fresh destination once. An existing destination, source/destination overlap,
unknown schema, corrupt data or inconsistent source snapshot is an error.
The source remains available for returning to cardano-wallet.

## Functional requirements

- FR-001: Provide a Rust CLI with help, read-only inspection/dry-run and actual
  migration; support a wallet file and a directory containing several families.
- FR-002: Recognize actual versioned cardano-wallet schemas and filenames. Pin
  qualification to reference revision2fc5c383; older supported layouts/credential
  formats need explicit tests. Reject unsupported schemas before publication.
- FR-003: Handle SQLite WAL snapshots without ignoring committed WAL contents.
  Use a private snapshot of source files and detect source mutation; document
  stopping cardano-wallet before migration. Never run upstream migrations on
  the original database.
- FR-004: Preserve family namespaces, encrypted V1 PBKDF2/scrypt and V2 roots,
  watch-only accounts, discovery pools, pending change allocations, shared
  cosigners/templates and all wallet-local metadata.
- FR-005: Transfer complete chain state for immediate startup. Rescan only as
  recovery from a subsequent rollback below the imported snapshot boundary;
  retain all local state/submission bytes and report what is being rebuilt.
- FR-006: Verify identities, public keys, address formats, numeric bounds, CBOR,
  pending transaction hashes and relational invariants. Fail closed on unsupported
  semantic state, rather than silently skip a wallet or table.
- FR-007: Use private staging under the destination parent; flush and close all
  sled databases, verify reopening, and publish without replacing an existing
  destination. Never merge into an existing live Quetzal database.
- FR-008: Emit a non-secret migration receipt with source/schema identities,
  wallet/family counts, preserved/rebuilt state and verification outcome.

## Edge cases

Empty directories; stake-pool database alongside wallets; wallet filename/ID
mismatch; duplicate IDs within a family; the same ID across independent families;
unsupported future schema; malformed encrypted envelopes; wrong network;
unsupported change-address modes or gaps; missing private key; inactive shared
wallets; two policies sharing an asset name; pending/expired/confirmed/forgotten
local submissions; interrupted publication; permissions and destination races.

## Success criteria

1. Real SQLite files written by the pinned Haskell wallet import and reopen in
   Quetzal with equivalent supported public reads and spending-key authentication.
2. Native-funded migration preserves exact quantities and original pending IDs,
   survives Quetzal restart and supports independently confirmed spending.
3. Malformed/unsupported/faulted imports never publish a usable partial destination
   and never modify original source files or overwrite an existing destination.
4. Nix-shell formatting, strict Clippy, relevant tests, release and package checks
   pass. cardano-wallet-conformance remains untouched.
