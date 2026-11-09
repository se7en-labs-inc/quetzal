//! Atomic wallet block updates and reversible ledger changes.
use super::*;
use sled::transaction::{
    ConflictableTransactionError as Conflict, TransactionError, Transactional,
};

type Change = (usize, Vec<u8>, Option<Vec<u8>>);
pub(crate) struct BlockChanges<'a> {
    pub utxos: &'a [(UtxoId, Option<UtxoEntry>)],
    pub records: &'a [TxRecord],
    pub used: &'a [Address],
    pub addresses: &'a [AddressEntry],
}

fn transaction_error(error: TransactionError<WalletError>) -> WalletError {
    match error {
        TransactionError::Abort(error) => error,
        TransactionError::Storage(error) => WalletError::Db(error),
    }
}

impl WalletDb {
    fn chain_trees(&self, wid: &WalletId) -> Result<Vec<Tree>, WalletError> {
        Ok(vec![
            self.utxos_tree(wid)?,
            self.txs_tree(wid)?,
            self.addrs_tree(wid)?,
            self.checkpoints_tree(wid)?,
            self.pending_tree(wid)?,
            self.wallets_tree()?,
            self.db.open_tree(format!("undo:{wid}"))?,
            self.db.open_tree(format!("stake:{wid}"))?,
        ])
    }

    pub fn commit_chain_block(
        &self,
        wid: &WalletId,
        checkpoint: &Checkpoint,
        synced: bool,
        utxos: &[(UtxoId, Option<UtxoEntry>)],
        records: &[TxRecord],
        used: &[Address],
    ) -> Result<(), WalletError> {
        self.commit_chain_block_with_addresses(
            wid,
            checkpoint,
            synced,
            BlockChanges {
                utxos,
                records,
                used,
                addresses: &[],
            },
        )
    }

    pub(crate) fn commit_chain_block_with_addresses(
        &self,
        wid: &WalletId,
        checkpoint: &Checkpoint,
        synced: bool,
        update: BlockChanges<'_>,
    ) -> Result<(), WalletError> {
        let BlockChanges {
            utxos,
            records,
            used,
            addresses,
        } = update;
        let trees = self.chain_trees(wid)?;
        let mut changes: Vec<Change> = Vec::new();
        for entry in addresses {
            changes.push((
                2,
                entry.address.as_str().as_bytes().to_vec(),
                Some(cbor_encode(entry)?),
            ));
        }
        for (id, entry) in utxos {
            changes.push((
                0,
                utxo_key(&id.tx_hash, id.output_index).to_vec(),
                entry.as_ref().map(cbor_encode).transpose()?,
            ));
        }
        for record in records {
            changes.push((
                1,
                tx_history_key(record.slot, &record.id).to_vec(),
                Some(cbor_encode(record)?),
            ));
            changes.push((4, record.id.0.to_vec(), None));
        }
        changes.push((
            3,
            slot_key(checkpoint.slot).to_vec(),
            Some(cbor_encode(checkpoint)?),
        ));
        let journal_key = slot_key(checkpoint.slot).to_vec();
        trees
            .as_slice()
            .transaction(|trees| {
                if trees[6].get(&journal_key)?.is_some() {
                    return Ok(());
                }
                let mut mutations = changes.clone();
                let confirmed: Vec<_> = records
                    .iter()
                    .filter(|record| {
                        record.status == quetzal_primitives::TxStatus::InLedger
                            && !record.certificates.is_empty()
                    })
                    .collect();
                if !confirmed.is_empty() {
                    let mut state: crate::staking::StakeState = trees[7]
                        .get(b"state")?
                        .map(|bytes| cbor_decode(&bytes))
                        .transpose()
                        .map_err(|e| Conflict::Abort(WalletError::from(e)))?
                        .unwrap_or_default();
                    for record in confirmed {
                        state.apply(record).map_err(Conflict::Abort)?;
                    }
                    mutations.push((
                        7,
                        b"state".to_vec(),
                        Some(
                            cbor_encode(&state)
                                .map_err(|e| Conflict::Abort(WalletError::from(e)))?,
                        ),
                    ));
                }
                for address in used {
                    let key = address.as_str().as_bytes().to_vec();
                    if let Some(bytes) = trees[2].get(&key)? {
                        let mut entry: AddressEntry = cbor_decode(&bytes)
                            .map_err(|error| Conflict::Abort(WalletError::from(error)))?;
                        entry.state = AddressState::Used;
                        mutations.push((
                            2,
                            key,
                            Some(
                                cbor_encode(&entry)
                                    .map_err(|error| Conflict::Abort(WalletError::from(error)))?,
                            ),
                        ));
                    }
                }
                let mut undo: Vec<Change> = Vec::new();
                for (index, key, value) in &mutations {
                    undo.push((
                        *index,
                        key.clone(),
                        trees[*index].get(key)?.map(|bytes| bytes.to_vec()),
                    ));
                    match value {
                        Some(value) => {
                            trees[*index].insert(key.as_slice(), value.as_slice())?;
                        }
                        None => {
                            trees[*index].remove(key.as_slice())?;
                        }
                    }
                }
                let bytes = trees[5]
                    .get(wid.0.as_slice())?
                    .ok_or_else(|| Conflict::Abort(WalletError::NotFound(wid.to_hex())))?;
                let mut meta: WalletMeta = cbor_decode(&bytes)
                    .map_err(|error| Conflict::Abort(WalletError::from(error)))?;
                meta.tip_slot = checkpoint.slot;
                meta.tip_height = checkpoint.block_height;
                meta.status = if synced {
                    WalletStatus::Ready
                } else {
                    WalletStatus::Syncing
                };
                trees[5].insert(
                    wid.0.as_slice(),
                    cbor_encode(&meta)
                        .map_err(|error| Conflict::Abort(WalletError::from(error)))?,
                )?;
                trees[6].insert(
                    journal_key.as_slice(),
                    cbor_encode(&undo)
                        .map_err(|error| Conflict::Abort(WalletError::from(error)))?,
                )?;
                Ok(())
            })
            .map_err(transaction_error)?;
        Ok(())
    }

