# State and command model

## Identity and revisions

`WalletKey = (ApiNamespace, WalletId)` distinguishes modern and Byron identities.
`WalletGeneration` changes on deletion/recreation. `StateRevision` monotonically
orders committed changes within a store. Use Rust newtypes, not loosely typed
tuples passed across the API. Durable generation/tombstone metadata prevents
restart or delayed work from reviving an earlier wallet incarnation.

## Storage commands

Explicit variants cover wallet creation/deletion/metadata/credentials, shared
templates, address discovery/allocation, pending/reserved inputs and signed
submissions, forget/expiry/confirmation, chain intersection/forward/rollback,
staking/rewards, pool catalogue/blocks/GC and settings. Inventory every existing
`WalletDb` mutation before claiming boundary coverage. Do not use arbitrary
unaccounted closures as the public command protocol.

Envelope fields: operation ID, namespace/wallet scope, expected generation and
revision where preparation depends on state, bounded owned payload, lane,
admission timestamp and response channel. Operation IDs are internal correlation
IDs; they do not add a new public idempotency contract.

Commands transition `waiting -> admitted -> executing -> committed -> published
-> acknowledged`, or fail before commit. Once committed, caller cancellation
cannot undo them. Local command acknowledgment additionally requires the flush
barrier. The writer validates constraints at execution time; compute preparation
is never authorization to commit against obsolete state.

## Read projections and indexes

`WalletSummaryView` contains generation/revision, wallet public metadata, balances,
assets, reservation effects and committed sync position. A namespace catalog
atomically publishes the set of summary handles for list calls. Readers hold
immutable snapshots; old snapshots are released when requests finish. Never
include encrypted or plaintext private-key material in public views.

History/address/UTxO queries carry filters, sort order and response bounds already
supported by the API. Derived ordered indexes retain existing tie-break and
filter semantics. Range queries verify one revision or execute through a bounded
consistent read barrier; they cannot combine old UTxOs and new pending state.
Keep scan work isolated for APIs that genuinely require complete output.

Persist a view/index schema version. Rebuild missing/outdated derived records
from canonical data before readiness, including on imported stores. A crash
after canonical commit but before view publication is recovered by rebuilding;
never roll back canonical funds to match a cache.

## Network observations

`NetworkObservation` retains the latest successful native snapshot, observation
time and refresh outcome. It is shared across public wallet namespaces. Last
success and current query failure are distinct; progress is evaluated against
current time using the existing interpreter. Cold start has no native snapshot.
Protocol parameters/era queries retain correct freshness requirements for
transaction preparation; they cannot use an obsolete snapshot silently.

## Migration state

Reuse feature 004's layout resolver and migration receipt: fresh/direct sled or
verified managed `quetzal.sled`; SQLite source with no existing state leads to
staging, validation, publication and reopen. Import failure never transitions to
fresh state. Migration owns exclusive offline database access; live writers and
view rebuild start only after state selection succeeds.

## Shared replay and discovery

`ReplayCohort` identifies a bounded stream, selected native point, chain generation
and participating wallet cursors. Membership never grants an unprocessed block
to a wallet. Live following and historical catch-up have separate bounded capacity.

`DiscoveryIndex` maps canonical address bytes and known UTxO outpoints to sets of
`WalletKey`/generation owners. Wallet-local discovery retains family-specific
state and role-specific sequential gap frontiers. Initialize from canonical
addresses/UTxOs on registration; extend on committed discovery/allocation and
remove on spend/deletion. Rollback/revision invalidation rebuilds or reverses
these derived structures before further matching. Secret discovery credentials
stay in protected compute state, never in shared public read projections.

`PreparedReplayBatch` carries contiguous native points, source generation and
wallet revision expectations, bounded decoded data and ordered per-block deltas.
Account for both candidate lookup entries and retained results. Discovery reaches
a fixed point over staged outputs before ledger-order application; publication
and cursor advancement occur only after ordered recoverable commit.
