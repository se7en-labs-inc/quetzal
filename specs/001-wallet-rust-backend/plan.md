# Implementation Plan: Quetzal Wallet Backend

**Branch**: `001-wallet-rust-backend` | **Date**: 2026-08-22 | **Spec**: [spec.md](./spec.md)

## Summary

### Transport and contract reconciliation — 2026-11-09

Reuse this feature's completed wallet implementation. Reconcile the original prose
with the reviewed executable contract before implementation. Keep the external
conformance checkout read-only and preserve the historical V58 evidence.

Add complete TLS configuration and an IP bind address (loopback default). Validate
the three TLS paths together before opening wallet state. Use Rustls with
WebPkiClientVerifier and only the supplied CA roots, validating the server
certificate/private key at startup. Require client-auth purpose, reject anonymous
clients, and serve HTTP/1.1 and HTTP/2 through the unchanged Axum router. Bound
TLS handshake duration and concurrent connections, and close plaintext on the
secure listener. Keep HTTP on loopback for isolated semantic fixtures; reject
explicit non-loopback HTTP configuration.

Verify configuration failures and real TLS positive/rejection cases using fresh
private certificates against production transport. Run workspace tests, formatting,
Clippy and release build through Nix. Build the unchanged external runner with
outputs outside its checkout, then run the full 646/38 contract against fresh
candidate/adapter binaries and independent provisioned oracles. Record unavailable
fixtures/tools separately from failures. Actual Daedalus startup and performance
qualification remain separate acceptance gates.

### Continuation plan — 2026-11-06

1. Repair stale worktree paths after the repository directory rename and retain
   the existing dirty wallet implementation.
2. Establish the baseline through the project's Nix development shell.
3. Rename crates, executable, package metadata and documentation to Quetzal.
4. Complete missing wallet behavior against the adjacent conformance contract,
   including configurable isolated service startup and an internal adapter.
5. Run formatting, clippy and workspace tests, then the supplied HTTP/internal
   profiles and full owned-node execution. Preserve fresh evidence and describe
   incomplete coverage accurately. Follow the user's failure-recovery agreement
   before changing an approach after a failed check.

### Shelley partial balancing continuation — 2026-11-07

Reuse the existing ledger decoders, multi-asset values, signed-size estimator and
live protocol parameters. Preserve supplied outputs, validity and auxiliary data;
retain and resolve preselected inputs, add available owned inputs only as needed,
and converge fee/change without mutating wallet storage. Reject existing
collateral fields in reference order before checking funding. Reject unsupported
script execution, withdrawals, certificates and unresolved external inputs
explicitly until their ledger accounting is implemented. Validate with empty and
funded partial payments, native-asset conservation, collateral precedence,
many-input size boundaries and actual node confirmation. Delegation-fee reads
must use the same unsigned empty-payment prototype and live deposit, with no
synthetic transaction or delegation-state mutation.

The pinned funded validity contract rounds both explicitly relative bounds up
to the first slot at or after the requested wall time; an omitted lower bound
remains slot zero. This differs from the generic floor/ceiling slotting helpers.

### Shared wallets continuation — 2026-11-09

Replace shared transaction stubs with existing production transaction handlers,
using shared account derivation and input native scripts in construction/fees.
Persist cosigner patches and initial address activation atomically in Sled;
validate all patch entries before writing, retain the wallet's identity and own
account key, and start restoration only after both templates become complete.
Preserve raw body identity and existing witnesses across independent signatures.
Verify activation, identical peer addresses, wrong-password state preservation,
partial/merged signing, real submission and both peers' confirmed history with
the unchanged funded shared profile. Existing C006/C008 cover this work.

The Shelley and Byron API families must permit identical public wallet IDs.
Use separate Sled stores and worker registries for these families; share the
node connection configuration, network tip and submission lock. This is needed
for Shelley creation alongside an Icarus wallet derived from the same mnemonic.
Route the entire Byron subtree through its family service and restore both
registries on startup. Verify same-ID independent lifecycle/passphrase/UTxO
state and the unchanged HTTP families profile. Existing prototype data remains
preserved; cross-store migration needs separate validation before any claim of
backward-compatible storage migration.

