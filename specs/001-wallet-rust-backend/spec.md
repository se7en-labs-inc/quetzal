# Feature Specification: Quetzal — Cardano Wallet Backend in Rust

**Feature Branch**: `001-wallet-rust-backend`
**Created**: 2026-08-22
**Status**: Public serve/runtime/registry/sync/EOF implementation complete; current independent qualification partial; historical transport qualification 646/646, 38 profiles; deferred fixtures and actual Daedalus use remain open
**Project**: Quetzal (se7en-labs-inc/quetzal)

## Overview

### Complete reference server compatibility — 2026-11-10

The user requires a usable cardano-wallet replacement for Daedalus, including
the complete pinned public `serve` command and its runtime effects. Implement
the consumer-independent T111 / FR-019–024 contract in the external read-only
conformance checkout (currently652 obligations/44 profiles). Preserve the
existing EOF implementation and all historical receipts with their original
fingerprints. No flag stripping, compatibility wrapper, ignored options or
candidate-specific genesis filename convention may earn compatibility credit.

- Match explicit mainnet/testnet selection, `--listen-address`, fixed/random API
  and UI listeners, database/socket options, TLS, duration parsing/defaults,
  token/pool metadata sources, IPFS gateway and reference logging controls.
  Missing/malformed/conflicting inputs fail explicitly. Help advertises the
  implemented public inventory. Preserve legacy Quetzal aliases where harmless.
- Honor the supplied Byron genesis without requiring a neighboring specially
  named Shelley file; use native era information for slot/epoch timing.
- Report actual connected-node tip and reference-derived wall-time sync progress,
  including numeric percent while syncing, configured tolerance and disconnect
  behavior. Per-wallet/service replay catch-up must not manufacture network-ready.
- Query the configured registry via pinned POST metadata/query. Match required
  and optional metadata validation, policy/name isolation and graceful missing,
  malformed/unavailable/disabled results. Keep ledger quantities unscaled.
- Preserve latest acknowledged metadata and pending submissions across actual
  EOF and same-database restart with TLS and complete public launch settings.
- Measure service-follower initialization/restart/reconnection separately from
  wallet checkpoints against the reference on the same hash-pinned representative
  native fixture. The current external policy requires100000blocks/1GiB and a
  genuinely historically funded wallet. Missing fixtures remain explicit gaps;
  a tiny cluster or source review cannot qualify that workload.

Acceptance includes real reference/candidate native/registry executions and a
ready-to-test release binary with the unchanged public command. Consumer source
and UI builds are not prerequisites under the corrected external specification.
The user deferred supplying mainnet/lagging/ahead and representative-history
fixtures for this iteration; their missing qualifications must remain explicit.

### Continuation — 2026-11-06

The project is renamed Quetzal. Continue the existing wallet implementation and
preserve in-progress changes. The acceptance contract is the reviewed executable
suite in `../../cardano-wallet-conformance/master`: the corrected contract has
646 active obligations across 38 transport, HTTP, internal-vector and owned real-node profiles. The V58 643/37 receipts qualify only the historical contract. Supply a thin adapter
calling actual wallet internals; unsupported operations fail explicitly. Passing
unit tests or a subset of profiles does not establish full conformance. Retain
fresh reports and report missing coverage separately from failures.

Quetzal reimplements the Cardano wallet backend in Rust. Its API target is all 78 paths (96 operations) in `swagger.yaml`, with the reviewed Haskell behavior corrections in the independent conformance contract. This includes Shelley, Byron and shared wallets, stake pools, governance, chain data, SMASH and network endpoints. Daedalus is the intended consumer; endpoint parity alone does not establish a drop-in replacement. Authenticated transport and a pinned actual-client launch-and-use acceptance run are required before claiming Daedalus compatibility. Existing Haskell wallet databases are not imported; supported wallets are restored from their recovery material into a fresh Quetzal database.

### Transport correction — 2026-11-09

