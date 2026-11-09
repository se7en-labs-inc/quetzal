use crate::Coin;
use serde::{Deserialize, Serialize};

/// 20-byte wallet identifier (Blake2b-160 of the root extended public key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WalletId(pub [u8; 20]);

/// Delegation state for a wallet.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationStatus {
    #[default]
    NotDelegating,
    Delegating,
    Retiring,
}

impl WalletId {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        let arr: [u8; 20] = bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)?;
        Ok(WalletId(arr))
    }
}

impl std::fmt::Display for WalletId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalletStatus {
    Ready,
    Syncing,
    NotResponding,
}

/// Passphrase encryption scheme — always argon2id-v2 for new wallets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PassphraseScheme {
    Argon2idV2,
    Pbkdf2HmacSha512,
    Scrypt,
}

/// The HD wallet flavour — determines key derivation paths and address format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WalletType {
    #[default]
    Shelley,
    Byron,
    Icarus,
    Shared,
}

/// Wallet metadata as stored in sled `wallets` tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletMeta {
    pub id: WalletId,
    pub name: String,
    pub account_index: u32,
    pub address_pool_gap: u32,
    pub status: WalletStatus,
    pub passphrase_scheme: PassphraseScheme,
    /// ISO-8601 UTC timestamp of last passphrase change.
    pub passphrase_last_updated_at: Option<String>,
    /// Highest confirmed slot we have processed.
    pub tip_slot: u64,
    pub tip_height: u64,
    pub created_at: String,
    pub balance_total: Coin,
    pub balance_available: Coin,
    pub balance_reward: Coin,
    #[serde(default)]
    pub assets_available: Vec<crate::tx::AssetAmount>,
    #[serde(default)]
    pub assets_total: Vec<crate::tx::AssetAmount>,
    /// Delegation state: not_delegating, delegating, or retiring.
    #[serde(default)]
    pub delegation_status: DelegationStatus,
    /// bech32 `pool1…` encoded pool id if currently delegating.
    #[serde(default)]
    pub stake_pool_id: Option<String>,
    /// Confirmed reward account balance in lovelace.
    #[serde(default)]
    pub reward_account_balance: Coin,
    /// Wallet type (Shelley, Byron, or Icarus).
    #[serde(default)]
    pub wallet_type: WalletType,
}