Persist pool metadata settings in the service's Sled settings tree. Implement
the reference SMASH status request at `/api/v1/status`, mapping valid status
payloads to available/unavailable and connection or malformed responses to
unreachable. Bound request duration and response bytes, validate absolute URLs,
and return no_smash_configured for direct/none sources. Verify local real HTTP
responses and restart persistence before replaying the network/extended suites.

Credential transition work reuses the existing authenticated V2 envelope and
legacy key encryption. Persist scheme-aware legacy credentials, authenticate
before any change, migrate Shelley/shared keys before invoking the unlock
callback, preserve timestamps during migration and atomically update key and
passphrase metadata. Failed migration must leave V1 intact while allowing the
authorized callback. Password rotation uses the same transaction. Expose a
deterministic entropy/backend hook only through the internal conformance seam;
normal wallet methods use OS entropy and the actual Sled commit. Exercise all
61 captured transitions and their independent laws against these production
functions, including callback failures and an injected backend commit failure.

Implement the pinned native selection component from cardano-coin-selection
176611048d5b, random1.3.1 and splitmix0.1.3.1: signed64 seed conversion, exact
bounded draws, asset-priority round robin, minimal/optimal targets, proportional
change with exact largest-remainder rounding, surplus asset coalescing, bundle
splitting and structured shortfalls. Keep intermediate quantities exact and
preserve deterministic ordering. Verify upstream RNG examples and the entire
independent seeded corpus before using the algorithm in other wallet paths.

Implement a Rust wallet backend matching the pinned cardano-wallet API behavior and client-authenticated TLS. Actual Daedalus launch-and-use compatibility remains unqualified until a pinned client is exercised. Uses Pallas, Sled, Axum and native node-to-client protocols.

---

## Technical Context

**Language/Version**: Rust stable (managed by fenix in nix flake)
**Primary Dependencies**:
- `pallas 0.32` — pallas-primitives, pallas-traverse, pallas-addresses, pallas-network, pallas-txbuilder, pallas-crypto
- `ed25519_bip32` — CIP-1852/BIP-44 hierarchical key derivation
- `axum 0.8` — HTTP server
- `sled 0.34` — embedded key-value storage
- `tokio 1` — async runtime
- `argon2` — passphrase KDF
- `proptest` — property-based testing
- `tower-http` — tracing and timeout middleware
- `ciborium` — CBOR encode/decode for sled values
- `zeroize` — memory-zero key material on drop

**Storage**: sled (embedded key-value, no SQL)
**Testing**: `cargo test`, `proptest`, axum test helpers, integration tests against live node socket
**Target Platform**: Linux x86_64
**Project Type**: HTTP service (Daedalus backend)
**Performance Targets (unqualified)**: ≥500 blocks/sec chain sync; ≤100ms p95 API latency; require an agreed reproducible workload before acceptance
**Constraints**: <512 MB RAM for 10k-address wallet; crash-safe (no partial blocks in sled)
**Scale/Scope**: Single-user embedded service; 1 instance per Daedalus installation

---

## Constitution Check

- [X] I. Safety-First: `zeroize` on all key material; no `unwrap()` in library code; `thiserror` everywhere
- [X] II. Era-Aware: pallas `MultiEraBlock`/`MultiEraTx` handles era switching automatically
- [X] III. Type Safety: Newtypes for WalletId, TxHash, Address, Coin; `#[must_use]` on Coin arithmetic
- [X] IV. REST API Contract: cardano-wallet OpenAPI spec is ground truth; golden-file tests for Daedalus endpoints
- [X] V. Reproducible Builds: Nix flake + crane + fenix-pinned toolchain
- [X] VI. Sled Storage: All persistence via named sled Trees; no SQL dependencies
- [X] VII. Comprehensive Testing: unit + property + integration + API golden test layers
- [X] VIII. Code Quality Gates: rustfmt, clippy --deny warnings, all tests green before merge

