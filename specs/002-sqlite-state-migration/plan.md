# Implementation plan: SQLite to sled migration

## Scope and architecture

Implement a standalone Rust migration binary in this workspace. Keep SQL reading
isolated from the running wallet: use the Nix-provided sqlite3 executable on a
private copied snapshot in read-only mode. This adds no SQL Cargo dependency and
keeps sled the sole persistent wallet storage, satisfying constitution principle
VI while handling the source format explicitly requested by the user.

Reuse Quetzal's typed wallet/crypto/address/ledger persistence code. Add a narrow
validated import boundary rather than copying arbitrary sled tree internals into
CLI code. Stage a complete destination, flush/close/reopen for verification and
publish using Linux no-replace rename. Source credentials remain encrypted.

## Research anchors

Pinned upstream2fc5c383: DB/Sqlite/Schema.hs and Types.hs; private-key
PersistPrivateKey serialization; checkpoint/UTxO-history stores and TxOutCBOR;
submission and delegation stores. Actual owned reference schema5 SQLite files
from the preceding qualification are available for read-only inspection.

Findings: public keys are hex text inside BLOBs; addresses are raw-address hex;
V2 keys use V2: prefixes and may retain an old leading V1 segment; V1 Byron has a
payload suffix; latest UTxO history has delta tables in addition to older snapshot
tables. Pending submissions and confirmed/expired local submission provenance
are distinct. Copying a current checkpoint without usable rollback history is
unsafe under the existing Quetzal undo-based rollback implementation.

## Constitution check

Sled remains exclusive persistent destination; no SQL dependency in wallet/API.
Secrets stay encrypted/zeroized; errors do not dump rows. Source identity/version
and unsupported semantics are explicit. New migration behavior needs genuine
SQLite/reference/native tests plus failure-publication and rollback controls.
No conformance repository changes, automatic commits or pushes.

## Confirmed operational choice

The user requires complete state preservation for immediate startup: balances,
history, pending submissions and sync checkpoints transfer with credentials and
discovery state. A rescan is recovery only when a later rollback crosses the
imported snapshot boundary.

Funded-source correction (2026-11-16): SQLite discovery pools persist payment-only
fingerprint addresses for Shelley/shared wallets. Reconstruct their complete
addresses from account/reward keys and shared payment/delegation templates at
the persisted role/index. Verify payment credentials and network before lifting;
reject mismatches and noncanonical full source addresses. Keep strict UTxO
ownership and duplicate validation at the import boundary. Reuse shared address
derivation in the wallet library, and qualify all five funded reference families.

History projection correction: recover explicit validity bounds and expiry from
hash-validated retained transaction CBOR, rejecting conflicting stored expiry.
Preserve absent CBOR/Byron semantics and backward readability of existing sled
records. Use the same validity projection for imported, pending and newly synced
transactions. Format slot timestamps with the shortest exact UTC fraction. Keep
native public snapshot equality strict, then qualify restart and spending.
Whole-object parity also requires the reference's explicit extra-signatures array
and omission of absent metadata. Recover required signer hashes from ledger CBOR
for imported and newly produced records; do not drop nonempty signer lists.
