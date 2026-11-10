# Research: Quetzal Wallet Backend

**Branch**: `001-wallet-rust-backend` | **Date**: 2026-08-22

---

## Decision: Pallas Crate Version

**Decision**: Use pallas 0.32.x (stable) rather than 1.0.0-alpha.5 (unstable).

**Rationale**: pallas 0.32 is the current stable release on crates.io. The 1.0.0-alpha.5 series (seen in the git repo) renames sub-crates and changes APIs; using it in production before it stabilises is risky. We pin 0.32 in workspace and will upgrade when it stabilises.

**Key sub-crates available at 0.32**:
- `pallas-primitives` — Byron/Shelley/Alonzo/Babbage/Conway ledger types + CBOR codecs
- `pallas-traverse` — Multi-era traversal (`MultiEraTx`, `MultiEraOutput`, `MultiEraBlock`)
- `pallas-addresses` — Shelley/Byron/Icarus address encoding/decoding (Bech32, base58)
- `pallas-network` — miniprotocols: ChainSync, BlockFetch, LocalTxSubmission, LocalStateQuery; Unix socket bearer
- `pallas-txbuilder` — `StagingTransaction`, `BuildConway` trait, `Ed25519Signer` trait
- `pallas-crypto` — Ed25519 keys, hashing
- `pallas-codec` — CBOR encode/decode utilities

**Alternatives considered**: `cardano-serialization-lib` (WASM-oriented, poor Rust ergonomics), `cardano-multiplatform-lib` (not idiomatic Rust) — both rejected.

---

## Decision: HD Key Derivation

**Decision**: Use `ed25519_bip32` crate directly for BIP-32/CIP-1852 key derivation.

**Rationale**: pallas-crypto re-exports `ed25519_bip32` internally. The wallet example in the pallas repo (`examples/wallet/src/hd.rs`) uses `ed25519_bip32::XPrv` as `Bip32PrivateKey`. This is the same scheme used by cardano-wallet (Haskell) for Shelley key derivation (CIP-1852: `m/1852'/1815'/account'/role/index`). For Byron/Icarus derivation (CIP-0011), the BIP-44 path `m/44'/1815'/account'/0/index` applies.

**Alternatives considered**: Reimplementing derivation from scratch — rejected, `ed25519_bip32` is well-tested and pallas-validated.

## Decision: Root Key Encryption at Rest

**Decision**: current authenticated root envelopes follow the pinned
cardano-crypto-wallet backend, implemented in `quetzal-crypto/src/rootkey.rs`.
They use Argon2id and XChaCha20-Poly1305 with the production parameters recorded
in `PRODUCTION_KDF_PARAMETERS`. Preserve the distinction between envelope metadata
and the actual backend KDF invocation; frozen independent vectors check the bytes.
The earlier AES-GCM-only design is superseded by these reference-compatible
current/legacy envelope and wallet credential transitions.

**Critical invariants**: normal encryption uses OS entropy for fresh salt/nonce;
authenticate before exposing key material; zeroize plaintext keys and wrapping
buffers; preserve identity through rotation/migration; failed authentication must
not mutate credentials. Supported legacy formats retain their exact family-aware
password schemes. The deterministic entropy seam exists only for conformance.

**PBKDF2 remains required for reference-compatible root derivation and credential operations.** Storage policy must not change wallet keys, addresses or identity. Independent vectors and restoration verify these.

---

## Decision: Chain Synchronisation

**Decision**: Use `pallas_network` N2C (node-to-client) ChainSync miniprotocol over Unix socket.

**Rationale**: The pallas-network crate provides `Bearer::connect_unix(path)` and `chainsync::Client` implementing the full Ouroboros ChainSync miniprotocol. The `NodeToClient` multiplexer handles protocol fanout. Local-state-query provides protocol parameters and stake pool data. This is the same wire protocol used by the Haskell wallet.

**Intersection strategy**: On startup, send the last N checkpoints (slots+block hashes) as `FindIntersect` points. Roll back to the intersection point if the node has forked. If no intersection found, sync from genesis.

**Rollback handling**: use undo records or equivalent reconstruction to restore all ledger mutations, including older inputs spent on the abandoned branch. Deleting newer entries alone is insufficient. Restore still-valid local submissions to pending, remove discovered-only effects, preserve retry provenance and verify exact native balances/history before and after reapplication. Retention and deeper recovery must follow actual network configuration rather than assume a universal 2160-block window.

---

## Decision: Storage Layout (sled)

**Decision**: Named sled `Tree`s with binary composite keys for ordering; no SQL.

**Rationale**: sled provides ACID batch operations across multiple trees. Composite keys enable range-scan-based pagination for transaction history (key: `slot_be_u64 ++ tx_hash`) without secondary indices for the common case. Secondary index trees (prefixed `idx:`) are maintained within the same sled batch to ensure consistency.

**Tree layout**:

| Tree name | Key | Value |
|---|---|---|
| `wallets` | wallet_id[20] | WalletMeta (CBOR) |
| `utxos:{wid}` | tx_hash[32] ++ out_idx[4 BE] | UtxoEntry (CBOR) |
| `txs:{wid}` | slot[8 BE] ++ tx_hash[32] | TxRecord (CBOR) |
| `addrs:{wid}` | addr_bytes | AddressState (CBOR) |
| `checkpoints:{wid}` | slot[8 BE] | block_hash[32] |
| `pending:{wid}` | tx_hash[32] | PendingTx (CBOR) |
| `pools` | pool_id[28] | PoolMeta (CBOR) |
| `idx:addr_wid:{addr}` | addr_bytes | wallet_id[20] |

