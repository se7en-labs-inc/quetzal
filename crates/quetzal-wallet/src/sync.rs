use std::collections::HashMap;
use std::sync::Arc;

use pallas_traverse::MultiEraBlock;

use quetzal_primitives::tx::{BlockHeight, CoinBundle, TxBlock, TxInput, TxOutput, TxTime};
use quetzal_primitives::{
    Address, Checkpoint, Coin, TxDirection, TxHash, TxRecord, TxStatus, UtxoEntry, UtxoId, WalletId,
};

use crate::db::WalletDb;
use crate::error::WalletError;

type SlotReference = dyn Fn(u64) -> (u64, u64, String) + Send + Sync;

pub struct ChainSyncWorker {
    db: Arc<WalletDb>,
    wallet: Option<WalletId>,
    slot_reference: Option<Arc<SlotReference>>,
}

impl ChainSyncWorker {
    pub fn new(db: Arc<WalletDb>) -> Self {
        ChainSyncWorker {
            db,
            wallet: None,
            slot_reference: None,
        }
    }

    pub fn for_wallet(db: Arc<WalletDb>, wallet: WalletId) -> Self {
        Self {
            db,
            wallet: Some(wallet),
            slot_reference: None,
        }
    }

    pub fn with_slot_reference(
        mut self,
        clock: impl Fn(u64) -> (u64, u64, String) + Send + Sync + 'static,
    ) -> Self {
        self.slot_reference = Some(Arc::new(clock));
        self
    }

    fn wallets(&self) -> Result<Vec<quetzal_primitives::WalletMeta>, WalletError> {
        match &self.wallet {
            Some(wallet) => Ok(vec![self
                .db
                .get_wallet(wallet)?
                .ok_or_else(|| WalletError::NotFound(wallet.to_hex()))?]),
            None => self.db.list_wallets(),
        }
    }

    pub fn checkpoints(&self) -> Result<Vec<Checkpoint>, WalletError> {
        match &self.wallet {
            Some(wallet) => self.db.checkpoints_since(wallet, 100),
            None => Ok(vec![]),
        }
    }

    pub fn process_await(&self) -> Result<(), WalletError> {
        for wallet in self.wallets()? {
            self.db.mark_wallet_synced(&wallet.id)?;
        }
        Ok(())
    }

