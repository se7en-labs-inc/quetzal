# Data Model: Quetzal Wallet Backend

## Core Entities

### WalletMeta
- `id: WalletId` (32-byte Blake2b-256 of root public key)
- `name: String`
- `state: WalletStatus` (syncing { progress: f32 } | ready)
- `tip: Option<Checkpoint>`
- `address_pool_gap: u32` (default 20)
- `delegation: DelegationStatus`
- `passphrase_last_updated: u64` (unix timestamp)
- `discovery: AddressDiscoveryType` (sequential | random)

### UtxoEntry
- `id: UtxoId` (tx_hash[32] ++ out_idx[4])
- `address: Address`
- `value: Value`
- `datum_hash: Option<[u8; 32]>`

### Value
- `coin: Coin` (u64 lovelace)
- `assets: BTreeMap<PolicyId, BTreeMap<AssetName, u64>>`

### TxRecord
- `id: TxHash`
- `slot: u64`
- `block_height: Option<u64>`
- `status: TxStatus` (pending | in_ledger | expired)
- `direction: TxDirection` (incoming | outgoing)
- `inputs: Vec<TxInput>`
- `outputs: Vec<TxOutput>`
- `fee: Option<Coin>`
- `metadata: Option<serde_json::Value>`
- `withdrawals: Vec<Withdrawal>`
- `certificates: Vec<Certificate>`
- `valid_contract: bool`

### Address
- `address: String` (Bech32 or base58)
- `state: AddressState` (used | unused)
- `derivation_path: DerivationPath`

### DerivationPath
- `account_ix: u32`
- `role: AddressRole` (ExternalChain=0 | InternalChain=1 | Staking=2)
- `address_ix: u32`

### Checkpoint
- `slot: u64`
- `block_hash: [u8; 32]`

### PoolMeta
- `id: PoolId` (28 bytes)
- `ticker: Option<String>`
- `name: Option<String>`
- `description: Option<String>`
- `homepage: Option<String>`
- `cost: Coin`
- `margin: Rational`
- `pledge: Coin`
- `saturation: f64`
- `relative_stake: f64`
- `produced_blocks: Option<u64>`

## Sled Tree Layout

| Tree | Key encoding | Value encoding |
|---|---|---|
| `wallets` | wallet_id[32] | WalletMeta (CBOR) |
| `utxos:{wallet_id_hex}` | tx_hash[32] ++ out_idx[4 BE] | UtxoEntry (CBOR) |
| `txs:{wallet_id_hex}` | slot[8 BE] ++ tx_hash[32] | TxRecord (CBOR) |
| `addrs:{wallet_id_hex}` | addr_bytes (raw) | AddressState (CBOR) |
| `checkpoints:{wallet_id_hex}` | slot[8 BE] | block_hash[32] |
| `pending:{wallet_id_hex}` | tx_hash[32] | PendingTx (CBOR) |
| `pools` | pool_id[28] | PoolMeta (CBOR) |
| `idx:addr_wallet` | addr_bytes | wallet_id[32] |

## State Transitions

### Wallet
```
(none) --[create]--> Syncing --[sync complete]--> Ready
Ready  --[delete]--> (none)
```

### Transaction
```
(none) --[submit]--> Pending --[confirmed on chain]--> InLedger
Pending --[rolled back + slot expired]--> Expired
```

### Address
```
Unused --[seen in block]--> Used
```