1. With all three `--tls-ca-cert`, `--tls-sv-cert` and `--tls-sv-key` paths, serve the API over TLS and require a client certificate with client-auth purpose chained to the supplied CA. Reject absent, foreign-CA and wrong-purpose credentials during TLS negotiation.
2. A trusted client must receive a schema-valid API response. Strict clients must validate server trust and hostname. Plaintext must not reach API handlers or redirect to them on the TLS listener.
3. Missing/unreadable files, malformed PEM, empty client trust, mismatched server certificate/private key or partial TLS arguments must fail startup before wallet workers start; never fall back to HTTP. Clients remain responsible for verifying server trust, validity dates, purpose and hostname during negotiation.
4. Bind to loopback by default. Permit an explicit IP bind via `--host`; unauthenticated HTTP must remain confined to loopback. HTTP fixture mode is separate from the secure launch and cannot earn transport credit.
5. Preserve the same router and wallet behavior in HTTP and TLS modes, with bounded TLS handshakes. Own and stop all test services. Keep fixture keys and runtime databases private.
6. Run the corrected independent suite without modifying its repository. Record fresh source/binary identities, configuration, coverage and cleanup; do not relabel V58 evidence. Actual Daedalus startup and performance remain separate, unqualified acceptance items until exercised.

---

## User Scenarios & Testing

### Parent-controlled shutdown — 2026-11-10

The external contract's T110 / FR-017–018 requires the literal valueless
`--shutdown-handler` flag as an exception to general CLI compatibility exclusions.
With this flag, read and discard stdin bytes until EOF. An open pipe and ordinary
input must leave HTTP/TLS serving; EOF, including immediate EOF, starts clean
shutdown. Without the flag, stdin EOF has no lifecycle effect.

Clean shutdown stops accepting connections, drains admitted requests with a
bounded grace period, cancels TLS handshakes and wallet/background workers, flushes
Shelley/shared and Byron stores, releases listeners and exits successfully within
the reference 30-second deadline. SIGTERM and Ctrl-C use the same cleanup path;
they are separate from EOF acceptance. Cleanup/storage errors must fail the process
rather than report successful clean exit. Avoid an uncancellable Tokio stdin read
that prevents runtime teardown on another shutdown trigger.

Verify actual owned HTTP and mutual-TLS subprocesses with open-pipe, ordinary-byte,
EOF, immediate-EOF and flag-absent controls, stalled handshakes, bounded successful
exit, released ports and same-database wallet identity/keys/addresses recovery.
Funded confirmed/pending state acceptance needs independent native observations;
mock or empty-wallet recovery does not qualify those cases. The external executable
646/38 catalogue does not yet implement T110; its prior receipts cannot qualify
this extension or a new binary by association.

### User Story 1 — Wallet Lifecycle Management (Priority: P1)

A Daedalus user can create a new Shelley wallet from a freshly generated mnemonic, restore an existing wallet from a known mnemonic and passphrase, view a list of all their wallets with current balance and sync status, and permanently delete a wallet they no longer need.

**Why this priority**: Without wallets there is no product. Every other story depends on a wallet existing and being accessible via its ID.

**Independent Test**: Create a Shelley wallet via `POST /wallets`, verify its reference identity and public sync state, list it via `GET /wallets`, then delete it and confirm it no longer appears. Readiness may be immediate within the reference sync tolerance; it does not replace historical restoration checks.

**Acceptance Scenarios**:

1. **Given** no wallets exist and the new mnemonic has no historical funds, **When** a user POSTs it with a passphrase to `/wallets`, **Then** a wallet is created with its deterministic ID, zero balance and the reference-consistent sync state.
2. **Given** recovery material for a wallet absent from this service, **When** the user POSTs it to `/wallets`, **Then** the wallet is restored with its original deterministic ID and recovers historical funds. An already-present same-family wallet instead returns 409.
3. **Given** a wallet ID, **When** the user sends `DELETE /wallets/{walletId}`, **Then** the wallet and all associated data are removed and subsequent `GET /wallets/{walletId}` returns 404.
4. **Given** an invalid or short mnemonic, **When** the user attempts to create a wallet, **Then** the request is rejected with a clear error message identifying the validation failure.
5. **Given** multiple wallets, **When** the user calls `GET /wallets`, **Then** all wallets are returned with accurate balances and sync states.

