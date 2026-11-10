# Tasks: Quetzal Wallet Backend

**Branch**: `001-wallet-rust-backend`
**Input**: spec.md, plan.md, research.md, data-model.md

## Transport and specification correction — 2026-11-09

This section supersedes historical continuation notes below. V58 qualified the
old 643/37 contract only. The corrected contract requires 646/38.

- [x] C009 Reconcile API side effects, duplicate conflict, identity, mnemonic derivation, historical restoration, rollback and acceptance boundaries; replace the stale checklist.
- [x] C010 Add fail-closed complete TLS configuration, loopback default and explicit safe IP binding.
- [x] C011 Serve the unchanged router with mandatory client-authenticated TLS, bounded handshakes/connections and no plaintext API on the secure listener.
- [x] C012 Verify real TLS acceptance/rejection and startup failures; run workspace tests, formatting, strict Clippy and release build through Nix. Preserve the initial test-client alert race under evidence/20261109-mtls; with user approval, use direct Rustls rejection reads while retaining strict alert assertions, repeated 30 times with authenticated brackets. All 85 executed workspace tests pass; the existing standalone live-node test remains ignored in this command and real-node qualification is performed by C013.
- [x] C013 Run the corrected external contract with outputs outside its read-only checkout. Fresh complete=true: 646/646 obligations, 3,808 checks, zero failures, 38 reports, unchanged identities, all attempts passed and owned cleanup verified. Receipts: evidence/20261109-mtls. The Nix package also builds and passes its tests; the conformance candidate is the separately measured local release binary.
- [ ] C014 Qualify actual pinned Daedalus launch-and-use and a defined performance workload; do not infer these from conformance counts.

## Continuation — 2026-11-06

Resumed after reboot on 2026-11-07. Latest checkpoint is
`evidence/20261109-retry/CONTINUATION.md`. The V46 complete HTTP/internal replay
plus expanded funded Shelley checks covers567/643 obligations (508/537 HTTP,
1241/1241 internal, 709/709 funded checks pass). This is not the full live gate.
V44 corrects the observed-registration pool catalogue rule; network13/14.
Focused actual delegation/rewards/refund204/204, DRep6/6 and pool lifecycle86/86
pass. V49 real Icarus/random Byron/Shelley payment+migration158/158 and checkpoint
29/29 pass. V52 full internal and real retry controls are underway. Keep all
canonical V52 reports on target19097/v2/ and environmentquetzal-v52; gate forbids
mixing targets/environments. The conformance suite remains unchanged.
User priority remains Shelley before Byron; the pinned legacy dependency is
already approved. Preserve all existing dirty changes.

API signing now merges verification-key witnesses idempotently. The v15 history
fix accepts `max_count=0` and implements individual-withdrawal filtering;
its focused funded replay passes 148/148 checks. Workspace tests, formatting and
release build pass. The user approved continuation with the Clippy refactor;
strict Clippy now passes without warning suppressions. Policy keys persist and
policy IDs use resolved native scripts; all five extended policy HTTP cases pass.
Worker cancellation and handshake cleanup tests pass; five real create/delete
cycles do not increase candidate socket descriptors. Partial balancing, live
delegation-fee prototypes, exact relative validity clock rounding and structured
node validity errors pass funded verification. The user approved the large-input
framing/body-preservation proposal and requested autonomous continuation through
all obligations. The encoding correction is implemented; fresh verification is
underway in `evidence/20261109-input-encoding/`.
C005–C008 remain open; all 643 obligations are required.

The user confirmed continued implementation until all 643 active obligations are
covered. Keep C005–C008 active until a fresh complete conformance gate passes;
intermediate audits are progress checkpoints, not completion.

- [x] C001 Repair renamed repository worktree pointers and verify host paths.
- [x] C002 Establish the Nix workspace test baseline; preserve existing changes.
- [x] C003 Rename workspace crates, binary, package metadata and docs to Quetzal.
- [x] C004 Implement isolated configurable startup for conformance service launch.
- [ ] C005 Implement the internal conformance adapter using actual wallet functions.
- [ ] C006 Complete missing HTTP, ledger and state-transition behavior required by the supplied contract.
- [ ] C007 Pass formatting, clippy, workspace tests and the release build through Nix.
- [ ] C008 Run supplied conformance profiles and retain fresh coverage/failure reports.

Current continuation results are retained in `evidence/20261107-shelley-v21`;
earlier quality, cancellation and balancing attempts are preserved separately.
Earlier reports remain preserved. The adapter and wallet behavior
remain incomplete, and the complete live
catalogue has not passed. The required Haskell key-oracle executable digest
also remains unreproduced; preserve the harness provenance checks.

---

## Phase 1: Setup

**Purpose**: Workspace dependency graph and project scaffolding

