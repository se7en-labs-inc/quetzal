//! Validated import boundary for offline migrations into a fresh sled database.
use super::*;
use quetzal_crypto::{Bip32PublicKey, EncryptedKey};
use quetzal_primitives::{AddressRole, WalletType};
use sled::transaction::{
    ConflictableTransactionError as Conflict, TransactionError, Transactional,
};
use std::collections::HashSet;

pub struct ImportedWallet {
    pub metadata: WalletMeta,
    pub account_public_key: [u8; 64],
    pub policy_public_key: Option<[u8; 64]>,
    pub credentials: Option<EncryptedKey>,
    pub payment_script: Option<serde_json::Value>,
    pub delegation_script: Option<serde_json::Value>,
    pub addresses: Vec<AddressEntry>,
    pub allocated_change: Vec<Address>,
    pub checkpoint: Checkpoint,
    pub utxos: Vec<UtxoEntry>,
    pub history: Vec<TxRecord>,
    pub pending: Vec<PendingTx>,
    /// Original local signed bytes, including still-retained confirmed/expired entries.
    pub submissions: Vec<PendingTx>,
    pub staking: crate::staking::StakeState,
}
fn invalid(message: &str) -> WalletError {
    WalletError::TxBuild(format!("invalid imported wallet: {message}"))
}
impl WalletDb {
    pub fn import_wallet(&self, input: &ImportedWallet) -> Result<(), WalletError> {
        let m = &input.metadata;
        let id = &m.id;
        Bip32PublicKey::from_raw_bytes(&input.account_public_key)?;
        if let Some(public) = input.policy_public_key {
            Bip32PublicKey::from_raw_bytes(&public)?;
        }
        if m.tip_slot != input.checkpoint.slot || m.tip_height != input.checkpoint.block_height {
            return Err(invalid("checkpoint and metadata differ"));
        }
        if m.wallet_type == WalletType::Shared && input.payment_script.is_none() {
            return Err(invalid("shared template missing"));
        }
        let mut addresses = HashSet::new();
        for entry in &input.addresses {
            entry
                .address
                .as_str()
                .parse::<pallas_addresses::Address>()
                .map_err(|_| invalid("malformed address"))?;
            if !addresses.insert(entry.address.as_str()) {
                return Err(invalid("duplicate address"));
            }
        }
        for address in &input.allocated_change {
            if !input
                .addresses
                .iter()
                .any(|e| e.address == *address && e.path.role == AddressRole::Internal)
            {
                return Err(invalid("unowned change allocation"));
            }
        }
        let mut total = Coin::ZERO;
        let mut ids = HashSet::new();
        for entry in &input.utxos {
            if !addresses.contains(entry.address.as_str()) {
                return Err(invalid("UTxO address is absent from imported discovery"));
            }
            if !ids.insert(entry.id.clone()) {
                return Err(invalid("duplicate UTxO"));
            }
            total = total
                .checked_add(entry.value.coin)
                .ok_or_else(|| invalid("balance overflow"))?;
        }
        if total != m.balance_total {
            return Err(invalid("balance does not match UTxO"));
        }
        let mut records = HashSet::new();
        for record in &input.history {
            if !records.insert(record.id.clone()) {
                return Err(invalid("duplicate history transaction"));
            }
        }
        let mut submitted = HashSet::new();
        for tx in &input.submissions {
            let hash = crate::txsize::body_hash(&tx.cbor)
                .map_err(|_| invalid("invalid signed transaction"))?;
            if hash != tx.id.0 || !submitted.insert(tx.id.clone()) {
                return Err(invalid("submission hash mismatch or duplicate"));
            }
        }
        let mut pending = HashSet::new();
        for tx in &input.pending {
            use pallas_primitives::{conway, Fragment};
            conway::Tx::decode_fragment(&tx.cbor)
                .map_err(|_| invalid("pending transaction cannot be read by the wallet"))?;
            if !submitted.contains(&tx.id) || !pending.insert(tx.id.clone()) {
                return Err(invalid(
                    "pending transaction lacks unique submission provenance",
                ));
            }
        }
        let trees = vec![
            self.wallets_tree()?,
            self.keys_tree(id)?,
            self.addrs_tree(id)?,
            self.addr_wid_index_tree()?,
            self.utxos_tree(id)?,
            self.txs_tree(id)?,
            self.pending_tree(id)?,
            self.db.open_tree(format!("submissions:{id}"))?,
            self.checkpoints_tree(id)?,
            self.db.open_tree(format!("stake:{id}"))?,
            self.db.open_tree(format!("change:{id}"))?,
            self.db.open_tree(format!("migration:{id}"))?,
        ];
        let mut rows: Vec<(usize, Vec<u8>, Vec<u8>)> = vec![
            (0, id.0.to_vec(), cbor_encode(m)?),
            (1, b"acct_xpub".to_vec(), input.account_public_key.to_vec()),
            (
                8,
                slot_key(input.checkpoint.slot).to_vec(),
                cbor_encode(&input.checkpoint)?,
            ),
            (9, b"state".to_vec(), cbor_encode(&input.staking)?),
            (11, b"floor".to_vec(), cbor_encode(&input.checkpoint)?),
        ];
        if let Some(key) = &input.credentials {
            rows.push((1, b"root".to_vec(), cbor_encode(key)?));
        }
        if let Some(key) = &input.policy_public_key {
            rows.push((1, b"policy_xpub".to_vec(), key.to_vec()));
        }
        for (name, script) in [
            (b"payment_script".as_slice(), &input.payment_script),
            (b"delegation_script".as_slice(), &input.delegation_script),
        ] {
            if let Some(script) = script {
                rows.push((
                    1,
                    name.to_vec(),
                    serde_json::to_vec(script)
                        .map_err(|_| invalid("script serialization failed"))?,
                ));
            }
        }
        for entry in &input.addresses {
            let address = entry.address.as_str().as_bytes().to_vec();
            rows.push((2, address.clone(), cbor_encode(entry)?));
            rows.push((3, address, id.0.to_vec()));
        }
        for address in &input.allocated_change {
            rows.push((10, address.as_str().as_bytes().to_vec(), vec![]));
        }
        for entry in &input.utxos {
            rows.push((
                4,
                utxo_key(&entry.id.tx_hash, entry.id.output_index).to_vec(),
                cbor_encode(entry)?,
            ));
        }
        for record in &input.history {
            rows.push((
                5,
                tx_history_key(record.slot, &record.id).to_vec(),
                cbor_encode(record)?,
            ));
        }
        for tx in &input.pending {
            rows.push((6, tx.id.0.to_vec(), cbor_encode(tx)?));
        }
        for tx in &input.submissions {
            rows.push((7, tx.id.0.to_vec(), vec![]));
            let mut key = b"submission:".to_vec();
            key.extend(tx.id.0);
            rows.push((11, key, cbor_encode(tx)?));
        }
        trees
            .as_slice()
            .transaction(|tx| {
                if tx[0].get(id.0.as_slice())?.is_some() {
                    return Err(Conflict::Abort(WalletError::AlreadyExists(id.to_hex())));
                }
                for (index, key, value) in &rows {
                    tx[*index].insert(key.as_slice(), value.as_slice())?;
                }
                Ok(())
            })
            .map_err(|e| match e {
                TransactionError::Abort(e) => e,
                TransactionError::Storage(e) => WalletError::Db(e),
            })?;
        self.flush()
    }
}