---

### User Story 2 — Address Derivation and Discovery (Priority: P2)

A Daedalus user can retrieve a list of unused and used addresses for their Shelley wallet, with the service automatically deriving new addresses as needed and discovering addresses that have received funds during chain synchronisation.

**Why this priority**: Address management underpins receiving funds, which is a core wallet operation.

**Independent Test**: Call `GET /wallets/{walletId}/addresses` and confirm addresses are returned in the Bech32 format with correct network tag; after simulating a receive transaction in integration tests, confirm the address is marked "used".

**Acceptance Scenarios**:

1. **Given** a synced wallet, **When** the user requests `GET /wallets/{walletId}/addresses`, **Then** at least 20 addresses are returned in Shelley Bech32 format, each with a `state` field indicating "used" or "unused".
2. **Given** all current addresses are used, **When** new addresses are needed, **Then** the service derives additional addresses automatically without user intervention.
3. **Given** a Byron wallet restored from an Icarus mnemonic, **When** the user requests addresses, **Then** addresses are returned in the Byron base58 format.
4. **Given** an address filter parameter, **When** the user requests only "unused" addresses, **Then** only unused addresses are returned.

---

### User Story 3 — Balance and UTxO Queries (Priority: P3)

A Daedalus user can view the total confirmed and unconfirmed balance of their wallet, inspect the UTxO distribution, and query the full UTxO set.

**Why this priority**: Balance display is the most frequently used feature in any wallet UI.

**Independent Test**: After a chain sync simulation that includes a UTxO for the wallet's address, `GET /wallets/{walletId}` returns a non-zero balance and `GET /wallets/{walletId}/statistics/utxos` returns a UTxO distribution histogram.

**Acceptance Scenarios**:

1. **Given** a fully synced wallet with a known UTxO, **When** the user queries `GET /wallets/{walletId}`, **Then** the response includes `balance.available`, `balance.total`, and `balance.reward` fields in lovelace.
2. **Given** a wallet with multiple UTxOs, **When** the user requests `GET /wallets/{walletId}/statistics/utxos`, **Then** a UTxO distribution histogram is returned showing the distribution of UTxO sizes.
3. **Given** a wallet with unconfirmed transactions, **When** the user queries balance, **Then** available balance excludes in-flight UTxOs and total balance includes them.

---

### User Story 4 — Transaction Construction, Signing, and Submission (Priority: P4)

A Daedalus user can construct a payment transaction to one or more recipients, get a fee estimate before signing, sign the transaction with their passphrase, and submit it to the Cardano network.

**Why this priority**: Sending funds is the primary financial operation a wallet performs.

**Independent Test**: POST a payment and passphrase to `/wallets/{walletId}/transactions`; require a 202 transaction record and independently confirm its ID on the node. Separately construct through `/transactions-construct`, sign the returned bytes through `/transactions-sign`, submit through `/transactions-submit`, and verify the same signed transaction confirms exactly once.

**Acceptance Scenarios**:

1. **Given** a funded wallet, **When** the user POSTs a payment and passphrase to `/wallets/{walletId}/transactions`, **Then** the wallet creates, signs and submits the payment and returns a 202 transaction record with its fee and ID; node inclusion and resulting balances are checked independently.
2. **Given** a valid unsigned transaction, **When** the user POSTs to `/wallets/{walletId}/transactions-sign` with their passphrase, **Then** the transaction is returned with valid witness signatures.
3. **Given** a signed transaction, **When** the user POSTs to `/wallets/{walletId}/transactions-submit`, **Then** the transaction is forwarded to the Cardano node and a transaction ID is returned.
4. **Given** insufficient funds, **When** the user attempts a payment exceeding their balance, **Then** the request is rejected with an error indicating the shortfall amount.
5. **Given** a payment request, **When** the user calls `POST /wallets/{walletId}/payment-fees`, **Then** a fee estimate with minimum and maximum is returned without constructing the transaction.
6. **Given** a multi-output transaction, **When** the user specifies multiple recipients, **Then** coin selection distributes inputs to cover all outputs and the change output.
7. **Given** a submitted payment temporarily rejected because a competing transaction occupies the node mempool, **When** the competing transaction expires, **Then** the wallet retries the unchanged signed transaction on the reference schedule and observes its confirmation. Pending state and the latest submission slot survive a process restart. Forgetting or expiring a transaction stops retries.

