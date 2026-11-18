# Migrating cardano-wallet SQLite state

Quetzal automatically migrates wallet SQLite state during persistent startup.
Migration
transfers encrypted credentials, wallet metadata, address discovery and change
allocations, balances and native assets, transaction history, local signed
submissions, staking state and the latest sync checkpoint. No recovery phrase or
spending password is needed. Existing passwords remain valid. Watch-only wallets
remain watch-only.

## Normal startup

Stop the source wallet backend and retain a backup of its database directory,
including any `-wal` and `-shm` files. Start Quetzal with the **same database
directory**, node socket and network arguments used by the launcher:

```sh
quetzal serve --database /data/wallets \
  --node-socket /run/cardano-node/node.socket \
  --testnet /path/to/preview/byron-genesis.json
```

If `/data/wallets` contains wallet `.sqlite` files, startup imports and verifies
them before opening API listeners or starting wallet workers. The matching node
must be running. A completed sled store is published at
`/data/wallets/quetzal.sled`; the original SQLite files remain unchanged. Later
starts with the same `--database /data/wallets` reuse the published store and
preserve subsequent Quetzal changes, even if the old SQLite files are archived.
No separate import command or spending password is required.

Migration errors stop startup with an error rather than serving an empty wallet
list. Interrupted unpublished stages do not count as a completed migration.
Existing incomplete `quetzal.sled` destinations and conflicting direct/managed
sled layouts are rejected; retain backups and resolve the authoritative store
before retrying. Do not delete a completed store to fix a startup error: it may
contain changes absent from the original SQLite source.

Fresh databases and existing sled stores directly at `--database DIR` continue
to work. Direct sled mixed with SQLite is rejected as ambiguous. A directory
containing only `stake-pools.sqlite` does not trigger a wallet migration. Symlinked
database layouts are rejected. Without `--database`, startup uses temporary empty
storage, as before.

## Explicit migration tools

`quetzal-migrate` remains available for inspection, a conversion dry-run or an
explicit import into a separate destination. It uses the same migration engine
as automatic startup. The qualification evidence directories record the actual
verification scope.

## Prepare

Stop cardano-wallet and retain a backup of its entire database directory,
including any `-wal` and `-shm` files. Keep Quetzal stopped during import. Use a
running node on the same network that retains the wallet's checkpoint. Migration
does not need the node to stop or a network connection to a wallet server.

The source may be one wallet `.sqlite` file or a database directory. A directory
imports all wallet `.sqlite` files and skips `stake-pools.sqlite`; the pool cache
is rebuilt by Quetzal from the node. The destination must not exist, and its
parent directory must already exist. Import never merges or overwrites data.

The Nix package includes SQLite for the standalone reader. Development commands
run inside `nix develop`; an alternate SQLite executable can be selected with
`--sqlite3 /absolute/path/to/sqlite3`.

## Inspect, check and import

```sh
quetzal-migrate inspect --source /backup/cardano-wallet

quetzal-migrate check --source /backup/cardano-wallet \
  --node-socket /run/cardano-node/node.socket --mainnet

quetzal-migrate import --source /backup/cardano-wallet \
  --destination /data/quetzal-migrated \
  --node-socket /run/cardano-node/node.socket --mainnet
```

For a test network, replace `--mainnet` with
`--testnet /path/to/byron-genesis.json`, using the same genesis as cardano-wallet.
The genesis hash/start and each imported checkpoint must match the selected
node. If a checkpoint is no longer canonical, first resume cardano-wallet against
that node, let it reconcile the chain, stop it, and take a new backup.

`inspect` reports source versions, wallet identities, names, row counts and file
hashes. `check` performs full conversion, writes temporary sled state, closes and
reopens it for verification, then removes it without publishing a destination.
`import` performs those checks and publishes the completed directory using a
Linux rename that cannot replace an existing destination. Both write operations
use private temporary directories. Original SQLite files are never opened by
SQLite: it reads only private copies, including committed WAL contents. Changes
to source files during copying cause an error.

After successful import, point the normal Quetzal `serve --database` option at
the new directory. Modern wallets live at its root and Byron/Icarus wallets in
its `byron` subdirectory; identical IDs in these API families remain independent.
`migration-receipt.json` records source hashes, network, checkpoint and imported
counts without private keys or spending passwords. Keep it with your backup.

## Source compatibility and recovery

The reader accepts SQLite schema version 5, qualified against cardano-wallet
revision `2fc5c383c94bb907deaa0cd27b1b407f86224b6c`. Supported discovery families
are Shelley, Icarus, random Byron, shared and watch-only sequential wallets.
Credential readers handle V2 envelopes and legacy PBKDF2/scrypt roots, including
the persisted random-wallet payload suffix. Schema versions outside this format
are rejected. Sequential wallets with different internal/external gaps or a
change-address allocation mode other than `increasing` are rejected.
Shelley/shared discovery fingerprints are reconstructed into complete addresses
using their persisted derivation paths and account/script templates, with payment
credentials and network checked. Shelley reward keys must match the supported
account delegation path `2/0`; inconsistent discovery or UTxO ownership is rejected.

Migration preserves the latest ledger snapshot. It does not manufacture Quetzal
undo records for older checkpoints. If a later native rollback crosses the
imported checkpoint boundary, Quetzal atomically discards derived ledger state
and replays from origin, retaining credentials, wallet metadata, address
allocations and retained local signed submissions. A later user-forgotten
submission is not resurrected. Ordinary rollbacks above the imported boundary
use Quetzal's normal undo journal.

The original cardano-wallet directory remains available for returning to that
backend. Subsequent changes in either backend are independent; migration does
not provide bidirectional synchronization or export from sled back to SQLite.

Retained historical CBOR is hash-checked before recovering explicit validity
bounds and transaction expiry, including confirmed history. Conflicting indexed
expiry is rejected. Records without retained CBOR retain their indexed expiry;
missing validity information is not invented. Public slot timestamps use the
shortest exact UTC decimal fraction, including subsecond development networks.
Unresolved historical inputs expose only their transaction ID and output index;
resolved inputs retain address, amount and assets, including empty asset arrays.

The funded native run in `evidence/20261116-funded-discovery` verifies exact public
snapshot equality for all five supported families, equality after same-database
restart, original pending confirmation and new spending independently checked
against the node. See its final receipt for the exact qualification scope.
