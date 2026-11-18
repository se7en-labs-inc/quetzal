use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use pallas_traverse::MultiEraBlock;

use quetzal_primitives::tx::{BlockHeight, CoinBundle, TxBlock, TxInput, TxOutput, TxTime};
use quetzal_primitives::{
    Address, Checkpoint, Coin, TxDirection, TxHash, TxRecord, TxStatus, UtxoEntry, UtxoId, WalletId,
};

use crate::db::WalletDb;
use crate::error::WalletError;

type SlotReference = dyn Fn(u64) -> (u64, u64, String) + Send + Sync;

/// Normalized block lookups shared by every wallet in a replay cohort.
#[derive(Default)]
pub struct BlockIndex {
    produced: Vec<Vec<u8>>,
    consumed: Vec<UtxoId>,
}
impl BlockIndex {
    pub fn new(block: &MultiEraBlock<'_>) -> Self {
        let mut produced = Vec::new();
        let mut consumed = Vec::new();
        for tx in block.txs() {
            for (_, output) in tx.produces() {
                if let Ok(address) = output.address() {
                    produced.push(address.to_vec());
                }
            }
            for input in tx.consumes() {
                let reference = input.output_ref();
                consumed.push(UtxoId::new(
                    TxHash(**reference.hash()),
                    reference.index() as u32,
                ));
            }
        }
        Self { produced, consumed }
    }
}

pub struct ChainSyncWorker {
    db: Arc<WalletDb>,
    wallet: Option<WalletId>,
    slot_reference: Option<Arc<SlotReference>>,
    replay: Mutex<Option<ReplayState>>,
    pools_only: bool,
    pool_batch: Mutex<Vec<crate::db::ObservedPoolBlock>>,
}

struct ReplayState {
    epoch: u64,
    wallets: Vec<quetzal_primitives::WalletMeta>,
    owners: HashMap<Vec<u8>, Vec<WalletId>>,
    pools: HashMap<WalletId, crate::discovery::DiscoveryPool>,
    current: HashMap<WalletId, HashMap<UtxoId, UtxoEntry>>,
    addresses: HashMap<WalletId, Vec<quetzal_primitives::AddressEntry>>,
    deferred: HashMap<WalletId, (Checkpoint, usize)>,
}

impl ChainSyncWorker {
    pub fn new(db: Arc<WalletDb>) -> Self {
        ChainSyncWorker {
            db,
            wallet: None,
            slot_reference: None,
            replay: Mutex::new(None),
            pools_only: false,
            pool_batch: Mutex::new(Vec::new()),
        }
    }

    pub fn for_wallet(db: Arc<WalletDb>, wallet: WalletId) -> Self {
        Self {
            db,
            wallet: Some(wallet),
            slot_reference: None,
            replay: Mutex::new(None),
            pools_only: false,
            pool_batch: Mutex::new(Vec::new()),
        }
    }

    pub fn with_slot_reference(
        mut self,
        clock: impl Fn(u64) -> (u64, u64, String) + Send + Sync + 'static,
    ) -> Self {
        self.slot_reference = Some(Arc::new(clock));
        self
    }

    pub(crate) fn for_pools(db: Arc<WalletDb>) -> Self {
        Self {
            pools_only: true,
            ..Self::new(db)
        }
    }

    pub fn is_pool_follower(&self) -> bool {
        self.pools_only
    }

    fn wallets(&self) -> Result<Vec<quetzal_primitives::WalletMeta>, WalletError> {
        if self.pools_only {
            return Ok(Vec::new());
        }
        match &self.wallet {
            Some(wallet) => Ok(vec![self
                .db
                .get_wallet(wallet)?
                .ok_or_else(|| WalletError::NotFound(wallet.to_hex()))?]),
            None => self.db.list_wallets(),
        }
    }

    pub fn checkpoints(&self) -> Result<Vec<Checkpoint>, WalletError> {
        if self.pools_only {
            return self.db.pool_checkpoints();
        }
        match &self.wallet {
            Some(wallet) => self.db.checkpoints_since(wallet, 100),
            None => Ok(vec![]),
        }
    }

    pub fn process_await(&self) -> Result<(), WalletError> {
        self.commit_progress()?;
        for wallet in self.wallets()? {
            self.db.mark_wallet_synced(&wallet.id)?;
        }
        Ok(())
    }