---

### User Story 5 — Transaction History (Priority: P5)

A user can view transaction history in the reference order, with supported time-range, order, maximum-count, address and withdrawal filters. There is no public status selector in the pinned API; pending and expired entries are checked in mixed history explicitly.

**Why this priority**: Transaction history is essential for financial record-keeping.

**Independent Test**: After simulating inbound and outbound transactions in integration tests, `GET /wallets/{walletId}/transactions` returns a list ordered by most-recent-first, with correct amounts, fees, and statuses.

**Acceptance Scenarios**:

1. **Given** a wallet with transaction history, **When** the user calls `GET /wallets/{walletId}/transactions`, **Then** transactions are returned newest-first with amount, fee, inputs, outputs, and status fields.
2. **Given** mixed confirmed, pending and expired history, **When** the user supplies supported `start`, `end`, `order` or `max_count` parameters, **Then** list/get results match the pinned reference, including unconfirmed ordering and limits; do not invent offset/cursor pagination or a status query.
3. **Given** an unconfirmed transaction, **When** the user queries history, **Then** the transaction appears with status "pending" until it is confirmed on-chain.
4. **Given** a confirmed transaction, **When** the user queries `GET /wallets/{walletId}/transactions/{transactionId}`, **Then** full transaction detail including block height and slot is returned.

---

### User Story 6 — Chain Synchronisation (Priority: P6)

The Quetzal service automatically synchronises with the Cardano node via the node-to-client miniprotocols, tracking the current chain tip and updating wallet state as new blocks arrive.

**Why this priority**: All wallet data depends on chain sync; without it balances and history are stale.

**Independent Test**: Start Quetzal pointing at a running cardano-node socket; after a configurable timeout the service reports sync progress > 0% via `GET /network/information`.

**Acceptance Scenarios**:

1. **Given** a running Cardano node and a configured socket path, **When** Quetzal starts, **Then** it intersects a valid persisted checkpoint or restores from genesis when no checkpoint is available. Recreating a previously funded wallet must recover historical addresses, transactions and exact funds; following only new tip blocks is insufficient.
2. **Given** an active chain-sync session, **When** a new block is received, **Then** wallet UTxO sets and transaction histories are updated atomically before the next block is processed.
3. **Given** a node restart or connection drop, **When** the connection is lost, **Then** Quetzal reconnects automatically and resumes from its last known checkpoint without data loss.
4. **Given** a synced service, **When** the user queries `GET /network/information`, **Then** it receives the reference network/node tip and sync-state shape. Network parameters are available through `/network/parameters`; a ready state has no progress field.
5. **Given** a wallet checkpoint within the reference default five-minute wall-clock sync tolerance, **When** its public state is rendered, **Then** it reports `ready` without a progress field. Older checkpoints report their elapsed-time percentage. This projection does not advance or alter the checkpoint or balances.

---

### User Story 7a — Byron Wallet Support (Priority: P7a)

A Daedalus user with existing Byron-era ADA can restore a Byron wallet (random or Icarus), view addresses and balance, send transactions, and migrate funds to a Shelley wallet.

**Why this priority**: Byron wallet holders need a migration path; required for full API parity.

**Independent Test**: Restore funded random and Icarus wallets; require base58 addresses, POST a migration plan with destination addresses, execute it through POST migrations and independently confirm the resulting transfers and history.

**Acceptance Scenarios**:

1. **Given** a Byron Icarus mnemonic, **When** the user POSTs to `/byron-wallets`, **Then** a Byron wallet is created with correct base58 addresses.
2. **Given** a funded Byron wallet, **When** the user POSTs destination addresses to `/byron-wallets/{id}/migrations/plan`, **Then** a plan with selections and fees is returned without submitting transactions.
3. **Given** valid destinations and a passphrase, **When** the user POSTs to `/byron-wallets/{id}/migrations`, **Then** the wallet creates and submits the migration transactions; returned records, exact native transfers and remaining balance must reconcile, including best-effort partial migration.

