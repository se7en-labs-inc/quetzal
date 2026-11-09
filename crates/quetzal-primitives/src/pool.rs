use crate::Coin;
use serde::{Deserialize, Serialize};

/// 28-byte stake pool identifier (Blake2b-224 of the pool verification key).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PoolId(pub [u8; 28]);

impl PoolId {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
        let bytes = hex::decode(s)?;
        let arr: [u8; 28] = bytes
            .try_into()
            .map_err(|_| hex::FromHexError::InvalidStringLength)?;
        Ok(PoolId(arr))
    }

    /// bech32 `pool1…` encoding
    pub fn to_bech32(&self) -> String {
        use bech32::Bech32;
        bech32::encode::<Bech32>(bech32::Hrp::parse_unchecked("pool"), &self.0)
            .unwrap_or_else(|_| self.to_hex())
    }
}

impl std::fmt::Display for PoolId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_bech32())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PoolStatus {
    Registered,
    Retiring,
    Retired,
    NotRegistered,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMeta {
    pub id: PoolId,
    pub name: Option<String>,
    pub ticker: Option<String>,
    pub description: Option<String>,
    pub homepage: Option<String>,
    pub margin: Option<PoolMargin>,
    pub cost: Option<Coin>,
    pub pledge: Option<Coin>,
    pub relative_stake: Option<f64>,
    pub saturation: Option<f64>,
    pub produced_blocks: u64,
    pub status: PoolStatus,
    pub retirement: Option<PoolRetirement>,
    pub flags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolMargin {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolRetirement {
    pub epoch_number: u32,
    pub epoch_start_time: String,
}

/// Last successful fetch of a configured SMASH server's delisted pool set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PoolGcState {
    pub source: String,
    pub last_run: String,
    pub delisted: Vec<PoolId>,
}