impl WalletDb {
    /// An imported checkpoint has no Quetzal undo journal before its floor.
    /// Atomically discard derived ledger state, retaining identity, credentials,
    /// discovery allocations and signed local submission provenance for replay.
    pub(crate) fn reset_imported_chain(&self, id: &WalletId) -> Result<(), WalletError> {
        let trees = self.chain_trees_for_import(id)?;
        let mut remove = Vec::new();
        for index in [0usize, 1, 3, 6, 7] {
            for entry in trees[index].iter() {
                let (key, _) = entry?;
                remove.push((index, key));
            }
        }
        let mut addresses = Vec::new();
        for mut entry in self.get_addresses(id)? {
            entry.state = AddressState::Unused;
            addresses.push((
                entry.address.as_str().as_bytes().to_vec(),
                cbor_encode(&entry)?,
            ));
        }
        let migration = &trees[8];
        let provenance = self.db.open_tree(format!("submissions:{id}"))?;
        let mut pending = Vec::new();
        for entry in migration.scan_prefix(b"submission:") {
            let (_, bytes) = entry?;
            let tx: PendingTx = cbor_decode(&bytes)?;
            if provenance.contains_key(tx.id.0.as_slice())? {
                pending.push((tx.id.0.to_vec(), cbor_encode(&tx)?));
            }
        }
        trees
            .as_slice()
            .transaction(|tx| {
                for (index, key) in &remove {
                    tx[*index].remove(key.as_ref())?;
                }
                for (key, value) in &addresses {
                    tx[2].insert(key.as_slice(), value.as_slice())?;
                }
                for (key, value) in &pending {
                    tx[4].insert(key.as_slice(), value.as_slice())?;
                }
                let raw = tx[5]
                    .get(id.0.as_slice())?
                    .ok_or_else(|| Conflict::Abort(WalletError::NotFound(id.to_hex())))?;
                let mut m: WalletMeta =
                    cbor_decode(&raw).map_err(|e| Conflict::Abort(WalletError::from(e)))?;
                m.tip_slot = 0;
                m.tip_height = 0;
                m.status = WalletStatus::Syncing;
                m.balance_total = Coin::ZERO;
                m.balance_available = Coin::ZERO;
                m.balance_reward = Coin::ZERO;
                m.reward_account_balance = Coin::ZERO;
                m.assets_total.clear();
                m.assets_available.clear();
                m.delegation_status = Default::default();
                m.stake_pool_id = None;
                tx[5].insert(
                    id.0.as_slice(),
                    cbor_encode(&m).map_err(|e| Conflict::Abort(WalletError::from(e)))?,
                )?;
                tx[8].remove(b"floor".as_slice())?;
                Ok(())
            })
            .map_err(|e| match e {
                TransactionError::Abort(e) => e,
                TransactionError::Storage(e) => WalletError::Db(e),
            })?;
        self.flush()
    }
    fn chain_trees_for_import(&self, id: &WalletId) -> Result<Vec<Tree>, WalletError> {
        Ok(vec![
            self.utxos_tree(id)?,
            self.txs_tree(id)?,
            self.addrs_tree(id)?,
            self.checkpoints_tree(id)?,
            self.pending_tree(id)?,
            self.wallets_tree()?,
            self.db.open_tree(format!("undo:{id}"))?,
            self.db.open_tree(format!("stake:{id}"))?,
            self.db.open_tree(format!("migration:{id}"))?,
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WalletService;
    use pallas_txbuilder::{BuildConway, StagingTransaction};
    use quetzal_primitives::{
        tx::{BlockHeight, CoinBundle, TxBlock, TxOutput, TxTime},
        TxDirection, TxStatus, Value,
    };
    fn snapshot(source: &WalletDb, id: &WalletId) -> ImportedWallet {
        let addresses = source.get_addresses(id).unwrap();
        let address = addresses[0].address.clone();
        let checkpoint = Checkpoint {
            slot: 10,
            block_hash: [6; 32],
            block_height: 1,
        };
        let mut metadata = source.get_wallet(id).unwrap().unwrap();
        metadata.tip_slot = 10;
        metadata.tip_height = 1;
        metadata.balance_total = Coin(5_000_000);
        metadata.balance_available = metadata.balance_total;
        let cbor = StagingTransaction::new()
            .network_id(0)
            .fee(0)
            .invalid_from_slot(100)
            .build_conway_raw()
            .unwrap()
            .tx_bytes
            .0;
        let pending = PendingTx {
            id: TxHash(crate::txsize::body_hash(&cbor).unwrap()),
            cbor,
            submitted_at: "2026-11-10T00:00:00Z".into(),
            expiry_slot: Some(1000),
            submitted_slot: 10,
            submitted_height: 1,
            latest_submission_slot: Some(10),
        };
        ImportedWallet {
            metadata,
            account_public_key: source.get_account_xpub(id).unwrap().unwrap(),
            policy_public_key: None,
            credentials: source.load_encrypted_key(id).unwrap(),
            payment_script: None,
            delegation_script: None,
            addresses,
            allocated_change: vec![],
            checkpoint,
            utxos: vec![UtxoEntry {
                id: UtxoId::new(TxHash([2; 32]), 0),
                address: address.clone(),
                value: Value::lovelace(Coin(5_000_000)),
            }],
            history: vec![TxRecord {
                ledger_details: Default::default(),
                id: TxHash([2; 32]),
                slot: 10,
                block_hash: [6; 32],
                block_height: 1,
                direction: TxDirection::Incoming,
                amount: Coin(5_000_000),
                fee: Some(Coin::ZERO),
                inputs: vec![],
                outputs: vec![TxOutput {
                    address,
                    amount: CoinBundle::lovelace(Coin(5_000_000)),
                    assets: vec![],
                }],
                status: TxStatus::InLedger,
                inserted_at: TxTime {
                    time: "2026-11-10T00:00:00Z".into(),
                    block: TxBlock {
                        slot_no: 10,
                        epoch_no: 0,
                        height: BlockHeight::new(1),
                    },
                },
                metadata: None,
                withdrawals: vec![],
                expires_at_slot: None,
                certificates: vec![],
                deposit_taken: Coin::ZERO,
                deposit_returned: Coin::ZERO,
                stake_derivation_path: None,
            }],
            pending: vec![pending.clone()],
            submissions: vec![pending],
            staking: Default::default(),
        }
    }
    #[tokio::test]
    async fn import_reopens_with_keys_pending_and_identity_then_safely_resets_deep_rollback() {
        let root = std::env::temp_dir().join(format!("quetzal-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = WalletService::new(root.join("source").to_str().unwrap()).unwrap();
        let password = b"public-migration-password";
        let m = source
            .create_wallet(
                "original-name".into(),
                &[42; 20],
                password,
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let input = snapshot(&source.db, &m.id);
        let target = root.join("target");
        let db = WalletDb::open(target.to_str().unwrap()).unwrap();
        db.import_wallet(&input).unwrap();
        assert!(db.import_wallet(&input).is_err());
        drop(db);
        let db = WalletDb::open(target.to_str().unwrap()).unwrap();
        assert_eq!(
            db.get_account_xpub(&m.id).unwrap(),
            Some(input.account_public_key)
        );
        assert_eq!(db.get_utxos(&m.id).unwrap()[0].value.coin, Coin(5_000_000));
        assert_eq!(db.list_pending(&m.id).unwrap()[0].id, input.pending[0].id);
        assert_eq!(
            db.latest_checkpoint(&m.id).unwrap(),
            Some(input.checkpoint.clone())
        );
        let key = db.load_encrypted_key(&m.id).unwrap().unwrap();
        assert_eq!(
            &*key.decrypt(password).unwrap(),
            &*input
                .credentials
                .as_ref()
                .unwrap()
                .decrypt(password)
                .unwrap()
        );
        assert!(key.decrypt(b"wrong-password").is_err());
        let mut updated = db.get_wallet(&m.id).unwrap().unwrap();
        updated.name = "latest-acknowledged-name".into();
        updated.passphrase_last_updated_at = Some("2026-11-10T01:00:00Z".into());
        db.update_wallet(&updated).unwrap();
        assert!(db.rollback_chain(&m.id, 5).is_err());
        assert!(db.latest_checkpoint(&m.id).unwrap().is_none());
        assert!(db.get_utxos(&m.id).unwrap().is_empty());
        assert!(db.get_tx(&m.id, &input.history[0].id).unwrap().is_none());
        assert_eq!(db.list_pending(&m.id).unwrap()[0].id, input.pending[0].id);
        let reset = db.get_wallet(&m.id).unwrap().unwrap();
        assert_eq!(reset.name, updated.name);
        assert_eq!(
            reset.passphrase_last_updated_at,
            updated.passphrase_last_updated_at
        );
        assert_eq!(reset.balance_total, Coin::ZERO);
        assert_eq!(reset.status, WalletStatus::Syncing);
        assert!(db
            .load_encrypted_key(&m.id)
            .unwrap()
            .unwrap()
            .decrypt(password)
            .is_ok());
        drop(db);
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[tokio::test]
    async fn malformed_snapshots_do_not_publish_wallet_and_post_import_undo_restores_baseline() {
        let root = std::env::temp_dir().join(format!("quetzal-migration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let source = WalletService::new(root.join("source").to_str().unwrap()).unwrap();
        let m = source
            .create_wallet(
                "baseline".into(),
                &[43; 20],
                b"public-migration-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let mut input = snapshot(&source.db, &m.id);
        let db = WalletDb::open(root.join("target").to_str().unwrap()).unwrap();
        input.metadata.balance_total = Coin(5_000_001);
        assert!(db.import_wallet(&input).is_err());
        assert!(db.get_wallet(&m.id).unwrap().is_none());
        input.metadata.balance_total = Coin(5_000_000);
        let owned = input.utxos[0].address.clone();
        input.utxos[0].address = Address("unowned-address".into());
        assert!(db
            .import_wallet(&input)
            .unwrap_err()
            .to_string()
            .contains("absent from imported discovery"));
        assert!(db.get_wallet(&m.id).unwrap().is_none());
        input.utxos[0].address = owned;
        input.utxos.push(input.utxos[0].clone());
        assert!(db
            .import_wallet(&input)
            .unwrap_err()
            .to_string()
            .contains("duplicate UTxO"));
        assert!(db.get_wallet(&m.id).unwrap().is_none());
        input.utxos.pop();
        db.import_wallet(&input).unwrap();
        db.commit_chain_block(
            &m.id,
            &Checkpoint {
                slot: 20,
                block_hash: [7; 32],
                block_height: 2,
            },
            true,
            &[(input.utxos[0].id.clone(), None)],
            &[],
            &[],
        )
        .unwrap();
        assert!(db.get_utxos(&m.id).unwrap().is_empty());
        db.rollback_chain(&m.id, 10).unwrap();
        assert_eq!(db.get_utxos(&m.id).unwrap()[0].value.coin, Coin(5_000_000));
        assert_eq!(
            db.latest_checkpoint(&m.id).unwrap(),
            Some(input.checkpoint.clone())
        );
        drop(db);
        drop(source);
        std::fs::remove_dir_all(root).unwrap();
    }
}