---

### User Story 7b — Shared (Multi-Sig) Wallets (Priority: P7b)

A Daedalus user participating in a multi-signature wallet can create or restore a shared wallet using a script template, view the shared wallet's addresses and UTxO, construct and sign transactions that require multiple signatories, and track which cosigners have signed.

**Why this priority**: Required for full Daedalus API parity; multi-sig is an increasingly common use case.

**Independent Test**: `POST /shared-wallets` with a payment script template creates a shared wallet; `GET /shared-wallets/{id}/addresses` returns script addresses; `POST /shared-wallets/{id}/transactions-construct` returns an unsigned tx awaiting cosigners.

**Acceptance Scenarios**:

1. **Given** a payment script template with cosigner keys, **When** the user POSTs to `/shared-wallets`, **Then** a shared wallet is created with a unique wallet ID and script-derived addresses.
2. **Given** a funded shared wallet, **When** the user constructs a transaction via `/shared-wallets/{id}/transactions-construct`, **Then** an unsigned transaction is returned that requires the script-specified signatures.
3. **Given** a shared transaction, **When** each cosigner signs the returned bytes via `/shared-wallets/{id}/transactions-sign`, **Then** existing witnesses and the body identity are preserved and returned witnesses can be merged until the script threshold is met. Do not assume hidden server-side accumulation.
4. **Given** a fully-witnessed shared transaction, **When** submitted via `/shared-wallets/{id}/transactions-submit`, **Then** it is accepted by the node.

---

### User Story 7c — DRep Governance and Voting Delegation (Priority: P7c)

A Daedalus user can browse available DReps, delegate their vote to a DRep, view suggested DReps, and query delegation status — all in the Conway governance era.

**Why this priority**: Conway era governance is live; Daedalus must surface DRep delegation.

**Independent Test**: `GET /dreps` returns a list of registered DReps; `PUT /dreps/{drepId}/wallets/{walletId}` returns a vote delegation transaction.

**Acceptance Scenarios**:

1. **Given** a synced Conway-era service, **When** the user calls `GET /dreps`, **Then** registered DReps are returned with their metadata and stake.
2. **Given** the pinned suggested-DRep behavior, **When** the user calls `GET /dreps/suggested`, **Then** the result matches that behavior; do not invent a stake-ranking policy.
3. **Given** a wallet with ADA and a valid passphrase, **When** the user delegates votes via `PUT /dreps/{drepId}/wallets/{walletId}`, **Then** a vote delegation transaction is signed and submitted, a 202 record is returned, and native confirmation and delegation state are verified.

---

### User Story 7 — Stake Pool Operations and Delegation (Priority: P7)

A Daedalus user can list available stake pools, delegate their wallet's stake to a pool, and withdraw staking rewards.

**Why this priority**: Staking is a key value-add over a basic payment wallet; required for full Daedalus feature parity.

**Independent Test**: `GET /stake-pools` returns a list of pools with metadata; `PUT /stake-pools/{poolId}/wallets/{walletId}` returns a delegation transaction.

**Acceptance Scenarios**:

1. **Given** a synced service, **When** the user calls `GET /stake-pools`, **Then** known stake pools are returned with their ticker, name, saturation, and estimated ROA.
2. **Given** a wallet with ADA, **When** the user delegates with a passphrase via `PUT /stake-pools/{poolId}/wallets/{walletId}`, **Then** the transaction is signed and submitted, a 202 transaction record is returned, and native confirmation and subsequent delegation state are verified.
3. **Given** a wallet with accumulated rewards, **When** the user queries `GET /wallets/{walletId}`, **Then** `balance.reward` reflects the current reward balance in lovelace.
4. **Given** accrued rewards, **When** the user submits a withdrawal transaction, **Then** rewards are swept into the wallet's available balance.

---

### Edge Cases

