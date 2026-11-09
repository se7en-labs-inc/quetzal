use crate::error::WalletError;
use quetzal_primitives::{
    cbor_decode, cbor_encode, slot_key, tx_history_key, utxo_key, Address, AddressEntry,
    AddressState, Checkpoint, Coin, PendingTx, PoolId, PoolMeta, TxHash, TxRecord, UtxoEntry,
    UtxoId, WalletId, WalletMeta, WalletStatus,
};
use sled::{Db, Tree};
mod chain;
mod change;
mod pool_chain;
mod submissions;
pub(crate) use chain::BlockChanges;

pub struct WalletDb {
    db: Db,
}

impl WalletDb {
    pub(crate) fn update_reward(&self, wid: &WalletId, reward: Coin) -> Result<(), WalletError> {
        use sled::transaction::{ConflictableTransactionError as Conflict, TransactionError};
        self.wallets_tree()?
            .transaction(|wallets| {
                let bytes = wallets
                    .get(wid.0.as_slice())?
                    .ok_or_else(|| Conflict::Abort(WalletError::NotFound(wid.to_hex())))?;
                let mut meta: WalletMeta =
                    cbor_decode(&bytes).map_err(|e| Conflict::Abort(WalletError::from(e)))?;
                meta.balance_reward = reward;
                meta.reward_account_balance = reward;
                wallets.insert(
                    wid.0.as_slice(),
                    cbor_encode(&meta).map_err(|e| Conflict::Abort(WalletError::from(e)))?,
                )?;
                Ok(())
            })
            .map_err(|error| match error {
                TransactionError::Abort(error) => error,
                TransactionError::Storage(error) => WalletError::Db(error),
            })
    }
    pub(crate) fn staking_state(
        &self,
        wid: &WalletId,
    ) -> Result<crate::staking::StakeState, WalletError> {
        self.db
            .open_tree(format!("stake:{wid}"))?
            .get(b"state")?
            .map(|bytes| cbor_decode(&bytes).map_err(WalletError::from))
            .transpose()
            .map(|state| state.unwrap_or_default())
    }
    pub fn flush(&self) -> Result<(), WalletError> {
        self.db.flush()?;
        Ok(())
    }
    pub fn open(path: &str) -> Result<Self, sled::Error> {
        let db = sled::open(path)?;
        Ok(WalletDb { db })
    }

    pub(crate) fn pool_metadata_source(&self) -> Result<String, WalletError> {
        self.db
            .open_tree("settings")?
            .get(b"pool_metadata_source")?
            .map(|bytes| {
                String::from_utf8(bytes.to_vec()).map_err(|e| WalletError::Cbor(e.to_string()))
            })
            .transpose()
            .map(|source| source.unwrap_or_else(|| "none".into()))
    }

    pub(crate) fn set_pool_metadata_source(&self, source: &str) -> Result<(), WalletError> {
        self.db
            .open_tree("settings")?
            .insert(b"pool_metadata_source", source.as_bytes())?;
        self.flush()
    }

    pub(crate) fn pool_gc(
        &self,
    ) -> Result<Option<quetzal_primitives::pool::PoolGcState>, WalletError> {
        self.db
            .open_tree("settings")?
            .get(b"pool_gc")?
            .map(|bytes| cbor_decode(&bytes).map_err(WalletError::from))
            .transpose()
    }

    pub(crate) fn store_pool_gc(
        &self,
        state: &quetzal_primitives::pool::PoolGcState,
    ) -> Result<(), WalletError> {
        self.db
            .open_tree("settings")?
            .insert(b"pool_gc", cbor_encode(state)?)?;
        self.flush()
    }

    pub(crate) fn pool_order_seed(&self) -> Result<u64, WalletError> {
        let settings = self.db.open_tree("settings")?;
        loop {
            if let Some(bytes) = settings.get(b"pool_order_seed")? {
                return Ok(cbor_decode(&bytes)?);
            }
            let seed = rand::RngCore::next_u64(&mut rand::rngs::OsRng);
            let encoded = cbor_encode(&seed)?;
            if settings
                .compare_and_swap(b"pool_order_seed", None as Option<&[u8]>, Some(encoded))?
                .is_ok()
            {
                self.flush()?;
                return Ok(seed);
            }
        }
    }

