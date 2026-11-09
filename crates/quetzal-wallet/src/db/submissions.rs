//! Forgetting affects the local submission pool. Rediscovered chain history
//! does not turn a previously forgotten transaction into a local submission.
use super::*;
use sled::transaction::{
    ConflictableTransactionError as Conflict, TransactionError, Transactional,
};

enum ForgetError {
    Retry,
    Wallet(WalletError),
}

impl WalletDb {
    pub(crate) fn forget_submission(
        &self,
        wallet: &WalletId,
        id: &TxHash,
    ) -> Result<(), WalletError> {
        let wallets = self.wallets_tree()?;
        let pending = self.pending_tree(wallet)?;
        let submissions = self.db.open_tree(format!("submissions:{wallet}"))?;
        let history = self.txs_tree(wallet)?;
        loop {
            let expected_wallet = wallets
                .get(wallet.0.as_slice())?
                .ok_or_else(|| WalletError::NotFound(wallet.to_hex()))?;
            let expected_pending = pending.get(id.0.as_slice())?;
            let record = self.get_tx(wallet, id)?;
            let key = record
                .as_ref()
                .map(|record| tx_history_key(record.slot, id));
            let result = (&wallets, &pending, &submissions, &history).transaction(
                |(wallets, pending, submissions, history)| {
                    let fail = |error| Conflict::Abort(ForgetError::Wallet(error));
                    if wallets.get(wallet.0.as_slice())?.as_ref() != Some(&expected_wallet)
                        || pending.get(id.0.as_slice())? != expected_pending
                    {
                        return Err(Conflict::Abort(ForgetError::Retry));
                    }
                    if submissions.get(id.0.as_slice())?.is_none() && expected_pending.is_none() {
                        return Err(fail(WalletError::TransactionNotFound(id.to_hex())));
                    }
                    if let Some(key) = &key {
                        let bytes = history
                            .get(&key[..])?
                            .ok_or(Conflict::Abort(ForgetError::Retry))?;
                        let stored: TxRecord =
                            cbor_decode(&bytes).map_err(|e| fail(WalletError::from(e)))?;
                        if stored.status == quetzal_primitives::TxStatus::InLedger {
                            return Err(fail(WalletError::TransactionInLedger(id.to_hex())));
                        }
                        history.remove(&key[..])?;
                    } else if expected_pending.is_none() {
                        return Err(fail(WalletError::TransactionNotFound(id.to_hex())));
                    }
                    pending.remove(id.0.as_slice())?;
                    submissions.remove(id.0.as_slice())?;
                    Ok(())
                },
            );
            match result {
                Ok(()) => return self.flush(),
                Err(TransactionError::Abort(ForgetError::Retry)) => continue,
                Err(TransactionError::Abort(ForgetError::Wallet(error))) => return Err(error),
                Err(TransactionError::Storage(error)) => return Err(WalletError::Db(error)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pallas_txbuilder::{BuildConway, StagingTransaction};
    #[tokio::test]
    async fn confirmed_and_forgotten_rediscovered_submissions_retain_distinct_durable_status() {
        let path =
            std::env::temp_dir().join(format!("quetzal-submissions-{}", rand::random::<u64>()));
        let service = crate::WalletService::new(path.to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "submissions".into(),
                &[27; 20],
                b"public-submission-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let addresses = service.get_addresses(&wallet.id).await.unwrap();
        let make_pending = |ttl| {
            let cbor = StagingTransaction::new()
                .network_id(0)
                .fee(0)
                .invalid_from_slot(ttl)
                .build_conway_raw()
                .unwrap()
                .tx_bytes
                .0;
            PendingTx {
                id: TxHash(crate::txsize::body_hash(&cbor).unwrap()),
                cbor,
                submitted_at: "public-test-time".into(),
                expiry_slot: Some(ttl),
                submitted_slot: 100,
                submitted_height: 50,
                latest_submission_slot: None,
            }
        };
        let fresh = make_pending(1000);
        service
            .db
            .insert_pending_and_change(&wallet.id, &fresh, &[])
            .unwrap();
        service.db.remove_pending(&wallet.id, &fresh.id).unwrap();
        let fresh_record = crate::pending::record(
            &fresh,
            &addresses,
            &[],
            quetzal_primitives::TxStatus::InLedger,
        )
        .unwrap();
        service.db.insert_tx(&wallet.id, &fresh_record).unwrap();
        assert!(matches!(
            service.delete_pending(&wallet.id, &fresh.id).await,
            Err(WalletError::TransactionInLedger(_))
        ));
        let forgotten = make_pending(1001);
        service
            .db
            .insert_pending_and_change(&wallet.id, &forgotten, &[])
            .unwrap();
        service
            .delete_pending(&wallet.id, &forgotten.id)
            .await
            .unwrap();
        let rediscovered = crate::pending::record(
            &forgotten,
            &addresses,
            &[],
            quetzal_primitives::TxStatus::InLedger,
        )
        .unwrap();
        service.db.insert_tx(&wallet.id, &rediscovered).unwrap();
        assert!(matches!(
            service.delete_pending(&wallet.id, &forgotten.id).await,
            Err(WalletError::TransactionNotFound(_))
        ));
        assert_eq!(
            cbor_encode(
                &service
                    .db
                    .get_tx(&wallet.id, &forgotten.id)
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            cbor_encode(&rediscovered).unwrap()
        );
        service.db.flush().unwrap();
        drop(service);
        let reopened = crate::WalletService::new(path.to_str().unwrap()).unwrap();
        assert!(matches!(
            reopened.delete_pending(&wallet.id, &fresh.id).await,
            Err(WalletError::TransactionInLedger(_))
        ));
        assert!(matches!(
            reopened.delete_pending(&wallet.id, &forgotten.id).await,
            Err(WalletError::TransactionNotFound(_))
        ));
        drop(reopened);
        std::fs::remove_dir_all(path).unwrap();
    }
}