    pub fn rollback_chain(&self, wid: &WalletId, to_slot: u64) -> Result<(), WalletError> {
        let trees = self.chain_trees(wid)?;
        let mut journals = Vec::new();
        for entry in trees[6].iter().rev() {
            let (key, value) = entry?;
            let slot = u64::from_be_bytes(
                key.as_ref()
                    .try_into()
                    .map_err(|_| WalletError::Cbor("invalid undo slot".into()))?,
            );
            if slot <= to_slot {
                break;
            }
            let undo: Vec<Change> = cbor_decode(&value)?;
            journals.push((key, undo));
        }
        let checkpoint = trees[3]
            .range(..=slot_key(to_slot).as_ref())
            .next_back()
            .transpose()?
            .map(|(_, bytes)| cbor_decode::<Checkpoint>(&bytes))
            .transpose()?;
        trees
            .as_slice()
            .transaction(|trees| {
                for (key, undo) in &journals {
                    for (index, key, value) in undo.iter().rev() {
                        match value {
                            Some(value) => {
                                trees[*index].insert(key.as_slice(), value.as_slice())?;
                            }
                            None => {
                                trees[*index].remove(key.as_slice())?;
                            }
                        }
                    }
                    trees[6].remove(key.as_ref())?;
                }
                let bytes = trees[5]
                    .get(wid.0.as_slice())?
                    .ok_or_else(|| Conflict::Abort(WalletError::NotFound(wid.to_hex())))?;
                let mut meta: WalletMeta = cbor_decode(&bytes)
                    .map_err(|error| Conflict::Abort(WalletError::from(error)))?;
                meta.tip_slot = checkpoint.as_ref().map_or(0, |point| point.slot);
                meta.tip_height = checkpoint.as_ref().map_or(0, |point| point.block_height);
                meta.status = WalletStatus::Syncing;
                trees[5].insert(
                    wid.0.as_slice(),
                    cbor_encode(&meta)
                        .map_err(|error| Conflict::Abort(WalletError::from(error)))?,
                )?;
                Ok(())
            })
            .map_err(transaction_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quetzal_primitives::{AddressRole, DerivationPath, PassphraseScheme, Value, WalletType};

    fn fixture() -> (WalletDb, WalletId, UtxoEntry) {
        let db = WalletDb {
            db: sled::Config::new().temporary(true).open().unwrap(),
        };
        let wid = WalletId([7; 20]);
        let meta = WalletMeta {
            id: wid.clone(),
            name: "before".into(),
            account_index: 0,
            address_pool_gap: 20,
            status: WalletStatus::Ready,
            passphrase_scheme: PassphraseScheme::Argon2idV2,
            passphrase_last_updated_at: None,
            tip_slot: 0,
            tip_height: 0,
            created_at: "2026-11-07T00:00:00Z".into(),
            balance_total: Coin::ZERO,
            balance_available: Coin::ZERO,
            balance_reward: Coin::ZERO,
            assets_available: vec![],
            assets_total: vec![],
            delegation_status: Default::default(),
            stake_pool_id: None,
            reward_account_balance: Coin::ZERO,
            wallet_type: WalletType::Shelley,
        };
        db.insert_wallet(&meta).unwrap();
        let utxo = UtxoEntry {
            id: UtxoId::new(TxHash([1; 32]), 0),
            address: Address("owned".into()),
            value: Value::lovelace(Coin(100)),
        };
        db.insert_address(
            &wid,
            &AddressEntry {
                address: utxo.address.clone(),
                path: DerivationPath {
                    account_ix: 0,
                    role: AddressRole::External,
                    address_ix: 0,
                },
                state: AddressState::Unused,
            },
        )
        .unwrap();
        (db, wid, utxo)
    }

    #[test]
    fn rollback_restores_stake_deposits_and_votes_without_reverting_credentials() {
        use pallas_primitives::{conway, StakeCredential};
        use quetzal_primitives::tx::{BlockHeight, TxBlock, TxTime};
        let (db, wid, _) = fixture();
        let credential = StakeCredential::AddrKeyhash([1; 28].into());
        let mut record = TxRecord {
            id: TxHash([2; 32]),
            slot: 10,
            block_hash: [3; 32],
            block_height: 1,
            direction: quetzal_primitives::TxDirection::Outgoing,
            amount: Coin(1000000),
            fee: Some(Coin(100)),
            inputs: vec![],
            outputs: vec![],
            status: quetzal_primitives::TxStatus::InLedger,
            inserted_at: TxTime {
                time: "public-test".into(),
                block: TxBlock {
                    slot_no: 10,
                    epoch_no: 0,
                    height: BlockHeight::new(1),
                },
            },
            metadata: None,
            withdrawals: vec![],
            expires_at_slot: None,
            certificates: vec![
                pallas_codec::minicbor::to_vec(conway::Certificate::Reg(
                    credential.clone(),
                    1000000,
                ))
                .unwrap(),
                pallas_codec::minicbor::to_vec(conway::Certificate::VoteDeleg(
                    credential.clone(),
                    conway::DRep::Abstain,
                ))
                .unwrap(),
            ],
            deposit_taken: Coin(1000000),
            deposit_returned: Coin::ZERO,
            stake_derivation_path: None,
        };
        let first = Checkpoint {
            slot: 10,
            block_hash: [3; 32],
            block_height: 1,
        };
        record.certificates.push(
            pallas_codec::minicbor::to_vec(conway::Certificate::StakeDelegation(
                credential.clone(),
                [7; 28].into(),
            ))
            .unwrap(),
        );
        db.commit_chain_block(&wid, &first, true, &[], std::slice::from_ref(&record), &[])
            .unwrap();
        let initial = db.staking_state(&wid).unwrap();
        assert_eq!(initial.deposit, Some(1000000));
        assert_eq!(initial.vote.as_deref(), Some("abstain"));
        assert!(initial.active_pool(1).is_none());
        assert_eq!(initial.active_pool(2), initial.pool.as_deref());
        record.slot = 20;
        record.id = TxHash([4; 32]);
        record.certificates =
            vec![
                pallas_codec::minicbor::to_vec(conway::Certificate::UnReg(credential, 1000000))
                    .unwrap(),
            ];
        let second = Checkpoint {
            slot: 20,
            block_hash: [5; 32],
            block_height: 2,
        };
        db.commit_chain_block(&wid, &second, true, &[], std::slice::from_ref(&record), &[])
            .unwrap();
        assert_eq!(
            db.staking_state(&wid).unwrap(),
            crate::staking::StakeState::default()
        );
        let mut changed = db.get_wallet(&wid).unwrap().unwrap();
        changed.name = "concurrently renamed".into();
        changed.passphrase_last_updated_at = Some("new password timestamp".into());
        db.insert_wallet(&changed).unwrap();
        db.rollback_chain(&wid, 10).unwrap();
        assert_eq!(db.staking_state(&wid).unwrap(), initial);
        let preserved = db.get_wallet(&wid).unwrap().unwrap();
        assert_eq!(preserved.name, changed.name);
        assert_eq!(
            preserved.passphrase_last_updated_at,
            changed.passphrase_last_updated_at
        );
        db.rollback_chain(&wid, 0).unwrap();
        assert_eq!(
            db.staking_state(&wid).unwrap(),
            crate::staking::StakeState::default()
        );
    }

    #[test]
    fn replay_and_rollback_restore_spent_inputs_addresses_and_checkpoints() {
        let (db, wid, input) = fixture();
        let cp1 = Checkpoint {
            slot: 10,
            block_hash: [1; 32],
            block_height: 1,
        };
        db.commit_chain_block(
            &wid,
            &cp1,
            true,
            &[(input.id.clone(), Some(input.clone()))],
            &[],
            &[],
        )
        .unwrap();
        let mut output = input.clone();
        output.id = UtxoId::new(TxHash([2; 32]), 0);
        output.value.coin = Coin(70);
        let cp2 = Checkpoint {
            slot: 20,
            block_hash: [2; 32],
            block_height: 2,
        };
        let changes = [
            (input.id.clone(), None),
            (output.id.clone(), Some(output.clone())),
        ];
        let expanded = AddressEntry {
            address: Address("derived".into()),
            path: DerivationPath::shelley_external(0, 1),
            state: AddressState::Unused,
        };
        db.commit_chain_block_with_addresses(
            &wid,
            &cp2,
            true,
            BlockChanges {
                utxos: &changes,
                records: &[],
                used: std::slice::from_ref(&input.address),
                addresses: &[expanded],
            },
        )
        .unwrap();
        db.commit_chain_block(
            &wid,
            &cp2,
            true,
            &changes,
            &[],
            std::slice::from_ref(&input.address),
        )
        .unwrap();
        assert_eq!(db.get_utxos(&wid).unwrap()[0].value.coin, Coin(70));
        assert_eq!(
            db.get_addresses(&wid)
                .unwrap()
                .iter()
                .find(|entry| entry.address == input.address)
                .unwrap()
                .state,
            AddressState::Used
        );
        let mut renamed = db.get_wallet(&wid).unwrap().unwrap();
        renamed.name = "after".into();
        db.update_wallet(&renamed).unwrap();
        db.rollback_chain(&wid, 10).unwrap();
        assert_eq!(db.get_addresses(&wid).unwrap().len(), 1);
        let restored = db.get_utxos(&wid).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, input.id);
        assert_eq!(restored[0].value.coin, Coin(100));
        assert_eq!(
            db.get_addresses(&wid).unwrap()[0].state,
            AddressState::Unused
        );
        assert_eq!(db.latest_checkpoint(&wid).unwrap().unwrap().slot, 10);
        let meta = db.get_wallet(&wid).unwrap().unwrap();
        assert_eq!(meta.name, "after");
        assert_eq!(meta.tip_height, 1);
        db.rollback_chain(&wid, 0).unwrap();
        assert!(db.get_utxos(&wid).unwrap().is_empty());
        assert!(db.latest_checkpoint(&wid).unwrap().is_none());
    }

    #[test]
    fn aborted_block_leaves_no_utxo_checkpoint_or_journal() {
        let (db, wid, input) = fixture();
        db.wallets_tree().unwrap().remove(wid.0.as_slice()).unwrap();
        let cp = Checkpoint {
            slot: 10,
            block_hash: [1; 32],
            block_height: 1,
        };
        assert!(db
            .commit_chain_block(
                &wid,
                &cp,
                true,
                &[(input.id.clone(), Some(input))],
                &[],
                &[]
            )
            .is_err());
        assert!(db.get_utxos(&wid).unwrap().is_empty());
        assert!(db.latest_checkpoint(&wid).unwrap().is_none());
        assert!(db.chain_trees(&wid).unwrap()[6].is_empty());
    }
}
