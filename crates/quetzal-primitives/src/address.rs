use serde::{Deserialize, Serialize};

/// An opaque Cardano address (bech32 or base58 string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Address(pub String);

impl Address {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<String> for Address {
    fn from(s: String) -> Self {
        Address(s)
    }
}

/// Whether the address has been used (received funds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddressState {
    Used,
    Unused,
}

/// Role in the CIP-1852 key hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AddressRole {
    /// External (receive) addresses: role index 0
    External,
    /// Internal (change) addresses: role index 1
    Internal,
    /// Stake keys: role index 2
    Stake,
}

impl AddressRole {
    pub fn index(self) -> u32 {
        match self {
            AddressRole::External => 0,
            AddressRole::Internal => 1,
            AddressRole::Stake => 2,
        }
    }
}

/// CIP-1852 / BIP-44 derivation path indices.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivationPath {
    pub account_ix: u32,
    pub role: AddressRole,
    pub address_ix: u32,
}

impl DerivationPath {
    pub fn shelley_external(account: u32, ix: u32) -> Self {
        DerivationPath {
            account_ix: account,
            role: AddressRole::External,
            address_ix: ix,
        }
    }

    pub fn shelley_internal(account: u32, ix: u32) -> Self {
        DerivationPath {
            account_ix: account,
            role: AddressRole::Internal,
            address_ix: ix,
        }
    }
}

/// Per-address state stored in sled.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressEntry {
    pub address: Address,
    pub path: DerivationPath,
    pub state: AddressState,
}
