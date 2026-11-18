# Runtime boundary inventory

Inspected source: `84f2f4d691df3a32928766733def609164888475`, 2026-11-17.
This inventory describes the existing execution paths; it does not claim those
paths have been moved or qualified.

## Persistence operations

| Command family | Existing entry points | Required ordering/atomic boundary |
|---|---|---|
| Wallet creation | `wallet.rs` create_wallet_with_second_factor/create_watch_wallet/create_byron_wallet_style/create_shared_wallet; db insert_wallet/store_encrypted_key/store_account_xpub/store_script_template/insert_address | Duplicate check + credentials/templates/discovery + metadata must become one domain change; current creation is several writes |
| Deletion | wallet delete_wallet/delete_byron_wallet/delete_shared_wallet; db delete_wallet | Stop producer, invalidate generation, remove metadata/indexes and all family state; dropping trees is not a multi-tree transaction |
| Metadata/readiness | wallet update_wallet_name; db update_wallet/update_wallet_tip/mark_wallet_synced | Preserve latest chain fields/credentials; CAS/transaction against current metadata |
| Credentials | credentials.rs import/unlock/recover/change; db replace_credentials | Existing keys+metadata transaction compares encrypted key and flushes; expensive authenticate/KDF must happen outside writer |
| Address book | byron_addresses.rs create/import_random_addresses; shared.rs patch_shared_cosigners; db insert_address/import_addresses/mark_address_used/patch_shared_template | Allocation/ownership/generation check; atomic index+address writes; template CAS + discovery publication |
| Pending/change/submission | wallet submit_pending/delete_pending; pending.rs mark_pending_submission; db insert_pending_and_change/forget_submission/mark_pending_submission | Pending+change+provenance transaction then flush; forgetting retains allocated change; confirmation/expiry preserve provenance semantics |
| Chain | sync.rs process_intersection/process_roll_forward/process_roll_backward/mark_ready; db commit_chain_block_with_addresses/rollback_chain/reset_imported_chain | Existing dynamic tree transaction joins UTxOs/history/discovery/checkpoint/pending/staking/undo/metadata; preserve cursor and rollback barriers |
| Rewards/staking | wallet update_reward; staking.rs staking_state and credential/script derivation; db update_reward/staking_state | Reward updates must preserve current metadata; staking transitions are part of chain commit |
| Policy | policy.rs/wallet.rs policy key derivation/write; db store_policy_xpub | Generation-check persisted key; derivation outside writer |
| Pools | wallet update_pool_cache/observe_pool_block/rollback_pool_blocks; db upsert_pool/pool_block/rollback_pool_blocks | Existing production/registration counter transactions and rollback; catalogue updates cannot bypass writer |
| Settings/GC | wallet set_pool_metadata_source/store_pool_gc/pool_order_seed; db corresponding methods | Persist before response; seed uses CAS; source-bound GC result cannot replace newer configuration |
| Offline import | db import_wallet, import reset boundary; quetzal-migrate source/convert/publish | Exclusive offline import; keep separate from normal live clients; flush/close/reopen before no-replace publication |

Every production `self.db` caller occurs in wallet.rs, sync.rs, credentials.rs,
shared.rs, byron_addresses.rs, pending.rs, payment.rs, bootstrap.rs, balancing.rs,
policy.rs and staking.rs, plus db modules. Payment/bootstrap/balancing consume
credentials, templates and UTxOs while preparing expensive transactions. They
need revision-qualified inputs even where they do not write directly.

## Read paths and network waits

- Wallet list calls get_wallet repeatedly; wallet get_wallet recalculates balances
  by scanning UTxOs, address ownership and decoded pending transactions. Modern
  API list then queries each wallet's rewards (500 ms timeout each) and reads the
  wallet again. Detail also queries rewards before returning.
- History handler loads all history with `usize::MAX`, merges pending, resolves
  projections and filters time/address/withdrawal before max_count. Bounded
  requests still do unbounded preparation. Preserve filter-before-limit and
  ascending/descending tie-break semantics when adding indexes.
- Address endpoint has only state filtering; UTxO/statistics endpoints return the
  complete set. They have no pagination. Benchmark full responses separately;
  never add a query option or truncate these results to meet a page budget.
- Asset/key/stake/distribution handlers consume the same wallet/address/UTxO and
  credential paths; JSON, script, metadata encryption and serialization can be
  expensive even without disk I/O.
- Network information makes a new query with a three-second timeout. Parameters,
  network block, fees, transaction construction/submission, stake keys, migration
  planning, pools and settings proxy queries open additional node sessions.
  Parameter freshness and definitive submission results must stay explicit.
- Background DRep and pool catalogue/GC refresh also call network/HTTP services.
  Their persistence and CBOR/JSON work must use bounded clients.

## Locks, runtime and submission outcomes

- `workers.rs::start_wallet_worker` owns the registry mutex while checking and
  installing handles; stop removes handles before awaiting completion. Keep the
  registry management short. Do not acquire database or admission locks inside it.
- Global-per-namespace submission mutex spans selection, persistence and network
  relay in transaction handlers and retries. Replace only with an equivalent
  reservation/ordering protocol; simply shortening this lock permits races.
- Shared-template and random-address mutexes serialize local discovery changes.
  They cannot be removed without writer-side generation/revision validation.
- Network snapshot/tip/config locks are short, but network observations are split
  across native snapshots and replay tips; retain native readiness semantics.
- API, global sync and per-wallet/retry tasks currently share `#[tokio::main]`.
  Synchronous `WalletService` work inside async functions does not yield.
- Submission saves/flushed pending state before relay. Definitive nonretryable
  `TxRejected` removes pending; retryable rejection or uncertain transport failure
  keeps it for retry. Forgetting confirmed history fails; forgotten submissions
  stay forgotten across deep imported rollback. No automatic retry of arbitrary
  timed-out API mutations may be added.

## Capacity and feasibility decisions

- JSON extractors use the existing axum default body limit; router installs no
  custom limit. Raw body and complete read responses require separate accounting.
  Do not infer that request-body limits bound stored wallet data or query output.
- Sled dynamic tree transactions are already exercised in chain/import code.
  Atomic closures must contain no node relay, expensive key work or publication.
  Tree deletion needs logical tombstoning before deferred physical cleanup.
- Range iterators are not treated as snapshots. All mutations increment an
  execution revision; range reads fence that revision, retry within a bounded
  budget, or use a consistent read barrier. Summary publication uses immutable
  committed views and therefore remains independent of writer stalls.
- Oversized valid jobs use exclusive, bounded producer admission with chunked
  preparation/spooling and streamed output; never enlarge queues automatically.
  Atomic ledger changes remain atomic even if prepared in chunks. Implementation
  must account for iterator buffers, output backpressure and retained view owners.

## API overload decision

The current API maps unclassified wallet failures to HTTP 500 `internal_error`.
The pinned swagger lists specific error alternatives per endpoint and has no
general overload status for all ordinary reads (its lone explicit 500 entry is
the shared-wallet missing-reward-account case). A new generic 429/503 cannot be
called part of that pinned OpenAPI contract. On 2026-11-17 the user explicitly
selected HTTP 503 `service_unavailable` for queue overload, using the existing
`{code,message}` JSON envelope. This is an approved additive overload response,
not a change to normal success/error semantics or permission to alter the
external conformance suite. Document it in Quetzal and test it locally.