---

## Decision: API Layer

**Decision**: axum 0.8 with `tower` middleware; JSON responses matching the cardano-wallet OpenAPI spec.

**Rationale**: axum provides typed extractors and handler ergonomics that match the REST API surface well. `tower-http` provides tracing, CORS, and request timeout middleware. The OpenAPI spec from the Haskell wallet (`specifications/api/swagger.yaml`) is used as the ground truth for request/response shapes.

**Error format**: All errors follow the cardano-wallet format:
```json
{"code": "no_such_wallet", "message": "I couldn't find a wallet with the given id: ..."}
```

**Examples of consumer-critical endpoints** (statuses, media, schemas, deterministic values and side effects must match; JSON object key order is not a compatibility rule):
- `POST /wallets`, `GET /wallets`, `GET /wallets/{id}`, `DELETE /wallets/{id}`
- `GET /wallets/{id}/addresses`
- `GET /wallets/{id}/transactions`, `POST /wallets/{id}/transactions`
- `POST /wallets/{id}/transactions-submit`
- `GET /network/information`

---

## Decision: Coin Selection

**Decision**: preserve reference selection, including the separately pinned seeded
component and live wallet selection behavior. Eligibility, exact conservation,
minimum ADA, native-asset change, collateral and failure precedence require
independent assertions; naming a strategy does not establish compatibility.

**Fee calculation**: use live protocol parameters and exact signed-size/ledger
accounting. The linear byte fee is only one component; script execution prices,
reference scripts, witnesses, certificates, withdrawals and deposits must be
accounted for or explicitly rejected according to the pinned operation contract.

---

## Decision: Root Key V2 Encryption Scheme (from spec 003)

**Decision**: expose the actual persisted scheme through `encryption_method`, preserving family-aware current and legacy behavior and authenticated migration.

**PassphraseScheme values** (matching cardano-wallet `ToText` instances):
- `"scrypt"` — supported legacy credentials
- `"pbkdf2-hmac-sha512"` — reference PBKDF2 credential preparation
- `"argon2id-v2"` — authenticated V2 scheme when attached to that wallet

**API field name**: `encryption_method` in `WalletResponse` (matches Haskell `ApiWallet.passphrase.encryption_method`).

**Migration note**: do not assume all families use V2. Preserve reference creation,
unlock and rotation transitions for Shelley/shared/Byron/Icarus, including callback
and backend-failure behavior. Mnemonic restoration creates fresh Quetzal state;
it is distinct from database import and supported legacy credential migration.

---

## Decision: DRep Endpoints (from spec 010)

**LSQ queries** (Conway+ only):
- `GetDRepState` (tag 25) — returns all DRep registrations (empty input = all); output: `Map DRepCredential DRepState`
  - `DRepState` fields: `expiry_epoch`, `anchor` (url + data_hash), `deposit`, `delegations`
- `GetDRepStakeDistr` (tag 26) — returns voting power per DRep; sentinel entries (`AlwaysAbstain`, `AlwaysNoConfidence`) are excluded from the list
- `GetEpochNo` (tag 1) — current epoch, used to compute `status` (`active` if `expiry_epoch > current_epoch`)

**DRepLayer**: Facade with 900-second cache TTL; pre-warmed at startup. Both `GET /v2/dreps` and `GET /v2/dreps/suggested` call through `DRepLayer`.

**Off-chain metadata** (CIP-0119): Background worker fetches anchor URLs, verifies Blake2b-256 hash, caches in sled. Retry interval: 15 minutes. `ipfs://` URLs rewritten to `https://ipfs.blockfrost.dev/ipfs/<CID>`.

**List vs detail split**: `GET /dreps` and `GET /dreps/suggested` return only `name` (givenName) from off-chain doc. Full `ApiDRepMetadata` (objectives, motivations, qualifications, payment_address, do_not_list, references) only via `GET /dreps/{drep_id}`.

**Suggested DReps algorithm**: Sort by voting_power descending → drop top 35 → filter to `active` + has metadata + `do_not_list = false` → uniform random sample up to 20 (max 200) via partial Fisher-Yates.

**voting_power.quantity** serialised as JSON String (lovelace exceeds JS `Number.MAX_SAFE_INTEGER`).

**DRep ID** bech32-encoded with `drep` HRP (matches cardano-cli).

**anchor.data_hash** lowercase hex, no `0x` prefix.

---

## Decision: Testing Strategy

**Decision**: Three-layer testing matching the Haskell test coverage:

1. **Unit + property tests** (`cargo test` + `proptest`): All pure functions in `quetzal-primitives`, `quetzal-crypto`, `quetzal-address`. CBOR/JSON roundtrips for every type.

2. **Integration tests** (`tests/` directory in `quetzal-wallet` and `quetzal-network`): Against a mock sled DB and a real cardano-node socket (via environment variable `CARDANO_NODE_SOCKET_PATH`). Uses `testcontainers` or a locally spawned node.

3. **API tests** (`tests/` in `quetzal-api`): `axum::test` helpers exercising every HTTP endpoint. Golden-file comparison against cardano-wallet reference responses for Daedalus-critical paths.

**Property test targets**: address derivation roundtrips, coin selection balance invariants, CBOR encode/decode roundtrips, fee calculation monotonicity.
