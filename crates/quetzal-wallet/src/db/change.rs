//! Allocated change is part of the address book, independent of forgetting a
//! pending transaction. Commit the allocation and pending bytes together.
use super::*;
use sled::transaction::{
    ConflictableTransactionError as Conflict, TransactionError, Transactional,
};

impl WalletDb {
    pub(crate) fn known_addresses(
        &self,
        wallet: &WalletId,
    ) -> Result<Vec<AddressEntry>, WalletError> {
        let allocated = self.db.open_tree(format!("change:{wallet}"))?;
        let mut known = Vec::new();
        for entry in self.get_addresses(wallet)? {
            if entry.path.role == quetzal_primitives::AddressRole::External
                || entry.state == AddressState::Used
                || allocated.contains_key(entry.address.as_str().as_bytes())?
            {
                known.push(entry);
            }
        }
        Ok(known)
    }

    pub(crate) fn insert_pending_and_change(
        &self,
        wallet: &WalletId,
        pending: &PendingTx,
        change: &[Address],
    ) -> Result<(), WalletError> {
        let wallets = self.wallets_tree()?;
        let addresses = self.addrs_tree(wallet)?;
        let allocated = self.db.open_tree(format!("change:{wallet}"))?;
        let pending_tree = self.pending_tree(wallet)?;
        let submissions = self.db.open_tree(format!("submissions:{wallet}"))?;
        let encoded = cbor_encode(pending)?;
        (
            &wallets,
            &addresses,
            &allocated,
            &pending_tree,
            &submissions,
        )
            .transaction(
                |(wallets, addresses, allocated, pending_tree, submissions)| {
                    if wallets.get(wallet.0.as_slice())?.is_none() {
                        return Err(Conflict::Abort(WalletError::NotFound(wallet.to_hex())));
                    }
                    for address in change {
                        if addresses.get(address.as_str().as_bytes())?.is_none() {
                            return Err(Conflict::Abort(WalletError::TxBuild(
                                "unowned change address".into(),
                            )));
                        }
                        allocated.insert(address.as_str().as_bytes(), &[])?;
                    }
                    pending_tree.insert(pending.id.0.as_slice(), encoded.as_slice())?;
                    submissions.insert(pending.id.0.as_slice(), &[])?;
                    Ok(())
                },
            )
            .map_err(|error| match error {
                TransactionError::Abort(error) => error,
                TransactionError::Storage(error) => WalletError::Db(error),
            })?;
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn allocation_is_atomic_survives_forgetting_and_does_not_expose_the_unused_pool() {
        let path =
            std::env::temp_dir().join(format!("quetzal-known-change-{}", rand::random::<u64>()));
        let service = crate::WalletService::new(path.to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "change-test".into(),
                &[31; 20],
                b"public-test-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let addresses = service.get_addresses(&wallet.id).await.unwrap();
        let internal: Vec<_> = addresses
            .iter()
            .filter(|entry| entry.path.role == quetzal_primitives::AddressRole::Internal)
            .collect();
        assert_eq!(
            service.get_known_addresses(&wallet.id).await.unwrap().len(),
            20
        );
        let pending = PendingTx {
            id: TxHash([23; 32]),
            cbor: vec![0x80],
            submitted_at: "public-test-time".into(),
            expiry_slot: Some(1000),
            submitted_slot: 100,
            submitted_height: 50,
            latest_submission_slot: None,
        };
        assert!(service
            .db
            .insert_pending_and_change(
                &wallet.id,
                &pending,
                &[internal[0].address.clone(), Address("unowned".into())]
            )
            .is_err());
        assert!(service.db.list_pending(&wallet.id).unwrap().is_empty());
        assert_eq!(service.db.known_addresses(&wallet.id).unwrap().len(), 20);
        service
            .db
            .insert_pending_and_change(&wallet.id, &pending, &[internal[0].address.clone()])
            .unwrap();
        assert_eq!(service.db.known_addresses(&wallet.id).unwrap().len(), 21);
        service
            .db
            .forget_submission(&wallet.id, &pending.id)
            .unwrap();
        service.db.flush().unwrap();
        drop(service);
        let reopened = crate::WalletService::new(path.to_str().unwrap()).unwrap();
        assert_eq!(
            reopened
                .get_known_addresses(&wallet.id)
                .await
                .unwrap()
                .len(),
            21
        );
        assert!(reopened.db.list_pending(&wallet.id).unwrap().is_empty());
        reopened.delete_wallet(&wallet.id).await.unwrap();
        let restored = reopened
            .create_wallet(
                "restored".into(),
                &[31; 20],
                b"public-test-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        assert_eq!(
            reopened
                .get_known_addresses(&restored.id)
                .await
                .unwrap()
                .len(),
            20
        );
        drop(reopened);
        std::fs::remove_dir_all(path).unwrap();
    }
}