---

## Project Structure

### Documentation

```text
specs/001-wallet-rust-backend/
├── spec.md
├── plan.md              ← this file
├── research.md
├── data-model.md
├── contracts/openapi.md
└── tasks.md
```

### Source Code

```text
Cargo.toml                              ← workspace root with [workspace.dependencies]
crates/
  quetzal-primitives/src/
    lib.rs
    wallet.rs                           ← WalletId, WalletMeta, WalletStatus
    tx.rs                               ← TxHash, TxRecord, TxStatus, PendingTx
    utxo.rs                             ← UtxoId, UtxoEntry, Value (lovelace + assets)
    address.rs                          ← Address newtype, AddressState, DerivationPath
    coin.rs                             ← Coin (u64 lovelace)
    checkpoint.rs                       ← Checkpoint { slot, block_hash }
    pool.rs                             ← PoolId, PoolMeta
    codec.rs                            ← sled binary key helpers (big-endian slot, composite)
  quetzal-primitives/tests/
    roundtrip.rs                        ← CBOR/JSON roundtrip proptest

  quetzal-crypto/src/
    lib.rs
    key.rs                              ← Bip32PrivateKey, Bip32PublicKey
    derivation.rs                       ← CIP-1852 and BIP-44 derivation paths
    passphrase.rs                       ← authenticated storage envelopes; mnemonic derivation retains PBKDF2
    signing.rs                          ← Ed25519Signer impl
  quetzal-crypto/tests/
    derivation.rs                       ← CIP-1852 known-answer vectors
    passphrase.rs                       ← encrypt/decrypt roundtrips

  quetzal-address/src/
    lib.rs
    shelley.rs                          ← Enterprise and base address derivation
    icarus.rs                           ← BIP-44 Icarus address
    byron.rs                            ← Byron random address
    discovery.rs                        ← Sequential gap-based discovery (gap=20)
  quetzal-address/tests/
    known_vectors.rs                    ← CIP test suite vectors
    discovery.rs                        ← Gap limit property tests

  quetzal-network/src/
    lib.rs
    client.rs                           ← NodeClient: Unix socket + N2C handshake
    chainsync.rs                        ← ChainSyncDriver wrapping pallas chainsync::Client
    txsubmit.rs                         ← LocalTxSubmission
    statequery.rs                       ← LocalStateQuery (protocol params, pools)
  quetzal-network/tests/
    mock_node.rs                        ← In-process mock node for unit tests

  quetzal-wallet/src/
    lib.rs
    db/
      mod.rs                            ← WalletDb: sled handle + tree accessors
      wallets.rs                        ← wallet CRUD
      utxos.rs                          ← UTxO set operations
      txs.rs                            ← tx history slot-ordered range queries
      addresses.rs                      ← address state management
      checkpoints.rs                    ← checkpoint read/write
      pools.rs                          ← pool metadata cache
    wallet.rs                           ← WalletService: orchestrates all operations
    sync.rs                             ← ChainSyncWorker: block processing + rollback
    coinselect.rs                       ← random-improve + largest-first
    txbuild.rs                          ← transaction construction (pallas-txbuilder)
    fees.rs                             ← linear fee calculation
  quetzal-wallet/tests/
    wallet.rs
    coinselect.rs
    txbuild.rs

  quetzal-api/src/
    main.rs                             ← tokio::main, router setup, startup
    config.rs                           ← Config { socket_path, db_path, port, network_magic }
    state.rs                            ← AppState { wallet_service, node_client }
    error.rs                            ← ApiError → cardano-wallet JSON error format
    handlers/
      wallets.rs
      addresses.rs
      transactions.rs
      fees.rs
      utxo.rs
      network.rs
      stake_pools.rs
      assets.rs
      keys.rs
      passphrase.rs
      migration.rs
      settings.rs
    json/
      wallet.rs                         ← WalletResponse, PostWalletRequest
      transaction.rs                    ← TransactionResponse, PostTransactionRequest
      address.rs                        ← AddressResponse
      network.rs                        ← NetworkInfoResponse, NetworkParamsResponse
      pool.rs                           ← StakePoolResponse, DelegationResponse
      error.rs                          ← ErrorResponse { code, message }
  quetzal-api/tests/
    api_wallets.rs
    api_transactions.rs
    api_network.rs
    api_pools.rs
    golden/                             ← reference JSON from cardano-wallet
```

