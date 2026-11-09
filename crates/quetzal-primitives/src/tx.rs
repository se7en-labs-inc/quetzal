use crate::{Address, Coin};
use serde::{Deserialize, Serialize};

/// A 32-byte transaction hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TxHash(pub [u8; 32]);

impl TxHash {
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)?;
        Ok(TxHash(arr))
    }
}

impl std::fmt::Display for TxHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxStatus {
    InLedger,
    Pending,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TxDirection {
    Incoming,
    Outgoing,
}

/// A confirmed transaction record stored in sled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxRecord {
    pub id: TxHash,
    pub slot: u64,
    pub block_hash: [u8; 32],
    pub block_height: u64,
    pub direction: TxDirection,
    pub amount: Coin,
    pub fee: Option<Coin>,
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,
    pub status: TxStatus,
    pub inserted_at: TxTime,
    pub metadata: Option<serde_json::Value>,
    pub withdrawals: Vec<Withdrawal>,
    #[serde(default)]
    pub expires_at_slot: Option<u64>,
    /// Canonical ledger certificates associated with this wallet's stake key.
    #[serde(default)]
    pub certificates: Vec<Vec<u8>>,
    #[serde(default)]
    pub deposit_taken: Coin,
    #[serde(default)]
    pub deposit_returned: Coin,
    #[serde(default)]
    pub stake_derivation_path: Option<crate::DerivationPath>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxInput {
    pub id: TxHash,
    pub index: u32,
    pub address: Option<Address>,
    pub amount: Option<CoinBundle>,
    #[serde(default)]
    pub assets: Vec<AssetAmount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxOutput {
    pub address: Address,
    pub amount: CoinBundle,
    pub assets: Vec<AssetAmount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoinBundle {
    pub quantity: Coin,
    pub unit: String,
}

impl CoinBundle {
    pub fn lovelace(coin: Coin) -> Self {
        CoinBundle {
            quantity: coin,
            unit: "lovelace".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetAmount {
    pub policy_id: String,
    pub asset_name: String,
    pub quantity: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxTime {
    pub time: String,
    pub block: TxBlock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxBlock {
    pub slot_no: u64,
    pub epoch_no: u32,
    pub height: BlockHeight,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockHeight {
    pub quantity: u64,
    pub unit: String,
}

impl BlockHeight {
    pub fn new(n: u64) -> Self {
        BlockHeight {
            quantity: n,
            unit: "block".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Withdrawal {
    pub stake_address: Address,
    pub amount: CoinBundle,
}

/// A pending (submitted but not confirmed) transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingTx {
    pub id: TxHash,
    pub cbor: Vec<u8>,
    pub submitted_at: String,
    pub expiry_slot: Option<u64>,
    #[serde(default)]
    pub submitted_slot: u64,
    #[serde(default)]
    pub submitted_height: u64,
    #[serde(default)]
    pub latest_submission_slot: Option<u64>,
}