    /// Process a roll-forward block event.
    ///
    /// Scans block outputs for addresses belonging to our wallets,
    /// updates the UTxO set and tx history accordingly.
    pub async fn process_roll_forward(
        &self,
        block_cbor: &[u8],
        node_tip_slot: u64,
    ) -> Result<(), WalletError> {
        // Decode block CBOR bytes into a MultiEraBlock.
        let block = MultiEraBlock::decode(block_cbor)
            .map_err(|e| WalletError::TxBuild(format!("block decode error: {e}")))?;

        let slot = block.slot();
        let block_hash_bytes: [u8; 32] = *block.hash();
        let block_height = block.number();
        let wallets = self.wallets()?;
        let mut owners = HashMap::<String, Vec<WalletId>>::new();
        let mut pools = HashMap::new();
        for wallet in &wallets {
            let addresses = self.db.get_addresses(&wallet.id)?;
            if let Some(public) = self.db.get_account_xpub(&wallet.id)? {
                if let Some(pool) =
                    crate::discovery::DiscoveryPool::new(wallet, addresses.clone(), public)?
                {
                    pools.insert(wallet.id.clone(), pool);
                }
            }
            for address in addresses {
                owners
                    .entry(address.address.0)
                    .or_default()
                    .push(wallet.id.clone());
            }
        }

        let mut current = HashMap::<WalletId, HashMap<UtxoId, UtxoEntry>>::new();
        let mut updates = HashMap::<WalletId, Vec<(UtxoId, Option<UtxoEntry>)>>::new();
        let mut records = HashMap::<WalletId, Vec<TxRecord>>::new();
        let mut used = HashMap::<WalletId, Vec<Address>>::new();
        for wallet in &wallets {
            current.insert(
                wallet.id.clone(),
                self.db
                    .get_utxos(&wallet.id)?
                    .into_iter()
                    .map(|utxo| (utxo.id.clone(), utxo))
                    .collect(),
            );
        }
        // Stage a whole block before committing any wallet ledger state.
        for tx in block.txs() {
            let tx_hash_bytes: [u8; 32] = *tx.hash();
            let tx_hash = TxHash(tx_hash_bytes);
            let fee = tx.fee().map(Coin);
            let metadata = tx
                .metadata()
                .as_alonzo()
                .map(crate::metadata::decode_ledger)
                .transpose()?;

            // Collect the raw tx outputs for the TxRecord.
            let tx_outputs: Vec<TxOutput> = tx
                .outputs()
                .iter()
                .map(|o| {
                    let addr_str = o.address().map(|a| a.to_string()).unwrap_or_default();
                    let lovelace = o.value().coin();
                    TxOutput {
                        address: Address(addr_str),
                        amount: CoinBundle::lovelace(Coin(lovelace)),
                        assets: crate::ledger::asset_amounts(&crate::ledger::value(&o.value())),
                    }
                })
                .collect();

            // Collect the raw tx inputs for the TxRecord.
            let tx_inputs: Vec<TxInput> = tx
                .consumes()
                .iter()
                .map(|i| {
                    let ref_ = i.output_ref();
                    TxInput {
                        id: TxHash(**ref_.hash()),
                        index: ref_.index() as u32,
                        address: None,
                        amount: None,
                        assets: vec![],
                    }
                })
                .collect();

            // Track per-wallet amounts for outputs sent to our wallets.
            // wallet_id -> (total_lovelace, utxo_entries)
            let mut wallet_received: HashMap<WalletId, u64> = HashMap::new();

            // Process outputs — match against address→wallet index.
            for (out_index, output) in tx.produces() {
                let addr = match output.address() {
                    Ok(a) => a,
                    Err(_) => continue,
                };
                let addr_str = addr.to_string();
                let primitives_addr = Address(addr_str);

                for wid in owners
                    .get(primitives_addr.as_str())
                    .cloned()
                    .unwrap_or_default()
                {
                    let lovelace = output.value().coin();

                    // Insert UTxO into this wallet's utxo tree.
                    let utxo = UtxoEntry {
                        id: UtxoId::new(tx_hash.clone(), out_index as u32),
                        address: primitives_addr.clone(),
                        value: crate::ledger::value(&output.value()),
                    };
                    current
                        .get_mut(&wid)
                        .expect("wallet initialized")
                        .insert(utxo.id.clone(), utxo.clone());
                    updates
                        .entry(wid.clone())
                        .or_default()
                        .push((utxo.id.clone(), Some(utxo)));

                    // Mark address as used.
                    used.entry(wid.clone())
                        .or_default()
                        .push(primitives_addr.clone());
                    if let Some(pool) = pools.get_mut(&wid) {
                        for entry in pool.observe(&primitives_addr)? {
                            owners.entry(entry.address.0).or_default().push(wid.clone());
                        }
                    }

                    // Accumulate received amount.
                    let received = wallet_received.entry(wid).or_insert(0);
                    *received = received
                        .checked_add(lovelace)
                        .ok_or_else(|| WalletError::TxBuild("received amount overflow".into()))?;
                }
            }

            let inserted_at = TxTime {
                time: self
                    .slot_reference
                    .as_ref()
                    .map(|clock| clock(slot).2)
                    .unwrap_or_default(),
                block: TxBlock {
                    slot_no: slot,
                    epoch_no: self
                        .slot_reference
                        .as_ref()
                        .map(|clock| clock(slot).0 as u32)
                        .unwrap_or(0),
                    height: BlockHeight::new(block_height),
                },
            };
            for wallet in &wallets {
                let utxos = current.get_mut(&wallet.id).expect("wallet initialized");
                let mut spent = 0u64;
                let mut resolved_inputs = tx_inputs.clone();
                for input in &mut resolved_inputs {
                    let spent_id = UtxoId::new(input.id.clone(), input.index);
                    if let Some(utxo) = utxos.remove(&spent_id) {
                        spent = spent
                            .checked_add(utxo.value.coin.0)
                            .ok_or_else(|| WalletError::TxBuild("input amount overflow".into()))?;
                        input.address = Some(utxo.address.clone());
                        input.amount = Some(CoinBundle::lovelace(utxo.value.coin));
                        input.assets = crate::ledger::asset_amounts(&utxo.value);
                        updates
                            .entry(wallet.id.clone())
                            .or_default()
                            .push((spent_id, None));
                    }
                }
                let received = wallet_received.get(&wallet.id).copied().unwrap_or(0);
                if spent == 0 && received == 0 {
                    continue;
                }
                let direction = if spent > received {
                    TxDirection::Outgoing
                } else {
                    TxDirection::Incoming
                };
                let amount = if spent > 0 {
                    spent.abs_diff(received)
                } else {
                    received
                };
                let staking = self
                    .db
                    .get_account_xpub(&wallet.id)?
                    .map(|public| {
                        crate::staking::owned_certificates(
                            &tx,
                            &crate::staking::stake_hash(&public)?,
                        )
                    })
                    .transpose()?
                    .unwrap_or_default();
                let record = TxRecord {
                    id: tx_hash.clone(),
                    slot,
                    block_hash: block_hash_bytes,
                    block_height,
                    direction,
                    amount: Coin(amount),
                    fee,
                    inputs: resolved_inputs,
                    outputs: tx_outputs.clone(),
                    status: TxStatus::InLedger,
                    inserted_at: inserted_at.clone(),
                    metadata: metadata.clone(),
                    withdrawals: crate::ledger::withdrawals(&tx)?,
                    expires_at_slot: tx.ttl(),
                    certificates: staking.encoded()?,
                    deposit_taken: Coin(staking.deposit_taken),
                    deposit_returned: Coin(staking.deposit_returned),
                    stake_derivation_path: Some(quetzal_primitives::DerivationPath {
                        account_ix: wallet.account_index,
                        role: quetzal_primitives::AddressRole::Stake,
                        address_ix: 0,
                    }),
                };
                records.entry(wallet.id.clone()).or_default().push(record);
            }
        }

        for wallet in &wallets {
            let addresses = self.db.get_addresses(&wallet.id)?;
            let utxos = current
                .get(&wallet.id)
                .map(|utxos| utxos.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for pending in self.db.list_pending(&wallet.id)? {
                if !pending.expiry_slot.is_some_and(|expiry| slot >= expiry) {
                    continue;
                }
                if records
                    .get(&wallet.id)
                    .is_some_and(|records| records.iter().any(|record| record.id == pending.id))
                {
                    continue;
                }
                let expired =
                    crate::pending::record(&pending, &addresses, &utxos, TxStatus::Expired)?;
                records.entry(wallet.id.clone()).or_default().push(expired);
            }
        }
        // Save checkpoint for all wallets.
        let cp = Checkpoint {
            slot,
            block_hash: block_hash_bytes,
            block_height,
        };
        for wallet in &wallets {
            let addresses = pools
                .get(&wallet.id)
                .map(|pool| pool.changes())
                .unwrap_or_default();
            self.db.commit_chain_block_with_addresses(
                &wallet.id,
                &cp,
                slot >= node_tip_slot,
                crate::db::BlockChanges {
                    utxos: updates
                        .get(&wallet.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    records: records
                        .get(&wallet.id)
                        .map(Vec::as_slice)
                        .unwrap_or_default(),
                    used: used.get(&wallet.id).map(Vec::as_slice).unwrap_or_default(),
                    addresses: &addresses,
                },
            )?;
        }

        Ok(())
    }

    /// Restore all wallet ledger mutations after the intersection point.
    pub async fn process_roll_backward(
        &self,
        to_slot: u64,
        _to_hash: [u8; 32],
    ) -> Result<(), WalletError> {
        for wallet in self.wallets()? {
            self.db.rollback_chain(&wallet.id, to_slot)?;
        }
        Ok(())
    }
}
