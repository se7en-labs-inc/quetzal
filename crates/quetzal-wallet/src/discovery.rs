//! Sequential address-pool extension while observing a block.
use crate::WalletError;
use pallas_addresses::{Address as LedgerAddress, Network};
use quetzal_crypto::Bip32PublicKey;
use quetzal_primitives::{
    Address, AddressEntry, AddressState, DerivationPath, WalletMeta, WalletType,
};
use std::collections::HashMap;
use std::str::FromStr;

pub(crate) struct DiscoveryPool {
    entries: HashMap<String, AddressEntry>,
    changed: HashMap<String, AddressEntry>,
    public: Bip32PublicKey,
    gap: u32,
    account: u32,
    network: Network,
    magic: Option<u32>,
    style: WalletType,
}
impl DiscoveryPool {
    pub fn new(
        wallet: &WalletMeta,
        entries: Vec<AddressEntry>,
        public: [u8; 64],
    ) -> Result<Option<Self>, WalletError> {
        if !matches!(wallet.wallet_type, WalletType::Shelley | WalletType::Icarus) {
            return Ok(None);
        }
        let Some(first) = entries.first() else {
            return Ok(None);
        };
        let address = LedgerAddress::from_str(first.address.as_str())
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let (network, magic) = if let LedgerAddress::Byron(address) = address {
            let payload = address
                .decode()
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            let mut magic = None;
            for attribute in payload.attributes.iter() {
                if let pallas_addresses::byron::AddrAttrProperty::NetworkTag(raw) = attribute {
                    magic = Some(
                        pallas_codec::minicbor::decode(raw)
                            .map_err(|e| WalletError::TxBuild(e.to_string()))?,
                    );
                }
            }
            (
                if magic.is_none() {
                    Network::Mainnet
                } else {
                    Network::Testnet
                },
                magic,
            )
        } else {
            let tag = address
                .to_vec()
                .first()
                .copied()
                .ok_or_else(|| WalletError::TxBuild("empty address".into()))?
                & 15;
            (
                match tag {
                    0 => Network::Testnet,
                    1 => Network::Mainnet,
                    other => Network::Other(other),
                },
                None,
            )
        };
        Ok(Some(Self {
            entries: entries
                .into_iter()
                .map(|entry| (entry.address.0.clone(), entry))
                .collect(),
            changed: HashMap::new(),
            public: Bip32PublicKey::from_raw_bytes(&public)?,
            gap: wallet.address_pool_gap,
            account: wallet.account_index,
            network,
            magic,
            style: wallet.wallet_type.clone(),
        }))
    }
    pub fn observe(&mut self, address: &Address) -> Result<Vec<AddressEntry>, WalletError> {
        let entry = self
            .entries
            .get_mut(address.as_str())
            .ok_or_else(|| WalletError::TxBuild("unknown discovered address".into()))?;
        entry.state = AddressState::Used;
        let role = entry.path.role;
        self.changed.insert(address.0.clone(), entry.clone());
        let used = self
            .entries
            .values()
            .filter(|entry| entry.path.role == role && entry.state == AddressState::Used)
            .map(|entry| entry.path.address_ix)
            .max()
            .unwrap_or(0);
        let maximum = self
            .entries
            .values()
            .filter(|entry| entry.path.role == role)
            .map(|entry| entry.path.address_ix)
            .max()
            .unwrap_or(0);
        let end = used
            .checked_add(self.gap)
            .and_then(|index| index.checked_add(1))
            .ok_or_else(|| WalletError::TxBuild("address index overflow".into()))?;
        let start = maximum
            .checked_add(1)
            .ok_or_else(|| WalletError::TxBuild("address index overflow".into()))?;
        let mut added = Vec::new();
        for index in start..end {
            let address = if self.style == WalletType::Shelley {
                quetzal_address::shelley::base_address_for_role(
                    &self.public,
                    role.index(),
                    index,
                    0,
                    self.network,
                )?
            } else {
                Address(
                    quetzal_address::byron::icarus_from_account_role(
                        &self.public,
                        role.index(),
                        index,
                        self.magic,
                    )?
                    .to_base58(),
                )
            };
            let entry = AddressEntry {
                address,
                path: DerivationPath {
                    account_ix: self.account,
                    role,
                    address_ix: index,
                },
                state: AddressState::Unused,
            };
            self.entries.insert(entry.address.0.clone(), entry.clone());
            self.changed.insert(entry.address.0.clone(), entry.clone());
            added.push(entry);
        }
        Ok(added)
    }
    pub fn changes(&self) -> Vec<AddressEntry> {
        self.changed.values().cloned().collect()
    }
}
