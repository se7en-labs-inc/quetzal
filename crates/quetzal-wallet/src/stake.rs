//! Stake ownership of the wallet's current UTxO, independent of payment ownership.
use crate::WalletError;
use pallas_addresses::{Address, ShelleyDelegationPart};
use quetzal_primitives::UtxoEntry;
use std::{collections::BTreeMap, str::FromStr};

/// Ledger credential discriminator (key=0, script=1) and hash.
pub type Credential = (u8, [u8; 28]);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Distribution {
    pub credentials: BTreeMap<Credential, u64>,
    pub unassigned: u64,
}

pub fn distribution(utxos: &[UtxoEntry]) -> Result<Distribution, WalletError> {
    let mut distribution = Distribution::default();
    for utxo in utxos {
        let address = Address::from_str(utxo.address.as_str())
            .map_err(|e| WalletError::TxBuild(format!("invalid stored UTxO address: {e}")))?;
        let credential = match address {
            Address::Shelley(address) => match address.delegation() {
                ShelleyDelegationPart::Key(hash) => Some((0, **hash)),
                ShelleyDelegationPart::Script(hash) => Some((1, **hash)),
                ShelleyDelegationPart::Pointer(_) | ShelleyDelegationPart::Null => None,
            },
            _ => None,
        };
        let total = match credential {
            Some(key) => distribution.credentials.entry(key).or_default(),
            None => &mut distribution.unassigned,
        };
        *total = total
            .checked_add(utxo.value.coin.0)
            .ok_or_else(|| WalletError::TxBuild("stake coin overflow".into()))?;
    }
    Ok(distribution)
}

#[cfg(test)]
mod tests {
    use super::*;
    use quetzal_primitives::{TxHash, UtxoId, Value};

    fn output(header: u8, stake: Option<[u8; 28]>, coin: u64) -> UtxoEntry {
        let mut bytes = vec![header];
        bytes.extend([7; 28]);
        if let Some(stake) = stake {
            bytes.extend(stake);
        }
        if header >> 4 == 4 {
            bytes.extend([0, 0, 0]);
        }
        UtxoEntry {
            id: UtxoId::new(TxHash([0; 32]), 0),
            address: quetzal_address::codec::bech32_encode("addr_test", &bytes)
                .unwrap()
                .into(),
            value: Value::lovelace(coin.into()),
        }
    }

    #[test]
    fn groups_by_stake_credential_including_script_and_unassigned() {
        let actual = distribution(&[
            output(0x00, Some([1; 28]), 10),
            output(0x10, Some([1; 28]), 20),
            output(0x20, Some([1; 28]), 30),
            output(0x00, Some([2; 28]), 40),
            output(0x60, None, 50),
            output(0x40, None, 60),
        ])
        .unwrap();
        assert_eq!(
            actual.credentials,
            BTreeMap::from([((0, [1; 28]), 30), ((1, [1; 28]), 30), ((0, [2; 28]), 40),])
        );
        assert_eq!(actual.unassigned, 110);
        assert!(distribution(&[output(0x60, None, u64::MAX), output(0x60, None, 1)]).is_err());
    }
}