    /// Persist the endpoint of a run of blocks containing no wallet mutations.
    /// Replaying these blocks after a crash is safe; relevant changes never wait.
    pub fn commit_progress(&self) -> Result<(), WalletError> {
        if self.pools_only {
            let mut batch = self
                .pool_batch
                .lock()
                .map_err(|_| WalletError::TxBuild("pool batch lock poisoned".into()))?;
            if !batch.is_empty() {
                self.db.pool_blocks(&batch)?;
                batch.clear();
            }
            return Ok(());
        }
        let mut cached = self
            .replay
            .lock()
            .map_err(|_| WalletError::TxBuild("replay lock poisoned".into()))?;
        if let Some(state) = cached.as_mut() {
            for (wallet, (checkpoint, _)) in &state.deferred {
                self.db
                    .commit_chain_block(wallet, checkpoint, false, &[], &[], &[])?;
            }
            state.deferred.clear();
        }
        Ok(())
    }

    /// Reconcile the selected native point before replay, including origin
    /// fallback when a migrated snapshot is no longer on the node's chain.
    pub async fn process_intersection(&self, selected: &Checkpoint) -> Result<(), WalletError> {
        self.invalidate()?;
        if self.pools_only {
            return self.db.rollback_pool_blocks(selected.slot);
        }
        for wallet in self.wallets()? {
            if self.db.latest_checkpoint(&wallet.id)?.is_some_and(|p| {
                p.slot > selected.slot
                    || p.slot == selected.slot && p.block_hash != selected.block_hash
            }) {
                self.db.rollback_chain(&wallet.id, selected.slot)?;
            }
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

        self.process_decoded(&block, node_tip_slot)
    }

    fn invalidate(&self) -> Result<(), WalletError> {
        self.pool_batch
            .lock()
            .map_err(|_| WalletError::TxBuild("pool batch lock poisoned".into()))?
            .clear();
        *self
            .replay
            .lock()
            .map_err(|_| WalletError::TxBuild("replay lock poisoned".into()))? = None;
        Ok(())
    }

    /// Apply a block already decoded by the shared follower. On failure discard
    /// speculative indexes; reconnect/replay rebuilds them from canonical state.
    pub fn process_decoded(
        &self,
        block: &MultiEraBlock<'_>,
        node_tip_slot: u64,
    ) -> Result<(), WalletError> {
        self.process_indexed(block, &BlockIndex::new(block), node_tip_slot)
    }

    pub fn process_indexed(
        &self,
        block: &MultiEraBlock<'_>,
        index: &BlockIndex,
        node_tip_slot: u64,
    ) -> Result<(), WalletError> {
        if self.pools_only {
            let issuer = block
                .header()
                .issuer_vkey()
                .map(|key| *pallas_crypto::hash::Hasher::<224>::hash(key));
            let mut registrations = Vec::new();
            for tx in block.txs() {
                if !tx.is_valid() {
                    continue;
                }
                for certificate in tx.certs() {
                    if let Some(pallas_primitives::conway::Certificate::PoolRegistration {
                        operator,
                        ..
                    }) = certificate.as_conway()
                    {
                        registrations.push(**operator);
                    } else if let Some(pallas_primitives::alonzo::Certificate::PoolRegistration {
                        operator,
                        ..
                    }) = certificate.as_alonzo()
                    {
                        registrations.push(**operator);
                    }
                }
            }
            let mut batch = self
                .pool_batch
                .lock()
                .map_err(|_| WalletError::TxBuild("pool batch lock poisoned".into()))?;
            batch.push(crate::db::ObservedPoolBlock {
                slot: block.slot(),
                hash: *block.hash(),
                issuer,
                registrations,
            });
            if batch.len() >= 256 || block.slot() >= node_tip_slot {
                self.db.pool_blocks(&batch)?;
                batch.clear();
            }
            return Ok(());
        }
        let mut cached = self
            .replay
            .lock()
            .map_err(|_| WalletError::TxBuild("replay lock poisoned".into()))?;
        let mut state = match cached
            .take()
            .filter(|state| state.epoch == self.db.discovery_epoch())
        {
            Some(state) => state,
            None => self.load_replay()?,
        };
        self.apply_decoded(block, index, node_tip_slot, &mut state)?;
        *cached = Some(state);
        Ok(())
    }

    fn load_replay(&self) -> Result<ReplayState, WalletError> {
        let epoch = self.db.discovery_epoch();
        let wallets = self.wallets()?;
        let mut owners = HashMap::<Vec<u8>, Vec<WalletId>>::new();
        let mut pools = HashMap::new();
        let mut current = HashMap::new();
        let mut known = HashMap::new();
        for wallet in &wallets {
            let addresses = self.db.get_addresses(&wallet.id)?;
            if let Some(public) = self.db.get_account_xpub(&wallet.id)? {
                if let Some(pool) =
                    crate::discovery::DiscoveryPool::new(wallet, addresses.clone(), public)?
                {
                    pools.insert(wallet.id.clone(), pool);
                }
            }
            for address in &addresses {
                owners
                    .entry(
                        address
                            .address
                            .as_str()
                            .parse::<pallas_addresses::Address>()
                            .map_err(|error| WalletError::TxBuild(error.to_string()))?
                            .to_vec(),
                    )
                    .or_default()
                    .push(wallet.id.clone());
            }
            known.insert(wallet.id.clone(), addresses);
            current.insert(
                wallet.id.clone(),
                self.db
                    .get_utxos(&wallet.id)?
                    .into_iter()
                    .map(|utxo| (utxo.id.clone(), utxo))
                    .collect(),
            );
        }
        Ok(ReplayState {
            epoch,
            wallets,
            owners,
            pools,
            current,
            addresses: known,
            deferred: HashMap::new(),
        })
    }

    fn apply_decoded(
        &self,
        block: &MultiEraBlock<'_>,
        index: &BlockIndex,
        node_tip_slot: u64,
        state: &mut ReplayState,
    ) -> Result<(), WalletError> {
        let ReplayState {
            wallets,
            owners,
            pools,
            current,
            addresses: known,
            deferred,
            ..
        } = state;

        let slot = block.slot();
        let block_hash_bytes: [u8; 32] = *block.hash();
        let block_height = block.number();
        let mut updates = HashMap::<WalletId, Vec<(UtxoId, Option<UtxoEntry>)>>::new();
        let mut records = HashMap::<WalletId, Vec<TxRecord>>::new();
        let mut used = HashMap::<WalletId, Vec<Address>>::new();
        // Discovery must reach a fixed point before applying transactions: a
        // later output can reveal an earlier output beyond the original gap.
        let mut observed = HashSet::new();
        loop {
            let mut extended = false;
            for raw in &index.produced {
                let Some(ids) = owners.get(raw).cloned() else {
                    continue;
                };
                if !observed.insert(raw.clone()) {
                    continue;
                }
                let address = Address(
                    pallas_addresses::Address::from_bytes(raw)
                        .map_err(|error| WalletError::TxBuild(error.to_string()))?
                        .to_string(),
                );
                for id in ids {
                    if let Some(pool) = pools.get_mut(&id) {
                        for entry in pool.observe(&address)? {
                            owners
                                .entry(
                                    entry
                                        .address
                                        .as_str()
                                        .parse::<pallas_addresses::Address>()
                                        .map_err(|error| WalletError::TxBuild(error.to_string()))?
                                        .to_vec(),
                                )
                                .or_default()
                                .push(id.clone());
                            known.entry(id.clone()).or_default().push(entry);
                            extended = true;
                        }
                    }
                }
            }
            if !extended {
                break;
            }
        }
        // Stage a whole block before committing any wallet ledger state.
        let relevant = !observed.is_empty()
            || index
                .consumed
                .iter()
                .any(|id| current.values().any(|utxos| utxos.contains_key(id)));
        for tx in if relevant { block.txs() } else { Vec::new() } {
            let relevant_output = tx.produces().iter().any(|(_, output)| {
                output
                    .address()
                    .ok()
                    .is_some_and(|address| owners.contains_key(&address.to_vec()))
            });
            let relevant_input = tx.consumes().iter().any(|input| {
                let reference = input.output_ref();
                let id = UtxoId::new(TxHash(**reference.hash()), reference.index() as u32);
                current.values().any(|utxos| utxos.contains_key(&id))
            });
            if !relevant_output && !relevant_input {
                continue;
            }
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

                for wid in owners.get(&addr.to_vec()).cloned().unwrap_or_default() {
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
            for wallet in wallets.iter() {
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
                    ledger_details: quetzal_primitives::tx::TxLedgerDetails {
                        extra_signatures: crate::ledger::extra_signatures(&tx),
                        validity_interval: (!matches!(tx, pallas_traverse::MultiEraTx::Byron(_)))
                            .then(|| {
                                (
                                    tx.validity_start().unwrap_or(0),
                                    tx.ttl().unwrap_or(u64::MAX),
                                )
                            }),
                        ..Default::default()
                    },
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

        for wallet in wallets.iter() {
            let expired: Vec<_> = self
                .db
                .list_pending(&wallet.id)?
                .into_iter()
                .filter(|pending| pending.expiry_slot.is_some_and(|expiry| slot >= expiry))
                .collect();
            if expired.is_empty() {
                continue;
            }
            let addresses = known.get(&wallet.id).map(Vec::as_slice).unwrap_or_default();
            let utxos = current
                .get(&wallet.id)
                .map(|utxos| utxos.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for pending in expired {
                if records
                    .get(&wallet.id)
                    .is_some_and(|records| records.iter().any(|record| record.id == pending.id))
                {
                    continue;
                }
                let expired =
                    crate::pending::record(&pending, addresses, &utxos, TxStatus::Expired)?;
                records.entry(wallet.id.clone()).or_default().push(expired);
            }
        }
        // Save checkpoint for all wallets.
        let cp = Checkpoint {
            slot,
            block_hash: block_hash_bytes,
            block_height,
        };
        for wallet in wallets.iter() {
            let addresses = pools
                .get_mut(&wallet.id)
                .map(|pool| pool.take_changes())
                .unwrap_or_default();
            for changed in &addresses {
                if let Some(existing) = known.get_mut(&wallet.id).and_then(|entries| {
                    entries
                        .iter_mut()
                        .find(|entry| entry.address == changed.address)
                }) {
                    *existing = changed.clone();
                }
            }
            let unchanged = addresses.is_empty()
                && updates.get(&wallet.id).is_none_or(Vec::is_empty)
                && records.get(&wallet.id).is_none_or(Vec::is_empty)
                && used.get(&wallet.id).is_none_or(Vec::is_empty);
            if unchanged && slot < node_tip_slot {
                let pending = deferred.entry(wallet.id.clone()).or_insert((cp.clone(), 0));
                *pending = (cp.clone(), pending.1 + 1);
                if pending.1 < 256 {
                    continue;
                }
            }
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
            deferred.remove(&wallet.id);
        }

        Ok(())
    }

    /// Restore all wallet ledger mutations after the intersection point.
    pub async fn process_roll_backward(
        &self,
        to_slot: u64,
        _to_hash: [u8; 32],
    ) -> Result<(), WalletError> {
        self.invalidate()?;
        if self.pools_only {
            return self.db.rollback_pool_blocks(to_slot);
        }
        for wallet in self.wallets()? {
            self.db.rollback_chain(&wallet.id, to_slot)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pallas_codec::{
        minicbor,
        utils::{KeyValuePairs, MaybeIndefArray},
    };
    use pallas_primitives::{babbage, conway, Fragment, VrfCert};
    use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
    use quetzal_primitives::AddressRole;

    fn block(slot: u64, transactions: Vec<conway::Tx>) -> Vec<u8> {
        let header = babbage::Header {
            header_body: babbage::HeaderBody {
                block_number: slot,
                slot,
                prev_hash: None,
                issuer_vkey: vec![0; 32].into(),
                vrf_vkey: vec![0; 32].into(),
                vrf_result: VrfCert(vec![0; 32].into(), vec![0; 80].into()),
                block_body_size: 0,
                block_body_hash: [0; 32].into(),
                operational_cert: babbage::OperationalCert {
                    operational_cert_hot_vkey: vec![0; 32].into(),
                    operational_cert_sequence_number: 0,
                    operational_cert_kes_period: 0,
                    operational_cert_sigma: vec![0; 64].into(),
                },
                protocol_version: (9, 0),
            },
            body_signature: vec![0; 448].into(),
        };
        let body = conway::Block {
            header,
            transaction_bodies: MaybeIndefArray::Def(
                transactions
                    .iter()
                    .map(|tx| tx.transaction_body.clone())
                    .collect(),
            ),
            transaction_witness_sets: MaybeIndefArray::Def(
                transactions
                    .iter()
                    .map(|tx| tx.transaction_witness_set.clone())
                    .collect(),
            ),
            auxiliary_data_set: KeyValuePairs::Def(vec![]),
            invalid_transactions: Some(MaybeIndefArray::Def(vec![])),
        };
        minicbor::to_vec((7u16, body)).unwrap()
    }
    fn transaction(input: [u8; 32], outputs: &[(Address, u64)]) -> conway::Tx {
        let mut staging = StagingTransaction::new()
            .fee(1)
            .input(Input::new(input.into(), 0));
        for (address, coin) in outputs {
            staging = staging.output(Output::new(address.as_str().parse().unwrap(), *coin));
        }
        conway::Tx::decode_fragment(&staging.build_conway_raw().unwrap().tx_bytes.0).unwrap()
    }
    async fn wallet() -> (tempfile::TempDir, crate::WalletService, WalletId) {
        let directory = tempfile::tempdir().unwrap();
        let service = crate::WalletService::new(directory.path().to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "sync regression".into(),
                &[111; 20],
                b"public-test-password",
                2,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        (directory, service, wallet.id)
    }
    #[tokio::test]
    async fn reversed_gap_trigger_and_same_block_spend_are_discovered_and_rollback_rebuilds() {
        let (_directory, service, id) = wallet().await;
        let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(
            &service.db.get_account_xpub(&id).unwrap().unwrap(),
        )
        .unwrap();
        let address = |index| {
            quetzal_address::shelley::base_address_for_role(
                &public,
                0,
                index,
                0,
                pallas_addresses::Network::Testnet,
            )
            .unwrap()
        };
        // Address 3 precedes address 1, which extends the initial gap [0,2).
        let receipt = transaction([7; 32], &[(address(3), 100), (address(1), 200)]);
        let receipt_id = pallas_crypto::hash::Hasher::<256>::hash(
            &minicbor::to_vec(&receipt.transaction_body).unwrap(),
        );
        let external = Address(
            pallas_addresses::Address::from_bytes(&[vec![0x60], vec![9; 28]].concat())
                .unwrap()
                .to_string(),
        );
        let spend = transaction(*receipt_id, &[(external, 99)]);
        let worker = service.wallet_sync_worker(id.clone());
        worker
            .process_roll_forward(&block(10, vec![receipt, spend]), 100)
            .await
            .unwrap();
        assert_eq!(service.db.get_total_balance(&id).unwrap().0, 200);
        assert_eq!(service.db.get_txs(&id, None, 100).unwrap().len(), 2);
        assert!(service
            .db
            .get_addresses(&id)
            .unwrap()
            .iter()
            .any(|entry| entry.path.role == AddressRole::External && entry.path.address_ix == 5));
        worker.process_roll_backward(0, [0; 32]).await.unwrap();
        assert_eq!(service.db.get_total_balance(&id).unwrap().0, 0);
        assert!(service.db.get_txs(&id, None, 100).unwrap().is_empty());
        worker
            .process_roll_forward(
                &block(11, vec![transaction([8; 32], &[(address(1), 300)])]),
                100,
            )
            .await
            .unwrap();
        assert_eq!(service.db.get_total_balance(&id).unwrap().0, 300);
    }
    #[tokio::test]
    async fn empty_progress_is_bounded_and_committed_on_await_and_external_addresses_invalidate() {
        let (_directory, service, id) = wallet().await;
        let worker = service.wallet_sync_worker(id.clone());
        for slot in 1..=255 {
            worker
                .process_roll_forward(&block(slot, vec![]), 1000)
                .await
                .unwrap();
        }
        assert!(service.db.latest_checkpoint(&id).unwrap().is_none());
        worker
            .process_roll_forward(&block(256, vec![]), 1000)
            .await
            .unwrap();
        assert_eq!(
            service.db.latest_checkpoint(&id).unwrap().unwrap().slot,
            256
        );
        worker
            .process_roll_forward(&block(257, vec![]), 1000)
            .await
            .unwrap();
        worker.process_await().unwrap();
        assert_eq!(
            service.db.latest_checkpoint(&id).unwrap().unwrap().slot,
            257
        );
        let mut entry = service.db.get_addresses(&id).unwrap().remove(0);
        entry.address = Address(
            pallas_addresses::Address::from_bytes(&[vec![0x60], vec![11; 28]].concat())
                .unwrap()
                .to_string(),
        );
        service.db.insert_address(&id, &entry).unwrap();
        worker
            .process_roll_forward(
                &block(258, vec![transaction([8; 32], &[(entry.address, 500)])]),
                1000,
            )
            .await
            .unwrap();
        assert_eq!(service.db.get_total_balance(&id).unwrap().0, 500);
    }
}
