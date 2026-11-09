use serde::{Deserialize, Serialize};

/// A point on the chain: slot number + block header hash.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Checkpoint {
    pub slot: u64,
    pub block_hash: [u8; 32],
    pub block_height: u64,
}

impl Checkpoint {
    pub fn origin() -> Self {
        Checkpoint {
            slot: 0,
            block_hash: [0u8; 32],
            block_height: 0,
        }
    }
}
