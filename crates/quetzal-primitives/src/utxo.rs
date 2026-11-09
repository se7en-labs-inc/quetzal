use crate::{Address, Coin, TxHash};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Identifies a specific output within a transaction.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UtxoId {
    pub tx_hash: TxHash,
    pub output_index: u32,
}

impl UtxoId {
    pub fn new(tx_hash: TxHash, output_index: u32) -> Self {
        UtxoId {
            tx_hash,
            output_index,
        }
    }
}

/// Multi-asset value: ADA lovelace + native tokens.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Value {
    pub coin: Coin,
    /// policy_id (hex) → asset_name (hex) → quantity
    pub assets: BTreeMap<String, BTreeMap<String, u64>>,
}

impl Value {
    pub fn checked_add(&self, other: &Self) -> Option<Self> {
        let mut result = self.clone();
        result.coin = result.coin.checked_add(other.coin)?;
        for (policy, names) in &other.assets {
            for (name, quantity) in names {
                if *quantity == 0 {
                    continue;
                }
                let current = result
                    .assets
                    .entry(policy.clone())
                    .or_default()
                    .entry(name.clone())
                    .or_default();
                *current = current.checked_add(*quantity)?;
            }
        }
        Some(result)
    }
    pub fn lovelace(coin: Coin) -> Self {
        Value {
            coin,
            assets: BTreeMap::new(),
        }
    }

    pub fn is_pure_ada(&self) -> bool {
        self.assets.is_empty()
    }
}

impl Default for Value {
    fn default() -> Self {
        Value::lovelace(Coin::ZERO)
    }
}

/// A UTxO entry as stored in sled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UtxoEntry {
    pub id: UtxoId,
    pub address: Address,
    pub value: Value,
}