---

## Complexity Tracking

No constitution violations.

---

## REST API Surface

The pinned `swagger.yaml` and reviewed Haskell corrections define all 78 paths
and 96 operations. The route overview below is an index, not an independent
contract; the executable assertions control methods, payloads and side effects.

```
# Shelley single-sig wallets
POST   /wallets
GET    /wallets
GET    /wallets/:id
DELETE /wallets/:id
PUT    /wallets/:id/passphrase
GET    /wallets/:id/addresses
GET    /wallets/:id/utxo
GET    /wallets/:id/statistics/utxos
GET    /wallets/:id/transactions
POST   /wallets/:id/transactions
GET    /wallets/:id/transactions/:txid
DELETE /wallets/:id/transactions/:txid
POST   /wallets/:id/transactions-construct
POST   /wallets/:id/transactions-sign
POST   /wallets/:id/transactions-decode
POST   /wallets/:id/transactions-submit
POST   /wallets/:id/transactions-balance
POST   /wallets/:id/payment-fees
POST   /wallets/:id/coin-selections/random
GET    /wallets/:id/assets
GET    /wallets/:id/assets/:policy_id
GET    /wallets/:id/assets/:policy_id/:asset_name
GET    /wallets/:id/keys
POST   /wallets/:id/keys/:index
GET    /wallets/:id/keys/:role/:index
POST   /wallets/:id/signatures/:role/:index
GET    /wallets/:id/stake-keys
GET    /wallets/:id/delegation-fees
POST   /wallets/:id/migrations
POST   /wallets/:id/migrations/plan
POST   /wallets/:id/policy-id
GET    /wallets/:id/policy-key

# Byron wallets
POST   /byron-wallets
GET    /byron-wallets/:id
DELETE /byron-wallets/:id
PUT    /byron-wallets/:id/passphrase
GET    /byron-wallets/:id/addresses
PUT    /byron-wallets/:id/addresses/:address_id  (import address)
GET    /byron-wallets/:id/assets
GET    /byron-wallets/:id/assets/:policy_id
GET    /byron-wallets/:id/assets/:policy_id/:asset_name
GET    /byron-wallets/:id/utxo
GET    /byron-wallets/:id/statistics/utxos
GET    /byron-wallets/:id/transactions
GET    /byron-wallets/:id/transactions/:txid
POST   /byron-wallets/:id/payment-fees
POST   /byron-wallets/:id/coin-selections/random
POST   /byron-wallets/:id/migrations
POST   /byron-wallets/:id/migrations/plan

# Shared (multi-sig) wallets
POST   /shared-wallets
GET    /shared-wallets/:id
DELETE /shared-wallets/:id
GET    /shared-wallets/:id/addresses
GET    /shared-wallets/:id/utxo
GET    /shared-wallets/:id/statistics/utxos
GET    /shared-wallets/:id/transactions
GET    /shared-wallets/:id/transactions/:txid
POST   /shared-wallets/:id/transactions-construct
POST   /shared-wallets/:id/transactions-sign
POST   /shared-wallets/:id/transactions-decode
POST   /shared-wallets/:id/transactions-submit
GET    /shared-wallets/:id/keys
POST   /shared-wallets/:id/keys/:index
GET    /shared-wallets/:id/keys/:role/:index
PATCH  /shared-wallets/:id/payment-script-template
PATCH  /shared-wallets/:id/delegation-script-template

# Stake pools
GET    /stake-pools
POST   /stake-pools/maintenance-actions
PUT    /stake-pools/:pool_id/wallets/:wallet_id
DELETE /stake-pools/*/wallets/:wallet_id

# DRep governance (Conway)
GET    /dreps
GET    /dreps/summary
GET    /dreps/suggested
GET    /dreps/:drep_id
PUT    /dreps/:drep_id/wallets/:wallet_id

# Network
GET    /network/information
GET    /network/parameters
GET    /network/clock

# Chain data
GET    /blocks/latest/header

# Addresses
POST   /addresses
GET    /addresses/:address_id

# Settings
GET    /settings
PUT    /settings

# Infrastructure
GET    /smash/health
POST   /proxy/transactions
```