- What happens when a node socket is unavailable at startup? Service starts, reports degraded status via `/network/information`, and retries connection automatically.
- How does the system handle restoring the same mnemonic twice in one wallet family? Return 409 `wallet_already_exists`, preserving the existing wallet. Delete/recreate restores its deterministic identity; different API families may independently contain the same public ID.
- What happens during a Cardano hard fork / era transition? Chain-sync resumes correctly; era-specific block decoders switch automatically.
- How are transactions with reference scripts shown in history? Decoded and displayed; script execution is not re-run by the wallet.
- What if the sled database is corrupted on startup? Service fails with a clear error and refuses to start rather than operating on bad state.
- How are large wallets (>100k addresses) handled? Address discovery gap limit is configurable; default is 20 sequential unused addresses.
- What happens when confirmed transactions are rolled back? Undo all affected ledger mutations, including restoring older spent inputs. Preserve local submission provenance: valid local transactions return to pending with their unchanged bytes and retry schedule; discovered-only transactions disappear. Expire only according to validity bounds. Reapplication must consume inputs and credit outputs exactly once.

---

## Requirements

### Functional Requirements

- **FR-001**: System MUST implement all 96 operations across the 78 pinned API paths, including exact status, media, errors, identity, values, ordering and observable side effects under the reviewed reference corrections.
- **FR-002**: System MUST support the pinned wallet-family mnemonic lengths, checksums, optional mnemonic second factor and public-account-key restoration. Mnemonic derivation is independent of storage encryption and must reproduce reference keys and addresses.
- **FR-003**: System MUST derive Shelley (CIP-1852), Icarus, and Byron address types from the appropriate key hierarchies.
- **FR-004**: System MUST use family-appropriate sequential or random address discovery. Sequential discovery has configurable gap limit, default 20, and must recover historical funds independently of live readiness reporting.
- **FR-005**: System MUST maintain a per-wallet UTxO set updated atomically as blocks are processed.
- **FR-006**: System MUST implement the pinned reference coin-selection behavior for the supported operations, including exact seeded component results, eligible inputs, conservation, change, collateral, fee accounting and structured shortfalls. Strategy labels alone are insufficient acceptance evidence.
- **FR-007**: System MUST construct, balance and sign transactions in the qualified Conway era and explicitly reject unsupported writing eras. Future eras require independent qualification, not an automatic compatibility claim.
- **FR-008**: System MUST submit signed transactions to a connected cardano-node via the local-tx-submission miniprotocol.
- **FR-009**: System MUST persist all wallet state, UTxO sets, transaction history, and chain checkpoints using embedded storage.
- **FR-010**: System MUST maintain per-wallet transaction history with slot-ordered range query support.
- **FR-011**: System MUST synchronise with a Cardano node via the node-to-client chain-sync miniprotocol.
- **FR-012**: System MUST resume chain sync from the last persisted checkpoint after restart without re-downloading the full chain.
- **FR-013**: System MUST expose stake pool listing, delegation, and reward withdrawal via the REST API.
- **FR-014**: System MUST support passphrase-protected wallet keys with secure in-memory handling.
- **FR-015**: System MUST return API error responses in the cardano-wallet JSON error format for Daedalus compatibility.
- **FR-016**: System MUST expose `/network/information`, `/network/parameters`, and `/network/clock` endpoints.
- **FR-017**: System MUST handle chain rollbacks by reverting affected wallet state to the last valid checkpoint.
- **FR-018**: System MUST support Byron wallet creation and restoration (random and Icarus styles) via `/byron-wallets/*` endpoints.
- **FR-019**: System MUST support the full Byron wallet API surface: addresses, transactions, payment fees, UTxO stats, coin selection, migrations, and passphrase management.
- **FR-020**: System MUST support shared (multi-sig) wallet creation with native script payment and delegation templates via `/shared-wallets/*` endpoints.
- **FR-021**: System MUST support multi-party transaction signing for shared wallets: construct → each cosigner signs → submit when threshold met.
- **FR-022**: System MUST expose DRep listing, suggested DReps, DRep details, and vote delegation transactions via `/dreps/*` endpoints.
- **FR-023**: System MUST expose `/blocks/latest/header` returning the current chain tip block header.
- **FR-024**: System MUST expose `/smash/health` proxying to a configured SMASH server.
- **FR-025**: System MUST expose `/proxy/transactions` for raw CBOR transaction submission.
- **FR-026**: System MUST provide mandatory client-authenticated TLS when configured, fail closed for invalid TLS inputs and default to loopback binding as specified above.