    fn wallets_tree(&self) -> Result<Tree, sled::Error> {
        self.db.open_tree("wallets")
    }

    fn utxos_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("utxos:{}", wid))
    }

    fn txs_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("txs:{}", wid))
    }

    fn addrs_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("addrs:{}", wid))
    }

    fn checkpoints_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("checkpoints:{}", wid))
    }

    fn pending_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("pending:{}", wid))
    }

    fn pools_tree(&self) -> Result<Tree, sled::Error> {
        self.db.open_tree("pools")
    }

    fn addr_wid_index_tree(&self) -> Result<Tree, sled::Error> {
        self.db.open_tree("idx:addr_wid")
    }

    fn keys_tree(&self, wid: &WalletId) -> Result<Tree, sled::Error> {
        self.db.open_tree(format!("keys:{}", wid))
    }

    // ── Wallet CRUD ───────────────────────────────────────────────────────────

    pub fn insert_wallet(&self, meta: &WalletMeta) -> Result<(), WalletError> {
        let tree = self.wallets_tree()?;
        let bytes = cbor_encode(meta)?;
        tree.insert(meta.id.0.as_ref(), bytes)?;
        Ok(())
    }

    pub fn get_wallet(&self, id: &WalletId) -> Result<Option<WalletMeta>, WalletError> {
        let tree = self.wallets_tree()?;
        match tree.get(id.0.as_ref())? {
            Some(ivec) => {
                let meta = cbor_decode::<WalletMeta>(&ivec)?;
                Ok(Some(meta))
            }
            None => Ok(None),
        }
    }

    pub fn update_wallet(&self, meta: &WalletMeta) -> Result<(), WalletError> {
        let tree = self.wallets_tree()?;
        let bytes = cbor_encode(meta)?;
        tree.insert(meta.id.0.as_ref(), bytes)?;
        Ok(())
    }

    pub fn delete_wallet(&self, id: &WalletId) -> Result<(), WalletError> {
        let tree = self.wallets_tree()?;
        tree.remove(id.0.as_ref())?;

        // Drop all per-wallet trees
        self.db.drop_tree(format!("utxos:{}", id))?;
        self.db.drop_tree(format!("txs:{}", id))?;
        self.db.drop_tree(format!("addrs:{}", id))?;
        self.db.drop_tree(format!("checkpoints:{}", id))?;
        self.db.drop_tree(format!("pending:{}", id))?;
        self.db.drop_tree(format!("change:{}", id))?;
        self.db.drop_tree(format!("submissions:{}", id))?;
        self.db.drop_tree(format!("undo:{}", id))?;
        self.db.drop_tree(format!("stake:{}", id))?;
        self.db.drop_tree(format!("keys:{}", id))?;

        Ok(())
    }

    pub fn list_wallets(&self) -> Result<Vec<WalletMeta>, WalletError> {
        let tree = self.wallets_tree()?;
        let mut wallets = Vec::new();
        for item in tree.iter() {
            let (_k, v) = item?;
            let meta = cbor_decode::<WalletMeta>(&v)?;
            wallets.push(meta);
        }
        Ok(wallets)
    }

    // ── Encrypted key storage ─────────────────────────────────────────────────

    pub fn store_encrypted_key(
        &self,
        wid: &WalletId,
        enc_key: &quetzal_crypto::EncryptedKey,
    ) -> Result<(), WalletError> {
        let tree = self.keys_tree(wid)?;
        let bytes = cbor_encode(enc_key)?;
        tree.insert(b"root", bytes)?;
        Ok(())
    }

    pub fn load_encrypted_key(
        &self,
        wid: &WalletId,
    ) -> Result<Option<quetzal_crypto::EncryptedKey>, WalletError> {
        let tree = self.keys_tree(wid)?;
        match tree.get(b"root")? {
            Some(ivec) => {
                let enc = cbor_decode::<quetzal_crypto::EncryptedKey>(&ivec)?;
                Ok(Some(enc))
            }
            None => Ok(None),
        }
    }

    pub(crate) fn replace_credentials(
        &self,
        wid: &WalletId,
        expected: Option<&quetzal_crypto::EncryptedKey>,
        replacement: Option<&quetzal_crypto::EncryptedKey>,
        scheme: quetzal_primitives::PassphraseScheme,
        updated_at: Option<&str>,
    ) -> Result<(), WalletError> {
        use sled::transaction::{
            ConflictableTransactionError as Conflict, TransactionError, Transactional,
        };
        let expected = expected.map(cbor_encode).transpose()?;
        let replacement = replacement.map(cbor_encode).transpose()?;
        let keys = self.keys_tree(wid)?;
        let wallets = self.wallets_tree()?;
        let result = (&keys, &wallets).transaction(|(keys, wallets)| {
            if keys.get(b"root")?.as_deref() != expected.as_deref() {
                return Err(Conflict::Abort(WalletError::TxBuild(
                    "credentials changed concurrently".into(),
                )));
            }
            let raw = wallets
                .get(wid.0.as_slice())?
                .ok_or_else(|| Conflict::Abort(WalletError::NotFound(wid.to_hex())))?;
            let mut meta: WalletMeta =
                cbor_decode(&raw).map_err(|e| Conflict::Abort(WalletError::from(e)))?;
            meta.passphrase_scheme = scheme.clone();
            if let Some(updated_at) = updated_at {
                meta.passphrase_last_updated_at = Some(updated_at.into());
            }
            wallets.insert(
                wid.0.as_slice(),
                cbor_encode(&meta).map_err(|e| Conflict::Abort(WalletError::from(e)))?,
            )?;
            if let Some(bytes) = &replacement {
                keys.insert(b"root", bytes.as_slice())?;
            } else {
                keys.remove(b"root")?;
            }
            Ok(())
        });
        result.map_err(|error| match error {
            TransactionError::Abort(error) => error,
            TransactionError::Storage(error) => WalletError::Db(error),
        })?;
        self.flush()
    }

    pub fn store_account_xpub(&self, wid: &WalletId, xpub: &[u8; 64]) -> Result<(), WalletError> {
        let tree = self.keys_tree(wid)?;
        tree.insert(b"acct_xpub", xpub.as_ref())?;
        Ok(())
    }

    pub fn store_policy_xpub(&self, wid: &WalletId, xpub: &[u8; 64]) -> Result<(), WalletError> {
        self.keys_tree(wid)?.insert(b"policy_xpub", xpub.as_ref())?;
        self.flush()
    }

    pub fn get_policy_xpub(&self, wid: &WalletId) -> Result<Option<[u8; 64]>, WalletError> {
        self.keys_tree(wid)?
            .get(b"policy_xpub")?
            .map(|bytes| {
                bytes
                    .as_ref()
                    .try_into()
                    .map_err(|_| WalletError::Cbor("invalid stored policy key length".into()))
            })
            .transpose()
    }

    pub fn get_account_xpub(&self, wid: &WalletId) -> Result<Option<[u8; 64]>, WalletError> {
        let tree = self.keys_tree(wid)?;
        match tree.get(b"acct_xpub")? {
            Some(ivec) if ivec.len() == 64 => {
                let mut arr = [0u8; 64];
                arr.copy_from_slice(&ivec);
                Ok(Some(arr))
            }
            _ => Ok(None),
        }
    }

    pub fn store_script_template(
        &self,
        wid: &WalletId,
        key: &[u8],
        json: &[u8],
    ) -> Result<(), WalletError> {
        let tree = self.keys_tree(wid)?;
        tree.insert(key, json)?;
        Ok(())
    }

    pub fn get_script_template(
        &self,
        wid: &WalletId,
        key: &[u8],
    ) -> Result<Option<Vec<u8>>, WalletError> {
        let tree = self.keys_tree(wid)?;
        Ok(tree.get(key)?.map(|v| v.to_vec()))
    }

    /// Compare and commit a cosigner patch together with its activation pool.
    pub(crate) fn patch_shared_template(
        &self,
        wid: &WalletId,
        key: &[u8],
        previous: &[u8],
        updated: &[u8],
        addresses: &[AddressEntry],
    ) -> Result<(), WalletError> {
        use sled::transaction::{ConflictableTransactionError, TransactionError, Transactional};
        let encoded = addresses
            .iter()
            .map(|entry| {
                Ok((
                    entry.address.as_str().as_bytes().to_vec(),
                    cbor_encode(entry)?,
                ))
            })
            .collect::<Result<Vec<_>, WalletError>>()?;
        let keys = self.keys_tree(wid)?;
        let addrs = self.addrs_tree(wid)?;
        let index = self.addr_wid_index_tree()?;
        let result = (&keys, &addrs, &index).transaction(|(keys, addrs, index)| {
            if keys.get(key)?.as_deref() != Some(previous) {
                return Err(ConflictableTransactionError::Abort(WalletError::TxBuild(
                    "shared template changed concurrently; retry the patch".into(),
                )));
            }
            keys.insert(key, updated)?;
            for (address, entry) in &encoded {
                addrs.insert(address.as_slice(), entry.as_slice())?;
                index.insert(address.as_slice(), wid.0.as_slice())?;
            }
            Ok(())
        });
        result.map_err(|error| match error {
            TransactionError::Abort(error) => error,
            TransactionError::Storage(error) => WalletError::Db(error),
        })?;
        self.flush()
    }

    // ── UTxO operations ───────────────────────────────────────────────────────

    pub fn insert_utxo(&self, wid: &WalletId, utxo: &UtxoEntry) -> Result<(), WalletError> {
        let tree = self.utxos_tree(wid)?;
        let key = utxo_key(&utxo.id.tx_hash, utxo.id.output_index);
        let bytes = cbor_encode(utxo)?;
        tree.insert(key, bytes)?;
        Ok(())
    }

    pub fn remove_utxo(&self, wid: &WalletId, id: &UtxoId) -> Result<(), WalletError> {
        let tree = self.utxos_tree(wid)?;
        let key = utxo_key(&id.tx_hash, id.output_index);
        tree.remove(key)?;
        Ok(())
    }

    pub fn get_utxos(&self, wid: &WalletId) -> Result<Vec<UtxoEntry>, WalletError> {
        let tree = self.utxos_tree(wid)?;
        let mut utxos = Vec::new();
        for item in tree.iter() {
            let (_k, v) = item?;
            let entry = cbor_decode::<UtxoEntry>(&v)?;
            utxos.push(entry);
        }
        Ok(utxos)
    }

    pub fn get_total_balance(&self, wid: &WalletId) -> Result<Coin, WalletError> {
        let utxos = self.get_utxos(wid)?;
        let mut total = Coin::ZERO;
        for utxo in &utxos {
            total = total
                .checked_add(utxo.value.coin)
                .ok_or(WalletError::TxBuild("coin overflow".to_string()))?;
        }
        Ok(total)
    }

    // ── Transaction history ───────────────────────────────────────────────────

    pub fn insert_tx(&self, wid: &WalletId, tx: &TxRecord) -> Result<(), WalletError> {
        let tree = self.txs_tree(wid)?;
        let key = tx_history_key(tx.slot, &tx.id);
        let bytes = cbor_encode(tx)?;
        tree.insert(key, bytes)?;
        Ok(())
    }

    pub fn get_txs(
        &self,
        wid: &WalletId,
        start_slot: Option<u64>,
        limit: usize,
    ) -> Result<Vec<TxRecord>, WalletError> {
        let tree = self.txs_tree(wid)?;
        let mut txs = Vec::new();

        let iter: Box<dyn Iterator<Item = sled::Result<(sled::IVec, sled::IVec)>>> =
            match start_slot {
                Some(slot) => {
                    let start_key = slot.to_be_bytes();
                    Box::new(tree.range(start_key.as_ref()..))
                }
                None => Box::new(tree.iter()),
            };

        for item in iter {
            if txs.len() >= limit {
                break;
            }
            let (_k, v) = item?;
            let tx = cbor_decode::<TxRecord>(&v)?;
            txs.push(tx);
        }

        Ok(txs)
    }

    pub fn get_tx(&self, wid: &WalletId, id: &TxHash) -> Result<Option<TxRecord>, WalletError> {
        let tree = self.txs_tree(wid)?;
        // Scan all entries and match by tx_hash since the key is slot ++ hash
        for item in tree.iter() {
            let (k, v) = item?;
            // Key is slot[8] ++ tx_hash[32]; check the last 32 bytes
            if k.len() >= 40 && &k[8..40] == id.0.as_ref() {
                let tx = cbor_decode::<TxRecord>(&v)?;
                return Ok(Some(tx));
            }
        }
        Ok(None)
    }

    // ── Address management ────────────────────────────────────────────────────

    pub(crate) fn import_addresses(
        &self,
        wid: &WalletId,
        entries: &[AddressEntry],
    ) -> Result<(), WalletError> {
        use sled::transaction::{
            ConflictableTransactionError as Conflict, TransactionError, Transactional,
        };
        let encoded = entries
            .iter()
            .map(|entry| {
                Ok((
                    entry.address.as_str().as_bytes().to_vec(),
                    cbor_encode(entry)?,
                ))
            })
            .collect::<Result<Vec<_>, WalletError>>()?;
        (
            &self.wallets_tree()?,
            &self.addrs_tree(wid)?,
            &self.addr_wid_index_tree()?,
        )
            .transaction(|(wallets, addresses, index)| {
                if wallets.get(wid.0.as_slice())?.is_none() {
                    return Err(Conflict::Abort(WalletError::NotFound(wid.to_hex())));
                }
                for (key, value) in &encoded {
                    if addresses.get(key.as_slice())?.is_none() {
                        addresses.insert(key.as_slice(), value.as_slice())?;
                    }
                    index.insert(key.as_slice(), wid.0.as_slice())?;
                }
                Ok(())
            })
            .map_err(|error| match error {
                TransactionError::Abort(error) => error,
                TransactionError::Storage(error) => WalletError::Db(error),
            })?;
        self.flush()
    }

    pub fn insert_address(&self, wid: &WalletId, entry: &AddressEntry) -> Result<(), WalletError> {
        let tree = self.addrs_tree(wid)?;
        let key = entry.address.as_str().as_bytes().to_vec();
        let bytes = cbor_encode(entry)?;
        tree.insert(key, bytes)?;

        // Update addr→wid index
        let idx_tree = self.addr_wid_index_tree()?;
        idx_tree.insert(entry.address.as_str().as_bytes(), wid.0.as_ref())?;

        Ok(())
    }

    pub fn get_addresses(&self, wid: &WalletId) -> Result<Vec<AddressEntry>, WalletError> {
        let tree = self.addrs_tree(wid)?;
        let mut addrs = Vec::new();
        for item in tree.iter() {
            let (_k, v) = item?;
            let entry = cbor_decode::<AddressEntry>(&v)?;
            addrs.push(entry);
        }
        Ok(addrs)
    }

    pub fn mark_address_used(&self, wid: &WalletId, addr: &Address) -> Result<(), WalletError> {
        let tree = self.addrs_tree(wid)?;
        let key = addr.as_str().as_bytes();
        if let Some(ivec) = tree.get(key)? {
            let mut entry = cbor_decode::<AddressEntry>(&ivec)?;
            entry.state = AddressState::Used;
            let bytes = cbor_encode(&entry)?;
            tree.insert(key, bytes)?;
        }
        Ok(())
    }

    pub fn lookup_wallet_for_address(
        &self,
        addr: &Address,
    ) -> Result<Option<WalletId>, WalletError> {
        let tree = self.addr_wid_index_tree()?;
        match tree.get(addr.as_str().as_bytes())? {
            Some(ivec) => {
                if ivec.len() == 20 {
                    let mut arr = [0u8; 20];
                    arr.copy_from_slice(&ivec);
                    Ok(Some(WalletId(arr)))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    // ── Checkpoints ───────────────────────────────────────────────────────────

    pub fn save_checkpoint(&self, wid: &WalletId, cp: &Checkpoint) -> Result<(), WalletError> {
        let tree = self.checkpoints_tree(wid)?;
        let key = slot_key(cp.slot);
        let bytes = cbor_encode(cp)?;
        tree.insert(key, bytes)?;
        Ok(())
    }

    pub fn latest_checkpoint(&self, wid: &WalletId) -> Result<Option<Checkpoint>, WalletError> {
        let tree = self.checkpoints_tree(wid)?;
        match tree.last()? {
            Some((_k, v)) => {
                let cp = cbor_decode::<Checkpoint>(&v)?;
                Ok(Some(cp))
            }
            None => Ok(None),
        }
    }

    pub fn checkpoints_since(
        &self,
        wid: &WalletId,
        n: usize,
    ) -> Result<Vec<Checkpoint>, WalletError> {
        let tree = self.checkpoints_tree(wid)?;
        let mut cps: Vec<Checkpoint> = Vec::new();

        // Collect in reverse to get the last n
        for item in tree.iter().rev() {
            if cps.len() >= n {
                break;
            }
            let (_k, v) = item?;
            let cp = cbor_decode::<Checkpoint>(&v)?;
            cps.push(cp);
        }

        // Chain intersection tries the most recent checkpoints first.
        Ok(cps)
    }

    // ── Pending transactions ──────────────────────────────────────────────────

    pub fn insert_pending(&self, wid: &WalletId, tx: &PendingTx) -> Result<(), WalletError> {
        let tree = self.pending_tree(wid)?;
        let bytes = cbor_encode(tx)?;
        tree.insert(tx.id.0.as_ref(), bytes)?;
        Ok(())
    }

    pub fn remove_pending(&self, wid: &WalletId, id: &TxHash) -> Result<(), WalletError> {
        let tree = self.pending_tree(wid)?;
        tree.remove(id.0.as_ref())?;
        Ok(())
    }

    pub fn list_pending(&self, wid: &WalletId) -> Result<Vec<PendingTx>, WalletError> {
        let tree = self.pending_tree(wid)?;
        let mut pending = Vec::new();
        for item in tree.iter() {
            let (_k, v) = item?;
            let tx = cbor_decode::<PendingTx>(&v)?;
            pending.push(tx);
        }
        Ok(pending)
    }

    pub fn mark_pending_submission(
        &self,
        wid: &WalletId,
        id: &TxHash,
        slot: u64,
    ) -> Result<(), WalletError> {
        let tree = self.pending_tree(wid)?;
        loop {
            let Some(previous) = tree.get(id.0.as_slice())? else {
                return Ok(());
            };
            let mut pending: PendingTx = cbor_decode(&previous)?;
            pending.latest_submission_slot = Some(slot);
            let encoded = cbor_encode(&pending)?;
            if tree
                .compare_and_swap(id.0.as_slice(), Some(previous), Some(encoded))?
                .is_ok()
            {
                self.flush()?;
                return Ok(());
            }
        }
    }

    // ── Pools ─────────────────────────────────────────────────────────────────

    pub fn upsert_pool(&self, pool: &PoolMeta) -> Result<(), WalletError> {
        let tree = self.pools_tree()?;
        let bytes = cbor_encode(pool)?;
        tree.insert(pool.id.0.as_ref(), bytes)?;
        Ok(())
    }

    pub fn get_pool(&self, id: &PoolId) -> Result<Option<PoolMeta>, WalletError> {
        let tree = self.pools_tree()?;
        match tree.get(id.0.as_ref())? {
            Some(ivec) => {
                let meta = cbor_decode::<PoolMeta>(&ivec)?;
                Ok(Some(meta))
            }
            None => Ok(None),
        }
    }

    pub fn list_pools(&self) -> Result<Vec<PoolMeta>, WalletError> {
        let tree = self.pools_tree()?;
        let mut pools = Vec::new();
        for item in tree.iter() {
            let (_k, v) = item?;
            let meta = cbor_decode::<PoolMeta>(&v)?;
            pools.push(meta);
        }
        Ok(pools)
    }

    // ── Tip tracking ──────────────────────────────────────────────────────────

    /// Update the tip_slot, tip_height, and status fields of a wallet's metadata.
    pub fn update_wallet_tip(
        &self,
        wid: &WalletId,
        slot: u64,
        height: u64,
        synced: bool,
    ) -> Result<(), WalletError> {
        if let Some(mut meta) = self.get_wallet(wid)? {
            meta.tip_slot = slot;
            meta.tip_height = height;
            meta.status = if synced {
                WalletStatus::Ready
            } else {
                WalletStatus::Syncing
            };
            self.update_wallet(&meta)?;
        }
        Ok(())
    }

    // ── Transaction deletion ──────────────────────────────────────────────────
    /// Change only readiness after the node acknowledges an up-to-date cursor.
    pub(crate) fn mark_wallet_synced(&self, wid: &WalletId) -> Result<(), WalletError> {
        let tree = self.wallets_tree()?;
        loop {
            let old = tree
                .get(wid.0.as_slice())?
                .ok_or_else(|| WalletError::NotFound(wid.to_hex()))?;
            let mut meta: WalletMeta = cbor_decode(&old)?;
            if meta.status == WalletStatus::Ready {
                return Ok(());
            }
            meta.status = WalletStatus::Ready;
            if tree
                .compare_and_swap(wid.0.as_slice(), Some(old), Some(cbor_encode(&meta)?))?
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    /// Delete a single TxRecord identified by slot and tx hash.
    ///
    /// The sled key for txs is `slot[8] ++ tx_hash[32]` (see `tx_history_key`).
    pub fn delete_tx_by_slot_and_hash(
        &self,
        wid: &WalletId,
        slot: u64,
        id: &TxHash,
    ) -> Result<(), WalletError> {
        let tree = self.txs_tree(wid)?;
        let key = tx_history_key(slot, id);
        tree.remove(key)?;
        Ok(())
    }
}

#[cfg(test)]
mod pending_retry_tests {
    use super::*;
    #[test]
    fn retry_slot_survives_reopen_and_never_recreates_a_forgotten_transaction() {
        let path = std::env::temp_dir().join(format!("quetzal-retry-{}", rand::random::<u64>()));
        let wallet = WalletId([11; 20]);
        let id = TxHash([12; 32]);
        let db = WalletDb::open(path.to_str().unwrap()).unwrap();
        let pending = PendingTx {
            id: id.clone(),
            cbor: vec![0x80],
            submitted_at: "public-test-time".into(),
            expiry_slot: Some(1000),
            submitted_slot: 100,
            submitted_height: 50,
            latest_submission_slot: None,
        };
        db.insert_pending(&wallet, &pending).unwrap();
        db.mark_pending_submission(&wallet, &id, 120).unwrap();
        drop(db);
        let db = WalletDb::open(path.to_str().unwrap()).unwrap();
        let stored = db.list_pending(&wallet).unwrap().remove(0);
        assert_eq!(stored.latest_submission_slot, Some(120));
        assert_eq!(stored.submitted_slot, 100);
        assert_eq!(stored.cbor, pending.cbor);
        db.remove_pending(&wallet, &id).unwrap();
        db.mark_pending_submission(&wallet, &id, 140).unwrap();
        assert!(db.list_pending(&wallet).unwrap().is_empty());
        drop(db);
        std::fs::remove_dir_all(path).unwrap();
    }
}
