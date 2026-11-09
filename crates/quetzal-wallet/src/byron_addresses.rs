//! Generation and authenticated, all-or-nothing import of Byron random addresses.
use crate::{WalletError, WalletService};
use pallas_addresses::byron::AddrAttrProperty;
use quetzal_primitives::{AddressEntry, AddressState, DerivationPath, WalletId, WalletType};

impl WalletService {
    pub async fn create_random_address(
        &self,
        wallet: &WalletId,
        password: &[u8],
        requested: Option<u32>,
        magic: Option<u32>,
    ) -> Result<AddressEntry, WalletError> {
        if self.get_byron_wallet(wallet).await?.wallet_type != WalletType::Byron {
            return Err(WalletError::InvalidRandomWalletType);
        }
        let root = self.decrypt_root(wallet, password).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root)?;
        let _guard = self.random_addresses.lock().await;
        let existing = self.db.get_addresses(wallet)?;
        let index = match requested {
            Some(index) if index < 0x80000000 => {
                return Err(WalletError::TxBuild(
                    "random address index must be hardened".into(),
                ))
            }
            Some(index) => index,
            None => loop {
                let index = rand::random::<u32>() | 0x80000000;
                if !existing.iter().any(|entry| entry.path.address_ix == index) {
                    break index;
                }
            },
        };
        if existing
            .iter()
            .any(|entry| entry.path.account_ix == 0x80000000 && entry.path.address_ix == index)
        {
            return Err(WalletError::AddressAlreadyExists(index));
        }
        let address = quetzal_address::byron::address(&root, true, 0x80000000, index, magic)?;
        let entry = AddressEntry {
            address: address.to_base58().into(),
            path: DerivationPath::shelley_external(0x80000000, index),
            state: AddressState::Unused,
        };
        self.db
            .import_addresses(wallet, std::slice::from_ref(&entry))?;
        Ok(entry)
    }

    pub async fn import_random_addresses(
        &self,
        wallet: &WalletId,
        addresses: &[String],
        magic: Option<u32>,
    ) -> Result<(), WalletError> {
        if self.get_byron_wallet(wallet).await?.wallet_type != WalletType::Byron {
            return Err(WalletError::InvalidRandomWalletType);
        }
        let public = self.get_account_xpub_bytes(wallet).await?;
        let payload_key = quetzal_crypto::byron::payload_key(&public)?;
        let mut entries = Vec::new();
        for encoded in addresses {
            let raw = quetzal_address::codec::base58_decode(encoded)
                .map_err(|_| WalletError::AddressNotOurs)?;
            quetzal_address::byron::inspect(&raw, true).map_err(|_| WalletError::AddressNotOurs)?;
            let address = pallas_addresses::byron::ByronAddress::from_bytes(&raw)
                .map_err(|_| WalletError::AddressNotOurs)?;
            let payload = address.decode().map_err(|_| WalletError::AddressNotOurs)?;
            let mut path = None;
            let mut observed_magic = None;
            for attribute in payload.attributes.iter() {
                match attribute {
                    AddrAttrProperty::DerivationPath(encrypted) => {
                        path = quetzal_crypto::byron::decrypt_path(&payload_key, encrypted)
                            .map_err(|_| WalletError::AddressNotOurs)?;
                    }
                    AddrAttrProperty::NetworkTag(bytes) => {
                        observed_magic = Some(
                            pallas_codec::minicbor::decode::<u32>(bytes)
                                .map_err(|_| WalletError::AddressNotOurs)?,
                        )
                    }
                    _ => {}
                }
            }
            let [account, index] = path
                .filter(|_| observed_magic == magic)
                .ok_or(WalletError::AddressNotOurs)?;
            entries.push(AddressEntry {
                address: encoded.clone().into(),
                path: DerivationPath::shelley_external(account, index),
                state: AddressState::Unused,
            });
        }
        let _guard = self.random_addresses.lock().await;
        self.db.import_addresses(wallet, &entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn random_address_creation_and_import_are_atomic_and_durable() {
        let path =
            std::env::temp_dir().join(format!("quetzal-random-address-{}", rand::random::<u64>()));
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        let wallet = service
            .create_byron_wallet_style(
                "random".into(),
                &[47; 16],
                b"public-test-password",
                Some(42),
                "random",
            )
            .await
            .unwrap();
        assert!(service
            .create_random_address(&wallet.id, b"wrong password", Some(0x80003039), Some(42))
            .await
            .is_err());
        assert!(service.get_addresses(&wallet.id).await.unwrap().is_empty());
        let (first, second) = tokio::join!(
            service.create_random_address(
                &wallet.id,
                b"public-test-password",
                Some(0x80003039),
                Some(42)
            ),
            service.create_random_address(
                &wallet.id,
                b"public-test-password",
                Some(0x80003039),
                Some(42)
            ),
        );
        assert!(matches!(
            (&first, &second),
            (Ok(_), Err(WalletError::AddressAlreadyExists(_)))
                | (Err(WalletError::AddressAlreadyExists(_)), Ok(_))
        ));
        let owned = first.or(second).unwrap();
        service
            .db
            .mark_address_used(&wallet.id, &owned.address)
            .unwrap();
        service
            .import_random_addresses(&wallet.id, std::slice::from_ref(&owned.address.0), Some(42))
            .await
            .unwrap();
        assert_eq!(
            service.get_addresses(&wallet.id).await.unwrap()[0].state,
            AddressState::Used
        );
        // Validation of the whole batch precedes the transaction: an owned but
        // previously unknown address must not leak through a rejected batch.
        let root = service
            .decrypt_root(&wallet.id, b"public-test-password")
            .await
            .unwrap();
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root).unwrap();
        let next = quetzal_address::byron::address(&root, true, 0x80000000, 0x80003040, Some(42))
            .unwrap()
            .to_base58();
        assert!(service
            .import_random_addresses(
                &wallet.id,
                &[next.clone(), "not-an-address".into()],
                Some(42)
            )
            .await
            .is_err());
        assert_eq!(service.get_addresses(&wallet.id).await.unwrap().len(), 1);
        assert!(service
            .import_random_addresses(&wallet.id, std::slice::from_ref(&next), Some(43))
            .await
            .is_err());
        service
            .import_random_addresses(&wallet.id, &[next], Some(42))
            .await
            .unwrap();
        assert_eq!(service.get_addresses(&wallet.id).await.unwrap().len(), 2);
        drop(service);
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        assert_eq!(service.get_addresses(&wallet.id).await.unwrap().len(), 2);
        service.delete_wallet(&wallet.id).await.unwrap();
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