## Additional Workspace Crate

Shared wallet logic lives in the existing `quetzal-wallet` crate and reuses its
transaction, persistence and signing paths. No additional shared-wallet crate or
hidden server-side witness accumulator is required.
# Pending submission continuation

Persist allocated internal change addresses independently of pending transaction
membership. Known-address reads include external, used internal, and allocated
change addresses. Preview calls do not allocate; admission commits pending bytes
and allocations atomically, and forgetting preserves the generated address book.
Keep local submission provenance after confirmation, and remove it atomically
when forgotten. History rediscovery does not recreate local submission ownership.
Guard forgetting against concurrent block updates with the wallet snapshot and
pending bytes inside the database transaction.

Reuse the wallet's persisted pending transaction records for local resubmission.
Persist the latest attempt slot with compare-and-swap, without recreating removed
records. Follow the pinned wallet's one-second rate limit and ten expected-block
interval, calculated as ceiling(10 / activeSlotsCoeff). A per-wallet task owns
cancellation and a cancellation-safe node connection. Preserve signed bytes and
ledger identity on retries. Retain the exact node all-inputs-spent mempool
rejection for retry; malformed signatures remain rejected without pending state.
Validate with unchanged owned pending/checkpoint/expiry/retry controls, including
both Byron styles and an independently running adversary wallet.

For the scheduled Babbage→Conway control, reject transaction construction and
balancing before Conway with the pinned node_not_yet_in_recent_era403 envelope,
using the acquired node protocol-parameter era. Perform this check before any
pending admission or wallet mutation. Keep restoration/history active across the
fork, then allow Conway writing without restarting the wallet. Verify with the
unchanged verify-era-boundary helper; it uses public vectors and requires no
key-oracle executable. The suite owner's reference identity update now permits
the source-built Haskell helper for verify-rollback without changing assertions.

ChainSync AwaitReply proves the consumer reached the node's tip even while
producers are paused. Expose that protocol event instead of swallowing it until
a new block arrives. Mark the existing wallet checkpoint ready atomically without
changing its slot, height, metadata, pending bytes or history. Apply this to
restored workers and the global chain readiness flag; retain protocol MustReply
state for the following receive. Verify by unchanged pending crash/restart and
checkpoint controls, including a paused producer with no new block available.
Query and validate the node era before attempting era-specific protocol parameter
payload decoding. The pinned Pallas parameter shape cannot decode Babbage, so a
check only after parameters incorrectly produces500. Use a separate acquired
hard-fork era query and release; abort the owned connection before returning a
pre-Conway403. Retain the later acquired-parameter era check for consistency.

The latest block header must describe the connected node's current chain even
when global pool-history replay is reconnecting after a branch replacement.
Acquire one local-state snapshot and query its chain point and block number,
then release and close the owned connection. Do not project this endpoint from
the asynchronous global replay cache. Validate with the unchanged native fork,
rollback and reapplication control.

Match upstream revision2fc5c383's Primitive.SyncProgress.syncProgress and CLI
syncToleranceOption: the default tolerance is300 seconds, and readiness compares
the checkpoint's relative time with wall time, rather than requiring exact node
tip equality. Compute older-checkpoint percentages from elapsed milliseconds.
Apply this to public wallet state only, preserving internal checkpoint/worker
readiness and all ledger state. Verify tolerance boundaries, era slot timing and
shared-wallet activation against the unchanged HTTP family profile.