### Key Entities

- **Wallet**: A named collection of keys derived from a mnemonic, with an associated UTxO set, address book, and transaction history.
- **UTxO**: An unspent transaction output identified by `(TxHash, OutputIndex)` with a value (lovelace + native assets) and an address.
- **Transaction**: An on-chain event affecting a wallet's UTxO set; includes inputs consumed, outputs created, fee paid, slot confirmed, and metadata.
- **Address**: A derived payment or staking address belonging to a wallet, in Shelley Bech32 or Byron base58 format.
- **Checkpoint**: A persisted `(SlotNo, BlockHash)` pair representing the last processed block for chain-sync resumption.
- **StakePool**: An on-chain stake pool with ID, metadata, saturation, and performance metrics.

---

## Success Criteria

### Measurable Outcomes

- **SC-001**: A fresh full execution of the current independent contract passes all 652 obligations and 44 profiles with unchanged identities and owned cleanup. Historical or mock-only evidence cannot satisfy this criterion. The earlier646/38 result remains a historical transport qualification.
- **SC-002**: Target, not current qualification: create/fund/send within 10 seconds excluding chain sync. Define the measurement boundary, faucet and node-inclusion conditions before asserting this on preprod; external faucet/network delays cannot be hidden.
- **SC-003**: Target, not current qualification: at least 500 blocks per second. A benchmark must pin hardware, era/block corpus, wallet count/address discovery and persistence settings before measuring.
- **SC-004**: Unit and property test coverage covers all domain functions in the primitives, crypto, and address crates.
- **SC-005**: Integration acceptance covers every wallet story, including historical restoration, shared signing, Byron migration, governance, restart, rollback, retry, era transitions and authenticated transport.
- **SC-006**: A separately recorded benchmark must define hardware, dataset, request mix, duration and latency/error thresholds before measuring 100 concurrent clients. This target is unqualified until that benchmark is agreed and run.
- **SC-007**: Wallet state is recoverable after a crash at any point during block processing — no partial-block corruption.
- **SC-008**: Target, not current qualification: peak process RSS below 512 MB for a 10,000-address/50,000-transaction wallet under a pinned workload and observation interval. Record corpus, assets, concurrency and measurement method before claiming success.

---

## Assumptions

- Daedalus requires mutually authenticated TLS. Actual launch arguments, certificate provisioning, platform support and a pinned client create/restore/read/send workflow require separate acceptance; no drop-in claim is made before that run.
- All 78 paths in `swagger.yaml` are in scope, including Shelley wallets, Byron wallets, shared/multi-sig wallets, stake pools, DRep governance, and chain data endpoints.
- The cardano-node socket path is provided via configuration; Quetzal does not launch the node.
- Plutus phase-2 evaluation is out of scope. Reading/decoding script-bearing transactions does not imply support for every script-writing workflow; unsupported operations must return the pinned error rather than fabricate execution units or successful construction.
- The project targets Linux x86_64 as the primary platform.
- pallas 0.32 is the pinned Cardano primitives library.
- SMASH integration (`/smash/health`) requires a configured SMASH server URL; the endpoint proxies health checks to it.

### Era and paused-tip restoration acceptance

- A restored wallet at a node-confirmed unchanged tip must become ready even
  when producers are paused. Readiness must not mutate the persisted tip,
  pending signed bytes, transaction history or user metadata.
- Before Conway, transaction-writing endpoints must reject unsupported writing
  with the pinned403 node_not_yet_in_recent_era envelope. Restoration and reads
  remain available. After the actual fork, the same wallet process can create,
  sign and submit a payment and observe stake registration/refund certificates.