- [ ] T001 Update root Cargo.toml [workspace.dependencies] with pallas-addresses, ed25519_bip32, argon2, zeroize, proptest, tower, tower-http, ciborium, bytes in Cargo.toml
- [ ] T002 [P] Add pallas-addresses and ed25519_bip32 to crates/quetzal-address/Cargo.toml
- [ ] T003 [P] Add ed25519_bip32, argon2, zeroize, ciborium to crates/quetzal-crypto/Cargo.toml
- [ ] T004 [P] Add ciborium, zeroize to crates/quetzal-primitives/Cargo.toml
- [ ] T005 [P] Add tower, tower-http to crates/quetzal-api/Cargo.toml
- [ ] T006 [P] Add proptest as dev-dependency to quetzal-primitives, quetzal-crypto, quetzal-address, quetzal-wallet crates
- [ ] T007 Verify `nix develop --command cargo check` passes with all new dependencies

---

## Phase 2: Foundational (quetzal-primitives)

**Purpose**: Core domain types that all higher crates depend on. BLOCKS all user stories.

- [ ] T008 Implement Coin newtype (u64 lovelace, #[must_use] checked arithmetic, Display) in crates/quetzal-primitives/src/coin.rs
- [x] T009 [P] Implement reference WalletId ([u8; 20] Blake2b-160 extended-public-key digest) and WalletStatus in crates/quetzal-primitives/src/wallet.rs
- [ ] T010 [P] Implement TxHash newtype ([u8; 32]), TxStatus enum, TxDirection enum in crates/quetzal-primitives/src/tx.rs
- [ ] T011 [P] Implement UtxoId (TxHash + output_index u32), Value struct (Coin + BTreeMap assets) in crates/quetzal-primitives/src/utxo.rs
- [ ] T012 [P] Implement Address newtype (String, Bech32/base58), AddressState enum, DerivationPath, AddressRole in crates/quetzal-primitives/src/address.rs
- [ ] T013 [P] Implement Checkpoint struct (slot: u64, block_hash: [u8; 32]) in crates/quetzal-primitives/src/checkpoint.rs
- [ ] T014 [P] Implement PoolId newtype ([u8; 28]), PoolMeta struct in crates/quetzal-primitives/src/pool.rs
- [ ] T015 Implement sled binary key encoding helpers: slot_key(u64)->IVec, utxo_key(TxHash,u32)->IVec, tx_history_key(u64,TxHash)->IVec in crates/quetzal-primitives/src/codec.rs
- [ ] T016 Implement CBOR serialization (ciborium) for WalletMeta, UtxoEntry, TxRecord, AddressState, PoolMeta in crates/quetzal-primitives/src/lib.rs
- [ ] T017 [P] Implement JSON serde for all primitives matching cardano-wallet OpenAPI field names in crates/quetzal-primitives/src/lib.rs
- [ ] T018 Write proptest roundtrip tests for CBOR and JSON serialization in crates/quetzal-primitives/tests/roundtrip.rs

**Checkpoint**: `cargo test -p quetzal-primitives` passes

---

## Phase 3: User Story 6 — Chain Synchronisation (Priority ordered by dependency)

**Goal**: Connect to cardano-node and sync blocks before wallet operations can work

**Independent Test**: `CARDANO_NODE_SOCKET_PATH=/tmp/node.socket cargo test -p quetzal-network` connects and receives at least one block

### quetzal-crypto (prerequisite for address derivation)

- [ ] T019 Implement Bip32PrivateKey and Bip32PublicKey wrappers around ed25519_bip32::{XPrv, XPub} with Zeroize in crates/quetzal-crypto/src/key.rs
- [ ] T020 [P] Implement derive_shelley_root(mnemonic_bytes) -> Bip32PrivateKey using Icarus master key derivation (PBKDF2-SHA512) in crates/quetzal-crypto/src/derivation.rs
- [ ] T021 [P] Implement derive_child(parent, index, hardened) and CIP-1852 path helpers (account, role, address indexes) in crates/quetzal-crypto/src/derivation.rs
- [x] T022 [P] Implement authenticated current/legacy root envelopes and storage KDFs; retain reference mnemonic derivation independently.
- [ ] T023 Implement Ed25519Signer trait impl for Bip32PrivateKey (pallas_txbuilder compatible) in crates/quetzal-crypto/src/signing.rs
- [ ] T024 [P] Write CIP-1852 known-answer tests in crates/quetzal-crypto/tests/derivation.rs
- [ ] T025 [P] Write passphrase encrypt/decrypt roundtrip tests in crates/quetzal-crypto/tests/passphrase.rs

### quetzal-network

- [ ] T026 Implement NodeClient::connect(socket_path) using pallas_network Bearer::connect_unix + N2C handshake in crates/quetzal-network/src/client.rs
- [ ] T027 Implement ChainSyncDriver: wrap pallas chainsync::Client, expose async channel for (RollForward(MultiEraBlock) | RollBackward(Checkpoint)) in crates/quetzal-network/src/chainsync.rs
- [ ] T028 [P] Implement TxSubmitter::submit(signed_cbor_bytes) -> TxHash using LocalTxSubmission miniprotocol in crates/quetzal-network/src/txsubmit.rs
- [ ] T029 [P] Implement StateQuerier::protocol_params() and StateQuerier::stake_pools() using LocalStateQuery in crates/quetzal-network/src/statequery.rs
- [ ] T030 Write mock node socket test (in-process Unix socket pair simulating FindIntersect/RollForward) in crates/quetzal-network/tests/mock_node.rs

**Checkpoint**: `cargo test -p quetzal-crypto -p quetzal-network` passes

---

## Phase 4: User Story 1 — Wallet Lifecycle Management (Priority: P1) 🎯 MVP

**Goal**: Create, restore, list, and delete Shelley wallets via REST API

**Independent Test**: `curl -X POST localhost:8090/wallets` creates a wallet, `GET /wallets` lists it, `DELETE /wallets/{id}` removes it

### quetzal-address (prerequisite for wallet creation)

- [ ] T031 Implement ShelleyAddress::enterprise(payment_vk, network) and ::base(payment_vk, stake_vk, network) using pallas_addresses in crates/quetzal-address/src/shelley.rs
- [ ] T032 [P] Implement IcarusAddress::from_key(pub_key, network) (Byron Icarus base58) in crates/quetzal-address/src/icarus.rs
- [ ] T033 [P] Implement ByronAddress::random(root_key, path, network_magic) in crates/quetzal-address/src/byron.rs
- [ ] T034 Implement AddressDiscovery: derive 20 addresses ahead of last used, is_mine(addr)->Option<DerivationPath>, extend pool on gap in crates/quetzal-address/src/discovery.rs
- [ ] T035 [P] Write known-answer tests from CIP-1852/CIP-0011 test vectors in crates/quetzal-address/tests/known_vectors.rs
- [ ] T036 [P] Write gap discovery property tests in crates/quetzal-address/tests/discovery.rs

### quetzal-wallet DB layer

- [ ] T037 Implement WalletDb::open(path) opening sled, creating all named trees (wallets, utxos:*, txs:*, addrs:*, checkpoints:*, pending:*, pools, idx:addr_wallet) in crates/quetzal-wallet/src/db/mod.rs
- [ ] T038 [P] Implement wallet CRUD on sled (insert, get, list, delete) in crates/quetzal-wallet/src/db/wallets.rs
- [ ] T039 [P] Implement checkpoint read/write helpers in crates/quetzal-wallet/src/db/checkpoints.rs

### WalletService — wallet lifecycle

- [ ] T040 Implement WalletService::new(db, node_client) in crates/quetzal-wallet/src/wallet.rs
- [ ] T041 Implement WalletService::create_wallet(name, mnemonic_words, passphrase) -> Result<WalletId>: validate mnemonic, derive root key, encrypt with passphrase, store WalletMeta in crates/quetzal-wallet/src/wallet.rs
- [x] T042 Implement restoration with deterministic reference identity; same-family duplicate creation returns 409 wallet_already_exists without mutation.
- [ ] T043 [P] Implement WalletService::delete_wallet(id): remove all trees for wallet in crates/quetzal-wallet/src/wallet.rs
- [ ] T044 [P] Implement WalletService::list_wallets() and get_wallet(id) in crates/quetzal-wallet/src/wallet.rs
- [ ] T045 Write wallet lifecycle integration tests using sled in-memory (sled::Config::new().temporary(true)) in crates/quetzal-wallet/tests/wallet.rs

### quetzal-api — wallet endpoints

- [ ] T046 Implement AppState struct and Config (socket_path, db_path, port, network_magic) in crates/quetzal-api/src/state.rs and config.rs
- [ ] T047 Implement ApiError enum and IntoResponse for cardano-wallet error JSON format in crates/quetzal-api/src/error.rs
- [ ] T048 Implement PostWalletRequest and WalletResponse JSON types matching OpenAPI spec in crates/quetzal-api/src/json/wallet.rs
- [ ] T049 Implement POST /wallets handler (create/restore) in crates/quetzal-api/src/handlers/wallets.rs
- [ ] T050 [P] Implement GET /wallets handler (list all) in crates/quetzal-api/src/handlers/wallets.rs
- [ ] T051 [P] Implement GET /wallets/:id handler in crates/quetzal-api/src/handlers/wallets.rs
- [ ] T052 [P] Implement DELETE /wallets/:id handler in crates/quetzal-api/src/handlers/wallets.rs
- [ ] T053 Implement axum router with /wallets routes and tower-http tracing middleware in crates/quetzal-api/src/main.rs
- [ ] T054 Write axum integration tests for wallet CRUD endpoints in crates/quetzal-api/tests/api_wallets.rs

**Checkpoint**: `cargo test -p quetzal-wallet -p quetzal-api` passes; wallet CRUD endpoints work

---

## Phase 5: User Story 2 — Address Derivation and Discovery (Priority: P2)

**Goal**: GET /wallets/:id/addresses returns correct Shelley/Icarus addresses

**Independent Test**: `GET /wallets/{id}/addresses` returns ≥20 Bech32 addresses each with state field

### WalletService — address management

- [ ] T055 Implement address DB layer: insert/list/mark-used operations on sled addrs tree in crates/quetzal-wallet/src/db/addresses.rs
- [ ] T056 Implement WalletService::list_addresses(id, filter) -> Vec<(Address, AddressState)>: derive next 20, mark used ones in crates/quetzal-wallet/src/wallet.rs
- [ ] T057 Implement WalletService::ensure_address_pool(id): pre-derive gap addresses after each block scan in crates/quetzal-wallet/src/wallet.rs

### quetzal-api — address endpoint

- [ ] T058 Implement AddressResponse JSON type in crates/quetzal-api/src/json/address.rs
- [ ] T059 Implement GET /wallets/:id/addresses handler with ?state= filter in crates/quetzal-api/src/handlers/addresses.rs
- [ ] T060 Add /wallets/:id/addresses route to router in crates/quetzal-api/src/main.rs
- [ ] T061 Write axum tests for address endpoint in crates/quetzal-api/tests/api_wallets.rs

**Checkpoint**: Address endpoint returns correct Bech32 addresses

---

## Phase 6: User Story 3 — Balance and UTxO Queries (Priority: P3)

**Goal**: GET /wallets/:id returns balance; GET /wallets/:id/utxo and statistics work

**Independent Test**: After inserting test UTxOs into sled, balance and UTxO endpoints return correct values

### quetzal-wallet — UTxO layer

- [ ] T062 Implement UTxO DB layer: insert, delete, list-by-wallet, sum operations in crates/quetzal-wallet/src/db/utxos.rs
- [ ] T063 Implement WalletService::get_balance(id) -> (available: Coin, total: Coin, reward: Coin) in crates/quetzal-wallet/src/wallet.rs
- [ ] T064 [P] Implement WalletService::list_utxos(id) -> Vec<UtxoEntry> in crates/quetzal-wallet/src/wallet.rs
- [ ] T065 [P] Implement WalletService::utxo_statistics(id) -> UtxoStatistics (histogram buckets matching cardano-wallet format) in crates/quetzal-wallet/src/wallet.rs

### quetzal-api — UTxO endpoints

- [ ] T066 Implement UTxO response JSON types in crates/quetzal-api/src/json/wallet.rs
- [ ] T067 Implement GET /wallets/:id/utxo handler in crates/quetzal-api/src/handlers/utxo.rs
- [ ] T068 [P] Implement GET /wallets/:id/statistics/utxos handler in crates/quetzal-api/src/handlers/utxo.rs
- [ ] T069 Add UTxO routes to router in crates/quetzal-api/src/main.rs
- [ ] T070 Update GET /wallets/:id to include balance fields (requires UTxO layer) in crates/quetzal-api/src/handlers/wallets.rs

**Checkpoint**: Balance and UTxO endpoints return correct values

---

## Phase 7: User Story 4 — Transaction Construction, Signing, Submission (Priority: P4)

**Goal**: Full payment flow: fee estimate → construct → sign → submit

**Independent Test**: POST /wallets/:id/transactions with funded wallet returns signed tx; submit succeeds

### quetzal-wallet — tx building

- [ ] T071 Implement linear fee calculation: fee(tx_bytes) = a + b * size using protocol params in crates/quetzal-wallet/src/fees.rs
- [ ] T072 Implement largest_first coin selection in crates/quetzal-wallet/src/coinselect.rs
- [ ] T073 [P] Implement random_improve coin selection in crates/quetzal-wallet/src/coinselect.rs
- [ ] T074 Write coin selection property tests: total_in = total_out + fee invariant in crates/quetzal-wallet/tests/coinselect.rs
- [ ] T075 Implement TxBuilder::build_payment(inputs, outputs, change_addr, fee) -> StagingTransaction using pallas_txbuilder in crates/quetzal-wallet/src/txbuild.rs
- [ ] T076 Implement TxBuilder::sign(staging_tx, signing_key) -> BuiltTransaction in crates/quetzal-wallet/src/txbuild.rs
- [ ] T077 [P] Implement TxBuilder::decode(cbor_bytes) -> DecodedTx in crates/quetzal-wallet/src/txbuild.rs
- [ ] T078 [P] Implement TxBuilder::balance(partial_tx, utxos, params) -> BalancedTx in crates/quetzal-wallet/src/txbuild.rs
- [ ] T079 Implement WalletService::estimate_fee(id, outputs) -> FeeEstimate in crates/quetzal-wallet/src/wallet.rs
- [ ] T080 Implement WalletService::create_payment(id, outputs, passphrase) -> SignedTxBytes in crates/quetzal-wallet/src/wallet.rs
- [ ] T081 [P] Implement WalletService::construct_transaction(id, body) -> UnsignedTxBytes in crates/quetzal-wallet/src/wallet.rs
- [ ] T082 [P] Implement WalletService::sign_transaction(id, unsigned_cbor, passphrase) -> SignedTxBytes in crates/quetzal-wallet/src/wallet.rs
- [ ] T083 [P] Implement WalletService::submit_transaction(signed_cbor) -> TxHash (delegates to TxSubmitter) in crates/quetzal-wallet/src/wallet.rs
- [ ] T084 Write tx construction tests in crates/quetzal-wallet/tests/txbuild.rs

### quetzal-wallet — pending tx tracking

- [x] T085 Implement pending tx DB layer: insert-pending, get-pending, list-pending, mark-confirmed/expired in crates/quetzal-wallet/src/db/mod.rs
- [x] T085a Persist latest local submission slots atomically without resurrecting forgotten or confirmed pending transactions.
- [x] T085b Implement cancellation-safe per-wallet resubmission with the pinned one-second/ten-expected-block schedule and exact transient mempool rejection classification.
- [x] T085c Pass unchanged owned checkpoint, pending deletion, expiry and real conflicting-mempool retry controls for Shelley, Icarus and random Byron.
- [ ] T086 Implement WalletService::forget_transaction(id, tx_hash) (remove from pending) in crates/quetzal-wallet/src/wallet.rs

### quetzal-api — transaction endpoints

- [ ] T087 Implement PostTransactionRequest and TransactionResponse JSON types (cardano-wallet compatible) in crates/quetzal-api/src/json/transaction.rs
- [ ] T088 Implement POST /wallets/:id/transactions handler (simple payment) in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T089 [P] Implement POST /wallets/:id/payment-fees handler in crates/quetzal-api/src/handlers/fees.rs
- [ ] T090 [P] Implement POST /wallets/:id/transactions-construct handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T091 [P] Implement POST /wallets/:id/transactions-sign handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T092 [P] Implement POST /wallets/:id/transactions-decode handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T093 [P] Implement POST /wallets/:id/transactions-submit handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T094 [P] Implement POST /wallets/:id/transactions-balance handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T095 [P] Implement DELETE /wallets/:id/transactions/:txid handler (forget pending) in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T096 Add all transaction routes to router in crates/quetzal-api/src/main.rs
- [ ] T097 Write axum tests for transaction endpoints in crates/quetzal-api/tests/api_transactions.rs

**Checkpoint**: Fee estimate, construct, sign, and submit endpoints work end-to-end

---

## Phase 8: User Story 5 — Transaction History (Priority: P5)

**Goal**: GET /wallets/:id/transactions returns paginated, slot-ordered history

**Independent Test**: After inserting test TxRecords into sled, transaction list endpoint returns correct ordered results

### quetzal-wallet — tx history

- [ ] T098 Implement tx DB layer: insert, get-by-hash, list-by-slot-range with pagination in crates/quetzal-wallet/src/db/txs.rs
- [ ] T099 Implement WalletService::list_transactions(id, start, end, order, limit) using sled range scan on slot key in crates/quetzal-wallet/src/wallet.rs
- [ ] T100 [P] Implement WalletService::get_transaction(id, tx_hash) in crates/quetzal-wallet/src/wallet.rs

### quetzal-api — transaction history endpoints

- [ ] T101 Implement GET /wallets/:id/transactions handler with ?start, ?end, ?order, ?limit query params in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T102 [P] Implement GET /wallets/:id/transactions/:txid handler in crates/quetzal-api/src/handlers/transactions.rs
- [ ] T103 Write axum tests for transaction history pagination in crates/quetzal-api/tests/api_transactions.rs

**Checkpoint**: Transaction history returns correct paginated results

---

## Phase 9: User Story 6 — Chain Sync Integration (Priority: P6)

**Goal**: Quetzal syncs with cardano-node, updating wallet state as blocks arrive

**Independent Test**: Service starts against cardano-node socket, GET /network/information shows progress > 0

### quetzal-wallet — sync worker

- [ ] T104 Implement ChainSyncWorker::new(chainsync_driver, wallet_db, address_discovery) in crates/quetzal-wallet/src/sync.rs
- [ ] T105 Implement block processing in sync.rs: for each MultiEraBlock → scan outputs for wallet addresses → update UTxO set → record TxRecords → advance checkpoint (all in one sled batch)
- [x] T106 Implement atomic rollback via undo data, restoring older spent inputs, reversing received outputs/history, preserving valid local pending provenance and removing discovered-only effects; verify exact native reapplication without duplicate credit.
- [x] T106a Query the latest native header from one acquired node snapshot across reconnects and validate with unchanged native rollback/reapplication assertions (V57: 224/224).
- [ ] T107 Implement pending tx confirmation: on block scan, check pending txs; mark confirmed or expired in crates/quetzal-wallet/src/sync.rs
- [ ] T108 Implement startup checkpoint restore: load last checkpoint from sled, send as FindIntersect points to chainsync driver in crates/quetzal-wallet/src/sync.rs
- [ ] T109 Spawn ChainSyncWorker as tokio::spawn task in crates/quetzal-api/src/main.rs

### quetzal-api — network endpoints

- [ ] T110 Implement NetworkInfoResponse JSON type in crates/quetzal-api/src/json/network.rs
- [ ] T111 Implement GET /network/information handler (sync progress, tip slot, network magic) in crates/quetzal-api/src/handlers/network.rs
- [ ] T112 [P] Implement GET /network/parameters handler (protocol params from StateQuerier) in crates/quetzal-api/src/handlers/network.rs
- [ ] T113 [P] Implement GET /network/clock handler in crates/quetzal-api/src/handlers/network.rs
- [ ] T114 Add network routes to router in crates/quetzal-api/src/main.rs
- [ ] T115 Write axum tests for network endpoints in crates/quetzal-api/tests/api_network.rs

**Checkpoint**: Service syncs blocks and network/information reports progress

---

## Phase 10: User Story 7 — Stake Pools and Delegation (Priority: P7)

**Goal**: List stake pools, delegate, withdraw rewards

**Independent Test**: GET /stake-pools returns pool list; PUT /stake-pools/:id/wallets/:id returns delegation tx

### quetzal-wallet — delegation

- [ ] T116 Implement pool metadata DB layer (insert, list, get by id) in crates/quetzal-wallet/src/db/pools.rs
- [ ] T117 Implement pool metadata sync from LocalStateQuery (periodically refresh pool list) in crates/quetzal-wallet/src/sync.rs
- [ ] T118 Implement WalletService::list_stake_pools() -> Vec<PoolMeta> in crates/quetzal-wallet/src/wallet.rs
- [ ] T119 Implement WalletService::create_delegation_tx(wallet_id, pool_id, passphrase) -> SignedTxBytes (stake key registration + delegation cert) in crates/quetzal-wallet/src/wallet.rs
- [ ] T120 [P] Implement WalletService::create_quit_delegation_tx(wallet_id, passphrase) -> SignedTxBytes in crates/quetzal-wallet/src/wallet.rs
- [ ] T121 [P] Implement WalletService::get_delegation_fees(wallet_id) -> FeeEstimate in crates/quetzal-wallet/src/wallet.rs
- [ ] T122 [P] Implement reward balance tracking: read rewards from LocalStateQuery, store in WalletMeta in crates/quetzal-wallet/src/wallet.rs

### quetzal-api — stake pool endpoints

- [ ] T123 Implement StakePoolResponse and DelegationResponse JSON types in crates/quetzal-api/src/json/pool.rs
- [ ] T124 Implement GET /stake-pools handler in crates/quetzal-api/src/handlers/stake_pools.rs
- [ ] T125 [P] Implement PUT /stake-pools/:pool_id/wallets/:wallet_id handler in crates/quetzal-api/src/handlers/stake_pools.rs
- [ ] T126 [P] Implement DELETE /stake-pools/*/wallets/:wallet_id handler (quit delegation) in crates/quetzal-api/src/handlers/stake_pools.rs
- [ ] T127 [P] Implement GET /wallets/:id/delegation-fees handler in crates/quetzal-api/src/handlers/stake_pools.rs
- [ ] T128 [P] Implement GET /wallets/:id/stake-keys handler in crates/quetzal-api/src/handlers/stake_pools.rs
- [ ] T129 Add stake pool routes to router in crates/quetzal-api/src/main.rs
- [ ] T130 Write axum tests for stake pool endpoints in crates/quetzal-api/tests/api_pools.rs

**Checkpoint**: Stake pool listing and delegation tx construction work

---

## Phase 11: Remaining Shelley Wallet Endpoints

**Purpose**: Complete the full Shelley wallet API surface

- [ ] T131 [P] Implement GET /wallets/:id/assets handler in crates/quetzal-api/src/handlers/assets.rs
- [ ] T132 [P] Implement GET /wallets/:id/assets/:policy_id and /:policy_id/:asset_name handlers in crates/quetzal-api/src/handlers/assets.rs
- [ ] T133 [P] Implement GET /wallets/:id/keys, GET /wallets/:id/keys/:index, GET /wallets/:id/keys/:role/:index handlers in crates/quetzal-api/src/handlers/keys.rs
- [ ] T134 [P] Implement POST /wallets/:id/signatures/:role/:index handler in crates/quetzal-api/src/handlers/keys.rs
- [ ] T135 [P] Implement PUT /wallets/:id/passphrase handler (re-encrypt root key) in crates/quetzal-api/src/handlers/passphrase.rs
- [ ] T136 [P] Implement GET /wallets/:id/coin-selections/random handler in crates/quetzal-api/src/handlers/transactions.rs
- [x] T137 [P] Implement POST /wallets/:id/migrations and POST /wallets/:id/migrations/plan; executing migrations submits the resulting transactions.
- [ ] T138 [P] Implement GET/PUT /settings handlers in crates/quetzal-api/src/handlers/settings.rs
- [ ] T139 [P] Implement GET /addresses and GET /addresses/:address handlers in crates/quetzal-api/src/handlers/addresses.rs
- [ ] T140 [P] Implement GET /wallets/:id/policy-id and GET /wallets/:id/policy-key handlers in crates/quetzal-api/src/handlers/keys.rs
- [ ] T141 Add all Shelley remaining routes to router in crates/quetzal-api/src/main.rs

---

## Phase 12: Byron Wallet API

**Purpose**: Full Byron wallet support (all /byron-wallets/* endpoints)

- [ ] T142 Add quetzal-byron-wallet module to crates/quetzal-wallet for Byron-specific key derivation (random + Icarus root key) and address management in crates/quetzal-wallet/src/byron/mod.rs
- [ ] T143 [P] Implement ByronWalletService: create_random, create_icarus, restore, delete, list, get in crates/quetzal-wallet/src/byron/wallet.rs
- [ ] T144 [P] Implement Byron address listing and import in crates/quetzal-wallet/src/byron/addresses.rs
- [ ] T145 [P] Implement Byron migration plan: select all Byron UTxOs, compute fee to sweep to Shelley address in crates/quetzal-wallet/src/byron/migration.rs
- [ ] T146 [P] Implement Byron coin selection (random) in crates/quetzal-wallet/src/byron/coinselect.rs
- [ ] T147 Implement POST /byron-wallets handler in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T148 [P] Implement GET/DELETE /byron-wallets/:id handlers in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T149 [P] Implement GET /byron-wallets/:id/addresses, POST /byron-wallets/:id/addresses/:id handlers in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T150 [P] Implement GET /byron-wallets/:id/transactions, GET /byron-wallets/:id/transactions/:txid handlers in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T151 [P] Implement GET /byron-wallets/:id/utxo, GET /byron-wallets/:id/statistics/utxos handlers in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T152 [P] Implement POST /byron-wallets/:id/payment-fees handler in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T153 [P] Implement GET /byron-wallets/:id/coin-selections/random handler in crates/quetzal-api/src/handlers/byron_wallets.rs
- [x] T154 [P] Implement POST /byron-wallets/:id/migrations and POST /byron-wallets/:id/migrations/plan with native transfer/history verification.
- [ ] T155 [P] Implement GET/PUT /byron-wallets/:id/assets* handlers in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T156 [P] Implement PUT /byron-wallets/:id/passphrase handler in crates/quetzal-api/src/handlers/byron_wallets.rs
- [ ] T157 Add all /byron-wallets/* routes to router in crates/quetzal-api/src/main.rs

---

## Phase 13: Shared (Multi-Sig) Wallet API

**Purpose**: Full shared wallet support (all /shared-wallets/* endpoints)

- [ ] T158 Add crates/quetzal-shared-wallet to workspace in Cargo.toml
- [ ] T159 Create crates/quetzal-shared-wallet/Cargo.toml with dependencies on quetzal-primitives, quetzal-crypto, pallas-primitives, pallas-txbuilder
- [ ] T160 Implement NativeScript types (sig, all, any, atLeast, before, after) in crates/quetzal-shared-wallet/src/script.rs
- [ ] T161 Implement ScriptTemplate (cosigner key placeholders + script structure) in crates/quetzal-shared-wallet/src/template.rs
- [ ] T162 Implement SharedWalletId derivation from payment script hash in crates/quetzal-shared-wallet/src/wallet.rs
- [ ] T163 Implement script address derivation for shared wallets in crates/quetzal-shared-wallet/src/address.rs
- [ ] T164 Implement SharedWalletService: create (from script template), restore, delete, get, list in crates/quetzal-shared-wallet/src/wallet.rs
- [ ] T165 [P] Implement partial transaction store: accumulate witness signatures per transaction in crates/quetzal-shared-wallet/src/partial_tx.rs
- [ ] T166 [P] Implement SharedTxBuilder: construct → returns unsigned tx; sign → adds one witness; finalize when threshold met in crates/quetzal-shared-wallet/src/txbuild.rs
- [ ] T167 Implement POST /shared-wallets handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T168 [P] Implement GET/DELETE /shared-wallets/:id handlers in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T169 [P] Implement GET /shared-wallets/:id/addresses handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T170 [P] Implement GET /shared-wallets/:id/utxo, GET /shared-wallets/:id/statistics/utxos handlers in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T171 [P] Implement GET/GET /shared-wallets/:id/transactions, GET /shared-wallets/:id/transactions/:txid handlers in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T172 [P] Implement POST /shared-wallets/:id/transactions-construct handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T173 [P] Implement POST /shared-wallets/:id/transactions-sign handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T174 [P] Implement POST /shared-wallets/:id/transactions-decode handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T175 [P] Implement POST /shared-wallets/:id/transactions-submit handler in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T176 [P] Implement GET /shared-wallets/:id/keys, GET /shared-wallets/:id/keys/:index, GET /shared-wallets/:id/keys/:role/:index handlers in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T177 [P] Implement GET /shared-wallets/:id/payment-script-template, GET /shared-wallets/:id/delegation-script-template handlers in crates/quetzal-api/src/handlers/shared_wallets.rs
- [ ] T178 Add all /shared-wallets/* routes to router in crates/quetzal-api/src/main.rs

---

## Phase 14: DRep Governance, Chain Data, and Infrastructure Endpoints

**Purpose**: Conway governance, block header, SMASH health, proxy tx

- [ ] T179 Implement DRepMeta types and DRepService::list, get, suggested, get_summary using LocalStateQuery in crates/quetzal-wallet/src/drep.rs
- [ ] T180 [P] Implement DRepService::create_vote_delegation_tx(wallet_id, drep_id, passphrase) in crates/quetzal-wallet/src/drep.rs
- [ ] T181 Implement GET /dreps handler in crates/quetzal-api/src/handlers/dreps.rs
- [ ] T182 [P] Implement GET /dreps/summary, GET /dreps/suggested, GET /dreps/:drep_id handlers in crates/quetzal-api/src/handlers/dreps.rs
- [ ] T183 [P] Implement PUT /dreps/:drep_id/wallets/:wallet_id handler in crates/quetzal-api/src/handlers/dreps.rs
- [x] T184 [P] Implement GET /blocks/latest/header handler from one acquired native chain snapshot in crates/quetzal-api/src/handlers/network.rs; validate across branch replacement (V57 rollback224/224).
- [ ] T185 [P] Implement GET /smash/health handler (proxy to configured SMASH_URL) in crates/quetzal-api/src/handlers/network.rs
- [ ] T186 [P] Implement POST /proxy/transactions handler (submit raw CBOR tx via TxSubmitter) in crates/quetzal-api/src/handlers/network.rs
- [ ] T187 Add all new routes (dreps, blocks, smash, proxy) to router in crates/quetzal-api/src/main.rs

---

## Phase 15: Polish and Production Readiness

**Purpose**: Test coverage, golden files, clippy, docs

- [ ] T140 Collect golden JSON response fixtures from cardano-wallet reference (capture via curl against Haskell server) into crates/quetzal-api/tests/golden/
- [ ] T141 Write golden-file comparison tests for all Daedalus-critical endpoints (wallet create/get, tx submit, network info) in crates/quetzal-api/tests/api_wallets.rs
- [ ] T142 [P] Add tracing::instrument to all WalletService methods in crates/quetzal-wallet/src/wallet.rs
- [ ] T143 [P] Implement graceful SIGTERM shutdown (axum graceful_shutdown + worker cancellation) in crates/quetzal-api/src/main.rs
- [ ] T144 [P] Add CORS headers (allow Daedalus localhost origins) via tower-http CorsLayer in crates/quetzal-api/src/main.rs
- [ ] T145 [P] Add request timeout middleware (default 30s) via tower-http TimeoutLayer in crates/quetzal-api/src/main.rs
- [x] T146 Run `cargo clippy --all-targets -- --deny warnings` and fix all warnings across workspace
- [x] T147 Run `cargo fmt --all` and verify no formatting changes needed
- [ ] T148 [P] Write property tests for fee calculation (fee monotonically increases with tx size) in crates/quetzal-wallet/tests/coinselect.rs
- [x] T149 Run `cargo test --workspace` and verify all tests pass
- [x] T150 Verify `nix develop --command cargo build --release` produces a working binary

---

## Dependencies

- Phase 1 (Setup) → unblocks all phases
- Phase 2 (Primitives) → unblocks Phase 3+
- Phase 3 (Crypto + Network) → unblocks Phase 4+
- Phase 4 (US1: Wallet Lifecycle) → unblocks Phase 5+
- Phase 5 (US2: Addresses) → needed by Phase 9 (sync addresses during chain sync)
- Phase 6 (US3: UTxO) → needed by Phase 7 (coin selection needs UTxO set)
- Phase 7 (US4: Transactions) → needed by Phase 10 (delegation txs)
- Phases 8, 9, 10 → independent of each other after Phase 7
- Phase 11 → independent, can run parallel to Phase 10
- Phase 12 → depends on all phases complete

## Parallel Opportunities

Within each phase, all `[P]`-marked tasks can run concurrently (different files, no shared state). Phases 8, 9, 10, and 11 can run in parallel after Phase 7 completes.

- [x] T188 Enforce the pinned pre-Conway transaction writing guard from acquired node era and pass the unchanged owned Babbage→Conway restoration/payment/certificate control.
- [x] T189 Match the reference five-minute public wallet sync tolerance and elapsed-time progress, preserving worker/checkpoint state; validate boundaries and shared activation HTTP parity (V58 families102/102).
