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

**Decision**: argon2id KDF + AES-256-GCM AEAD for encrypting root keys stored in sled. No PBKDF2.

**Rationale**: argon2id is the current best-practice password hashing / KDF (memory-hard, side-channel resistant, winner of PHC). The Haskell cardano-wallet uses scrypt; we use argon2id as a strict improvement. AES-256-GCM provides authenticated encryption — any tampering with the ciphertext is detected before decryption.

**Encryption scheme** (`PassphraseProtected` struct stored in sled):
```
struct EncryptedKey {
    argon2_salt: [u8; 32],   // random 32 bytes, generated fresh on every encrypt
    aes_nonce: [u8; 12],     // random 12 bytes (96-bit), generated fresh on every encrypt
    ciphertext: Vec<u8>,     // AES-256-GCM ciphertext of the 96-byte XPrv extended key
    tag: [u8; 16],           // AES-256-GCM authentication tag (appended by aes-gcm crate)
}
```

**Key derivation parameters** (argon2id):
- `m_cost = 65536` (64 MiB memory) — prevents GPU brute-force
- `t_cost = 3` (3 iterations)
- `p_cost = 4` (4 parallel lanes)
- Output length: 32 bytes → used directly as AES-256 key

**Critical invariants**:
1. `argon2_salt` and `aes_nonce` MUST be independently generated via `rand::rngs::OsRng` on every call to `encrypt()`. Never reuse.
2. The 32-byte derived key is `zeroize`d immediately after AES-GCM encrypt/decrypt completes.
3. The decrypted `XPrv` bytes are wrapped in a `Zeroizing<[u8; 96]>` and zeroed on drop.
4. `ciphertext` includes the 16-byte GCM tag (appended by `aes_gcm::AeadInPlace::encrypt_in_place`); decryption verifies this tag before returning plaintext.
5. Wrong passphrase returns `Err(PassphraseError::InvalidPassphrase)` — never panics or leaks timing info beyond the argon2 computation itself.

**No PBKDF2 anywhere in the codebase.** The `pbkdf2` crate has been removed from workspace dependencies.

---

## Decision: Chain Synchronisation

**Decision**: Use `pallas_network` N2C (node-to-client) ChainSync miniprotocol over Unix socket.

**Rationale**: The pallas-network crate provides `Bearer::connect_unix(path)` and `chainsync::Client` implementing the full Ouroboros ChainSync miniprotocol. The `NodeToClient` multiplexer handles protocol fanout. Local-state-query provides protocol parameters and stake pool data. This is the same wire protocol used by the Haskell wallet.

**Intersection strategy**: On startup, send the last N checkpoints (slots+block hashes) as `FindIntersect` points. Roll back to the intersection point if the node has forked. If no intersection found, sync from genesis.

**Rollback handling**: Keep a configurable rollback window (default 2160 blocks = k parameter). On `RollBackward`, revert the sled trees to the checkpoint at the rollback target slot using a sled transaction that deletes entries created after that slot.

---

## Decision: Storage Layout (sled)

**Decision**: Named sled `Tree`s with binary composite keys for ordering; no SQL.

**Rationale**: sled provides ACID batch operations across multiple trees. Composite keys enable range-scan-based pagination for transaction history (key: `slot_be_u64 ++ tx_hash`) without secondary indices for the common case. Secondary index trees (prefixed `idx:`) are maintained within the same sled batch to ensure consistency.

**Tree layout**:

| Tree name | Key | Value |
|---|---|---|
| `wallets` | wallet_id[32] | WalletMeta (CBOR) |
| `utxos:{wid}` | tx_hash[32] ++ out_idx[4 BE] | UtxoEntry (CBOR) |
| `txs:{wid}` | slot[8 BE] ++ tx_hash[32] | TxRecord (CBOR) |
| `addrs:{wid}` | addr_bytes | AddressState (CBOR) |
| `checkpoints:{wid}` | slot[8 BE] | block_hash[32] |
| `pending:{wid}` | tx_hash[32] | PendingTx (CBOR) |
| `pools` | pool_id[28] | PoolMeta (CBOR) |
| `idx:addr_wid:{addr}` | addr_bytes | wallet_id[32] |

---

## Decision: API Layer

**Decision**: axum 0.8 with `tower` middleware; JSON responses matching the cardano-wallet OpenAPI spec.

**Rationale**: axum provides typed extractors and handler ergonomics that match the REST API surface well. `tower-http` provides tracing, CORS, and request timeout middleware. The OpenAPI spec from the Haskell wallet (`specifications/api/swagger.yaml`) is used as the ground truth for request/response shapes.

**Error format**: All errors follow the cardano-wallet format:
```json
{"code": "wallet_not_found", "message": "No wallet with id: ..."}
```

**Daedalus-critical endpoints** (must be byte-for-byte JSON-compatible):
- `POST /wallets`, `GET /wallets`, `GET /wallets/{id}`, `DELETE /wallets/{id}`
- `GET /wallets/{id}/addresses`
- `GET /wallets/{id}/transactions`, `POST /wallets/{id}/transactions`
- `POST /wallets/{id}/transactions-submit`
- `GET /network/information`

---

## Decision: Coin Selection

**Decision**: Implement largest-first and random-improve algorithms matching the Haskell reference.

**Rationale**: Daedalus uses the random-improve strategy for normal payments. Largest-first is used as a fallback. Both are specified in the Haskell cardano-wallet codebase. We port the same algorithm to Rust to ensure identical fee calculations.

**Fee calculation**: Linear fee model `a + b * tx_size_bytes` where a (intercept) and b (coefficient) come from protocol parameters fetched via LocalStateQuery.

---

## Decision: Root Key V2 Encryption Scheme (from spec 003)

**Decision**: Support only V2 (`argon2id-v2`) in new wallets; expose `encryption_method` field in wallet API responses.

**PassphraseScheme values** (matching cardano-wallet `ToText` instances):
- `"scrypt"` — V1 legacy (cardano-wallet Haskell < 2026)
- `"pbkdf2-hmac-sha512"` — V1 legacy variant
- `"argon2id-v2"` — Current standard (our Rust impl always uses this)

**API field name**: `encryption_method` in `WalletResponse` (matches Haskell `ApiWallet.passphrase.encryption_method`).

**Migration note**: Since this is a fresh Rust implementation, all wallets created here use `argon2id-v2`. If a user imports an existing wallet via mnemonic restoration, the key is re-encrypted with V2 on first creation.

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
