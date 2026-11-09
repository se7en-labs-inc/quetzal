# Feature Specification: Quetzal — Cardano Wallet Backend in Rust

**Feature Branch**: `001-wallet-rust-backend`
**Created**: 2026-08-22
**Status**: Draft
**Project**: Quetzal (se7en-labs-inc/quetzal)

## Overview

### Continuation — 2026-11-06

The project is renamed Quetzal. Continue the existing wallet implementation and
preserve in-progress changes. The acceptance contract is the reviewed executable
suite in `../../cardano-wallet-conformance/master`: 643 active obligations across
37 HTTP, internal-vector and owned real-node profiles. Supply a thin adapter
calling actual wallet internals; unsupported operations fail explicitly. Passing
unit tests or a subset of profiles does not establish full conformance. Retain
fresh reports and report missing coverage separately from failures.

Quetzal is a full reimplementation of the Cardano wallet backend in Rust, designed as a drop-in replacement for the Haskell `cardano-wallet` service. The primary consumer is Daedalus desktop wallet. Quetzal must expose **all 78 paths** in `swagger.yaml` so that Daedalus requires zero protocol-level changes to switch backends. This includes Shelley single-sig wallets, Byron wallets, shared multi-sig wallets, stake pool operations, DRep governance, chain data, SMASH integration, and network endpoints.

---

## User Scenarios & Testing

### User Story 1 — Wallet Lifecycle Management (Priority: P1)

A Daedalus user can create a new Shelley wallet from a freshly generated mnemonic, restore an existing wallet from a known mnemonic and passphrase, view a list of all their wallets with current balance and sync status, and permanently delete a wallet they no longer need.

**Why this priority**: Without wallets there is no product. Every other story depends on a wallet existing and being accessible via its ID.

**Independent Test**: Create a Shelley wallet via `POST /wallets`, verify the response contains a wallet ID and "syncing" status, list it via `GET /wallets`, then delete it via `DELETE /wallets/{id}` and confirm it no longer appears.

**Acceptance Scenarios**:

1. **Given** no wallets exist, **When** a user POSTs a valid 24-word mnemonic and passphrase to `/wallets`, **Then** a wallet is created, a unique wallet ID is returned, and the wallet appears in `GET /wallets` with a balance of 0 and sync status "syncing".
2. **Given** an existing wallet mnemonic, **When** the user POSTs that mnemonic to `/wallets`, **Then** the wallet is restored with the same wallet ID as the original and begins chain synchronisation.
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

**Independent Test**: Use `POST /wallets/{walletId}/transactions` with a target address and amount; verify the returned transaction contains a valid CBOR body, correct fee, and can be submitted via `POST /wallets/{walletId}/transactions-submit`.

**Acceptance Scenarios**:

1. **Given** a funded wallet, **When** the user POSTs a payment to `/wallets/{walletId}/transactions` with a valid recipient address and amount, **Then** a signed transaction is returned with a fee and the transaction ID.
2. **Given** a valid unsigned transaction, **When** the user POSTs to `/wallets/{walletId}/transactions-sign` with their passphrase, **Then** the transaction is returned with valid witness signatures.
3. **Given** a signed transaction, **When** the user POSTs to `/wallets/{walletId}/transactions-submit`, **Then** the transaction is forwarded to the Cardano node and a transaction ID is returned.
4. **Given** insufficient funds, **When** the user attempts a payment exceeding their balance, **Then** the request is rejected with an error indicating the shortfall amount.
5. **Given** a payment request, **When** the user calls `POST /wallets/{walletId}/payment-fees`, **Then** a fee estimate with minimum and maximum is returned without constructing the transaction.
6. **Given** a multi-output transaction, **When** the user specifies multiple recipients, **Then** coin selection distributes inputs to cover all outputs and the change output.
7. **Given** a submitted payment temporarily rejected because a competing transaction occupies the node mempool, **When** the competing transaction expires, **Then** the wallet retries the unchanged signed transaction on the reference schedule and observes its confirmation. Pending state and the latest submission slot survive a process restart. Forgetting or expiring a transaction stops retries.

---

### User Story 5 — Transaction History (Priority: P5)

A Daedalus user can view a paginated, chronologically-ordered list of all transactions affecting their wallet, filtered by time range or status.

**Why this priority**: Transaction history is essential for financial record-keeping.

**Independent Test**: After simulating inbound and outbound transactions in integration tests, `GET /wallets/{walletId}/transactions` returns a list ordered by most-recent-first, with correct amounts, fees, and statuses.

**Acceptance Scenarios**:

1. **Given** a wallet with transaction history, **When** the user calls `GET /wallets/{walletId}/transactions`, **Then** transactions are returned newest-first with amount, fee, inputs, outputs, and status fields.
2. **Given** a large transaction history, **When** the user supplies `start`, `end`, or pagination parameters, **Then** only transactions within the specified range are returned.
3. **Given** an unconfirmed transaction, **When** the user queries history, **Then** the transaction appears with status "pending" until it is confirmed on-chain.
4. **Given** a confirmed transaction, **When** the user queries `GET /wallets/{walletId}/transactions/{transactionId}`, **Then** full transaction detail including block height and slot is returned.

---

### User Story 6 — Chain Synchronisation (Priority: P6)

The Quetzal service automatically synchronises with the Cardano node via the node-to-client miniprotocols, tracking the current chain tip and updating wallet state as new blocks arrive.

**Why this priority**: All wallet data depends on chain sync; without it balances and history are stale.

**Independent Test**: Start Quetzal pointing at a running cardano-node socket; after a configurable timeout the service reports sync progress > 0% via `GET /network/information`.

**Acceptance Scenarios**:

1. **Given** a running Cardano node and a configured socket path, **When** Quetzal starts, **Then** it connects via the chain-sync miniprotocol and begins requesting blocks from the node's tip.
2. **Given** an active chain-sync session, **When** a new block is received, **Then** wallet UTxO sets and transaction histories are updated atomically before the next block is processed.
3. **Given** a node restart or connection drop, **When** the connection is lost, **Then** Quetzal reconnects automatically and resumes from its last known checkpoint without data loss.
4. **Given** a fully synced wallet, **When** the user queries `GET /network/information`, **Then** the response shows the current tip slot, sync progress as a percentage, and network parameters.
5. **Given** a wallet checkpoint within the reference default five-minute wall-clock sync tolerance, **When** its public state is rendered, **Then** it reports `ready` without a progress field. Older checkpoints report their elapsed-time percentage. This projection does not advance or alter the checkpoint or balances.

---

### User Story 7a — Byron Wallet Support (Priority: P7a)

A Daedalus user with existing Byron-era ADA can restore a Byron wallet (random or Icarus), view addresses and balance, send transactions, and migrate funds to a Shelley wallet.

**Why this priority**: Byron wallet holders need a migration path; required for full API parity.

**Independent Test**: `POST /byron-wallets` with an Icarus mnemonic creates a Byron wallet; `GET /byron-wallets/{id}/addresses` returns base58 addresses; `GET /byron-wallets/{id}/migrations/plan` returns a migration plan.

**Acceptance Scenarios**:

1. **Given** a Byron Icarus mnemonic, **When** the user POSTs to `/byron-wallets`, **Then** a Byron wallet is created with correct base58 addresses.
2. **Given** a Byron wallet with funds, **When** the user requests `GET /byron-wallets/{id}/migrations/plan`, **Then** a plan is returned showing inputs, outputs, and fees for migrating to a Shelley address.
3. **Given** a migration plan, **When** the user POSTs to `/byron-wallets/{id}/migrations`, **Then** signed migration transactions are returned, ready to submit.

---

### User Story 7b — Shared (Multi-Sig) Wallets (Priority: P7b)

A Daedalus user participating in a multi-signature wallet can create or restore a shared wallet using a script template, view the shared wallet's addresses and UTxO, construct and sign transactions that require multiple signatories, and track which cosigners have signed.

**Why this priority**: Required for full Daedalus API parity; multi-sig is an increasingly common use case.

**Independent Test**: `POST /shared-wallets` with a payment script template creates a shared wallet; `GET /shared-wallets/{id}/addresses` returns script addresses; `POST /shared-wallets/{id}/transactions-construct` returns an unsigned tx awaiting cosigners.

**Acceptance Scenarios**:

1. **Given** a payment script template with cosigner keys, **When** the user POSTs to `/shared-wallets`, **Then** a shared wallet is created with a unique wallet ID and script-derived addresses.
2. **Given** a funded shared wallet, **When** the user constructs a transaction via `/shared-wallets/{id}/transactions-construct`, **Then** an unsigned transaction is returned that requires the script-specified signatures.
3. **Given** an unsigned shared transaction, **When** each cosigner POSTs to `/shared-wallets/{id}/transactions-sign`, **Then** witness signatures accumulate until the script threshold is met.
4. **Given** a fully-witnessed shared transaction, **When** submitted via `/shared-wallets/{id}/transactions-submit`, **Then** it is accepted by the node.

---

### User Story 7c — DRep Governance and Voting Delegation (Priority: P7c)

A Daedalus user can browse available DReps, delegate their vote to a DRep, view suggested DReps, and query delegation status — all in the Conway governance era.

**Why this priority**: Conway era governance is live; Daedalus must surface DRep delegation.

**Independent Test**: `GET /dreps` returns a list of registered DReps; `PUT /dreps/{drepId}/wallets/{walletId}` returns a vote delegation transaction.

**Acceptance Scenarios**:

1. **Given** a synced Conway-era service, **When** the user calls `GET /dreps`, **Then** registered DReps are returned with their metadata and stake.
2. **Given** a wallet, **When** the user calls `GET /dreps/suggested`, **Then** suggested DReps are returned based on stake distribution.
3. **Given** a wallet with ADA, **When** the user delegates votes via `PUT /dreps/{drepId}/wallets/{walletId}`, **Then** a vote delegation certificate transaction is constructed and returned.

---

### User Story 7 — Stake Pool Operations and Delegation (Priority: P7)

A Daedalus user can list available stake pools, delegate their wallet's stake to a pool, and withdraw staking rewards.

**Why this priority**: Staking is a key value-add over a basic payment wallet; required for full Daedalus feature parity.

**Independent Test**: `GET /stake-pools` returns a list of pools with metadata; `PUT /stake-pools/{poolId}/wallets/{walletId}` returns a delegation transaction.

**Acceptance Scenarios**:

1. **Given** a synced service, **When** the user calls `GET /stake-pools`, **Then** known stake pools are returned with their ticker, name, saturation, and estimated ROA.
2. **Given** a wallet with ADA, **When** the user delegates to a pool via `PUT /stake-pools/{poolId}/wallets/{walletId}`, **Then** a delegation certificate transaction is constructed and returned for signing.
3. **Given** a wallet with accumulated rewards, **When** the user queries `GET /wallets/{walletId}`, **Then** `balance.reward` reflects the current reward balance in lovelace.
4. **Given** accrued rewards, **When** the user submits a withdrawal transaction, **Then** rewards are swept into the wallet's available balance.

---

### Edge Cases

- What happens when a node socket is unavailable at startup? Service starts, reports degraded status via `/network/information`, and retries connection automatically.
- How does the system handle restoring the same mnemonic twice? Idempotent — returns the existing wallet without duplicating it.
- What happens during a Cardano hard fork / era transition? Chain-sync resumes correctly; era-specific block decoders switch automatically.
- How are transactions with reference scripts shown in history? Decoded and displayed; script execution is not re-run by the wallet.
- What if the sled database is corrupted on startup? Service fails with a clear error and refuses to start rather than operating on bad state.
- How are large wallets (>100k addresses) handled? Address discovery gap limit is configurable; default is 20 sequential unused addresses.
- What happens if a submitted transaction is rolled back? Transaction is marked "expired" in history when the rollback is detected during chain sync.

---

## Requirements

### Functional Requirements

- **FR-001**: System MUST expose a REST API compatible with the cardano-wallet OpenAPI specification for all Shelley wallet endpoints used by Daedalus.
- **FR-002**: System MUST support wallet creation and restoration from 15-word and 24-word BIP-39 mnemonics.
- **FR-003**: System MUST derive Shelley (CIP-1852), Icarus, and Byron address types from the appropriate key hierarchies.
- **FR-004**: System MUST discover used addresses during chain synchronisation using the sequential address discovery algorithm (configurable gap limit, default 20).
- **FR-005**: System MUST maintain a per-wallet UTxO set updated atomically as blocks are processed.
- **FR-006**: System MUST implement coin selection (largest-first and random-improve strategies) for transaction construction.
- **FR-007**: System MUST construct, balance, and sign transactions compatible with the current Cardano era (Conway and later).
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

- **SC-001**: All cardano-wallet REST API endpoints used by Daedalus return responses structurally identical to the Haskell reference implementation.
- **SC-002**: A Shelley wallet can be created, funded (via test faucet), and a payment sent end-to-end in under 10 seconds on preprod network (excluding chain sync time).
- **SC-003**: Chain synchronisation processes at least 500 blocks per second on commodity hardware (i7/Ryzen 5 class, 16 GB RAM).
- **SC-004**: Unit and property test coverage covers all domain functions in the primitives, crypto, and address crates.
- **SC-005**: Integration tests against a local cluster pass for all P1–P5 user story acceptance scenarios.
- **SC-006**: The service handles 100 concurrent Daedalus API requests without error or significant response time degradation.
- **SC-007**: Wallet state is recoverable after a crash at any point during block processing — no partial-block corruption.
- **SC-008**: Memory usage for a fully synced wallet with 10,000 addresses and 50,000 transactions stays below 512 MB.

---

## Assumptions

- Daedalus will connect to Quetzal over localhost HTTP; TLS is a follow-on.
- All 78 paths in `swagger.yaml` are in scope, including Shelley wallets, Byron wallets, shared/multi-sig wallets, stake pools, DRep governance, and chain data endpoints.
- The cardano-node socket path is provided via configuration; Quetzal does not launch the node.
- Plutus script *execution* (phase-2 validation) is out of scope; Quetzal constructs and submits transactions containing Plutus scripts but does not evaluate them locally.
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
