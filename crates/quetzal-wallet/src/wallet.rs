use std::sync::Arc;

use pallas_addresses::Network;
use pallas_crypto::hash::Hasher;
use quetzal_address::script_enterprise_address;
use quetzal_crypto::{
    derive_icarus_master_key, icarus_account_key, shared_account_key, shelley_account_key,
    shelley_address_key, EncryptedKey,
};
use quetzal_primitives::{
    AddressEntry, AddressRole, AddressState, Coin, DelegationStatus, DerivationPath,
    PassphraseScheme, PendingTx, PoolId, PoolMeta, TxHash, TxRecord, UtxoEntry, WalletId,
    WalletMeta, WalletStatus, WalletType,
};
use std::collections::HashMap;

use crate::db::WalletDb;
use crate::error::WalletError;

pub struct WalletService {
    pub(crate) db: Arc<WalletDb>,
    pub(crate) shared_templates: tokio::sync::Mutex<()>,
    pub(crate) random_addresses: tokio::sync::Mutex<()>,
    storage_path: std::path::PathBuf,
}

pub struct SharedWalletCreation<'a> {
    pub name: String,
    pub entropy: &'a [u8],
    pub passphrase: &'a [u8],
    pub account_ix: u32,
    pub cosigners: HashMap<String, Option<[u8; 64]>>,
    pub payment_template: &'a serde_json::Value,
    pub delegation_template: Option<&'a serde_json::Value>,
    pub network: Network,
}

impl WalletService {
    /// Flush the service's committed wallet state after all writers stop.
    pub fn flush(&self) -> Result<(), WalletError> {
        self.db.flush()
    }

    pub fn new(db_path: &str) -> Result<Self, WalletError> {
        let db = WalletDb::open(db_path)?;
        Ok(WalletService {
            db: Arc::new(db),
            shared_templates: tokio::sync::Mutex::new(()),
            random_addresses: tokio::sync::Mutex::new(()),
            storage_path: db_path.into(),
        })
    }

    /// The public API families have independent identifier namespaces.
    pub fn byron_service(&self) -> Result<Self, WalletError> {
        let path = self.storage_path.join("byron");
        Self::new(
            path.to_str()
                .ok_or_else(|| WalletError::TxBuild("database path is not UTF-8".into()))?,
        )
    }

    pub fn pool_metadata_source(&self) -> Result<String, WalletError> {
        self.db.pool_metadata_source()
    }

    pub fn set_pool_metadata_source(&self, source: &str) -> Result<(), WalletError> {
        self.db.set_pool_metadata_source(source)
    }

    pub fn pool_gc(&self) -> Result<Option<quetzal_primitives::pool::PoolGcState>, WalletError> {
        let source = self.pool_metadata_source()?;
        Ok(self.db.pool_gc()?.filter(|state| state.source == source))
    }

    pub fn update_reward(&self, wallet: &WalletId, reward: Coin) -> Result<(), WalletError> {
        self.db.update_reward(wallet, reward)
    }

    pub fn store_pool_gc(
        &self,
        state: &quetzal_primitives::pool::PoolGcState,
    ) -> Result<(), WalletError> {
        self.db.store_pool_gc(state)
    }

    /// Create a new Shelley wallet from mnemonic entropy.
    ///
    /// Derives the root HD key via the Icarus spec (PBKDF2 with empty passphrase),
    /// derives the account-0 key, computes the WalletId as Blake2b-160 of the
    /// 64-byte root public key, encrypts the root key with the user passphrase,
    /// stores everything in sled, and returns the new WalletMeta.
    pub async fn create_wallet(
        &self,
        name: String,
        entropy: &[u8],
        passphrase: &[u8],
        address_pool_gap: u32,
        network: Network,
    ) -> Result<WalletMeta, WalletError> {
        self.create_wallet_with_second_factor(
            name,
            entropy,
            passphrase,
            address_pool_gap,
            network,
            b"",
        )
        .await
    }

    pub async fn create_wallet_with_second_factor(
        &self,
        name: String,
        entropy: &[u8],
        passphrase: &[u8],
        address_pool_gap: u32,
        network: Network,
        second_factor: &[u8],
    ) -> Result<WalletMeta, WalletError> {
        let root = derive_icarus_master_key(entropy, second_factor)?;

        // 2. Derive account key: m/1852'/1815'/0'
        let account_key = shelley_account_key(&root, 0);

        // 3. Get account public key
        let account_pub = account_key.public_key();

        // 4. Wallet identity is independent of the account derivation path.
        let acct_pub_bytes = account_pub.to_raw_bytes();
        let hash = Hasher::<160>::hash(&root.public_key().to_raw_bytes());
        let wid = WalletId(*hash);

        // Check for duplicate
        if self.db.get_wallet(&wid)?.is_some() {
            return Err(WalletError::AlreadyExists(wid.to_hex()));
        }

        // 5. Encrypt root key with user passphrase
        let enc_key = EncryptedKey::encrypt(&root.to_raw_bytes(), passphrase)?;

        // 6. Store encrypted key and account xpub (xpub can be used for key export without passphrase)
        self.db.store_encrypted_key(&wid, &enc_key)?;
        self.db.store_account_xpub(&wid, &acct_pub_bytes)?;

        // 7. Derive and store initial gap_limit addresses (external and internal)
        let now = chrono_now();
        for ix in 0..address_pool_gap {
            // External (role=0)
            let addr_ext =
                quetzal_address::shelley::base_address_for_role(&account_pub, 0, ix, 0, network)?;
            let ext_entry = AddressEntry {
                address: addr_ext,
                path: DerivationPath::shelley_external(0, ix),
                state: AddressState::Unused,
            };
            self.db.insert_address(&wid, &ext_entry)?;

            // Internal (role=1)
            let addr_int =
                quetzal_address::shelley::base_address_for_role(&account_pub, 1, ix, 0, network)?;
            let int_entry = AddressEntry {
                address: addr_int,
                path: DerivationPath::shelley_internal(0, ix),
                state: AddressState::Unused,
            };
            self.db.insert_address(&wid, &int_entry)?;
        }

        // 8. Build and store WalletMeta
        let meta = WalletMeta {
            id: wid.clone(),
            name,
            account_index: 0,
            address_pool_gap,
            status: WalletStatus::Syncing,
            passphrase_scheme: PassphraseScheme::Argon2idV2,
            passphrase_last_updated_at: Some(now.clone()),
            tip_slot: 0,
            tip_height: 0,
            created_at: now,
            balance_total: Coin::ZERO,
            balance_available: Coin::ZERO,
            balance_reward: Coin::ZERO,
            assets_available: vec![],
            assets_total: vec![],
            delegation_status: DelegationStatus::NotDelegating,
            stake_pool_id: None,
            reward_account_balance: Coin::ZERO,
            wallet_type: WalletType::Shelley,
        };
        self.db.insert_wallet(&meta)?;

        Ok(meta)
    }

    /// List all wallets.
    pub async fn list_wallets(&self) -> Result<Vec<WalletMeta>, WalletError> {
        let mut wallets = Vec::new();
        for wallet in self.db.list_wallets()? {
            wallets.push(self.get_wallet(&wallet.id).await?);
        }
        Ok(wallets)
    }

    pub async fn create_watch_wallet(
        &self,
        name: String,
        account: &[u8],
        gap: u32,
        network: Network,
    ) -> Result<WalletMeta, WalletError> {
        let key = quetzal_crypto::Bip32PublicKey::from_raw_bytes(account)?;
        let id = WalletId(*Hasher::<160>::hash(account));
        if self.db.get_wallet(&id)?.is_some() {
            return Err(WalletError::AlreadyExists(id.to_hex()));
        }
        self.db.store_account_xpub(&id, &key.to_raw_bytes())?;
        for index in 0..gap {
            for role in [0, 1] {
                let address =
                    quetzal_address::shelley::base_address_for_role(&key, role, index, 0, network)?;
                self.db.insert_address(
                    &id,
                    &AddressEntry {
                        address,
                        path: DerivationPath {
                            account_ix: 0,
                            role: if role == 0 {
                                quetzal_primitives::AddressRole::External
                            } else {
                                quetzal_primitives::AddressRole::Internal
                            },
                            address_ix: index,
                        },
                        state: AddressState::Unused,
                    },
                )?;
            }
        }
        let meta = WalletMeta {
            id: id.clone(),
            name,
            account_index: 0,
            address_pool_gap: gap,
            status: WalletStatus::Syncing,
            passphrase_scheme: PassphraseScheme::Argon2idV2,
            passphrase_last_updated_at: None,
            tip_slot: 0,
            tip_height: 0,
            created_at: chrono_now(),
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
        self.db.insert_wallet(&meta)?;
        Ok(meta)
    }

    /// Get a wallet by ID.
    pub async fn get_wallet(&self, id: &WalletId) -> Result<WalletMeta, WalletError> {
        let mut wallet = self
            .db
            .get_wallet(id)?
            .ok_or_else(|| WalletError::NotFound(id.to_hex()))?;
        let (available, total) = self.calculate_balances(id)?;
        wallet.balance_available = available.coin;
        wallet.assets_available = crate::ledger::asset_amounts(&available);
        wallet.assets_total = crate::ledger::asset_amounts(&total);
        wallet.balance_total = total
            .coin
            .checked_add(wallet.balance_reward)
            .ok_or_else(|| WalletError::TxBuild("balance overflow".into()))?;
        Ok(wallet)
    }

    fn reserved_inputs(
        &self,
        id: &WalletId,
    ) -> Result<Vec<quetzal_primitives::UtxoId>, WalletError> {
        use pallas_primitives::{conway, Fragment};
        let mut inputs = Vec::new();
        for pending in self.db.list_pending(id)? {
            let tx = conway::Tx::decode_fragment(&pending.cbor)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            inputs.extend(tx.transaction_body.inputs.iter().map(|input| {
                quetzal_primitives::UtxoId::new(TxHash(*input.transaction_id), input.index as u32)
            }));
        }
        Ok(inputs)
    }

    pub async fn available_utxos(&self, id: &WalletId) -> Result<Vec<UtxoEntry>, WalletError> {
        self.get_wallet(id).await?;
        let reserved = self.reserved_inputs(id)?;
        Ok(self
            .db
            .get_utxos(id)?
            .into_iter()
            .filter(|utxo| !reserved.contains(&utxo.id))
            .collect())
    }

    fn calculate_balances(
        &self,
        id: &WalletId,
    ) -> Result<(quetzal_primitives::Value, quetzal_primitives::Value), WalletError> {
        let reserved = self.reserved_inputs(id)?;
        let mut available = quetzal_primitives::Value::default();
        for utxo in self.db.get_utxos(id)? {
            if !reserved.contains(&utxo.id) {
                available = available
                    .checked_add(&utxo.value)
                    .ok_or_else(|| WalletError::TxBuild("balance overflow".into()))?;
            }
        }
        let mut total = available.clone();
        let owned = self.db.get_addresses(id)?;
        for pending in self.db.list_pending(id)? {
            let tx = pallas_traverse::MultiEraTx::decode(&pending.cbor)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            for output in tx.outputs() {
                let address = output
                    .address()
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?
                    .to_string();
                if owned.iter().any(|entry| entry.address.0 == address) {
                    total = total
                        .checked_add(&crate::ledger::value(&output.value()))
                        .ok_or_else(|| WalletError::TxBuild("balance overflow".into()))?;
                }
            }
        }
        Ok((available, total))
    }

    /// Delete a wallet and all associated data.
    pub async fn delete_wallet(&self, id: &WalletId) -> Result<(), WalletError> {
        // Verify it exists first
        let _ = self.get_wallet(id).await?;
        self.db.delete_wallet(id)
    }

    /// Update passphrase: verify the old passphrase, then re-encrypt the root key
    /// with the new passphrase.
    pub async fn update_passphrase(
        &self,
        id: &WalletId,
        old_passphrase: &[u8],
        new_passphrase: &[u8],
    ) -> Result<(), WalletError> {
        self.change_credentials(id, old_passphrase, new_passphrase, &mut rand::rngs::OsRng)
            .await
    }

    /// Get all derived addresses for a wallet.
    pub async fn get_addresses(&self, id: &WalletId) -> Result<Vec<AddressEntry>, WalletError> {
        let _ = self.get_wallet(id).await?;
        self.db.get_addresses(id)
    }

    pub async fn get_known_addresses(
        &self,
        id: &WalletId,
    ) -> Result<Vec<AddressEntry>, WalletError> {
        self.get_wallet(id).await?;
        self.db.known_addresses(id)
    }

    /// Get all UTxOs for a wallet.
    pub async fn get_utxos(&self, id: &WalletId) -> Result<Vec<UtxoEntry>, WalletError> {
        let _ = self.get_wallet(id).await?;
        self.db.get_utxos(id)
    }

    /// Get total balance as (available, total).
    ///
    /// Currently both are the same (sum of all confirmed UTxOs).
    /// Reward balance is always 0 until staking/withdrawal tracking is implemented.
    pub async fn get_balance(&self, id: &WalletId) -> Result<(Coin, Coin), WalletError> {
        let _ = self.get_wallet(id).await?;
        let (available, total) = self.calculate_balances(id)?;
        Ok((available.coin, total.coin))
    }

    /// Get transaction history with optional slot range and limit.
    pub async fn get_transactions(
        &self,
        id: &WalletId,
        start: Option<u64>,
        limit: usize,
    ) -> Result<Vec<TxRecord>, WalletError> {
        let _ = self.get_wallet(id).await?;
        let mut records = self.db.get_txs(id, start, usize::MAX)?;
        let addresses = self.db.get_addresses(id)?;
        let utxos = self.db.get_utxos(id)?;
        for pending in self.db.list_pending(id)? {
            if start.is_some_and(|start| pending.submitted_slot < start)
                || records.iter().any(|record| record.id == pending.id)
            {
                continue;
            }
            records.push(crate::pending::record(
                &pending,
                &addresses,
                &utxos,
                quetzal_primitives::TxStatus::Pending,
            )?);
        }
        records.sort_by(|a, b| b.slot.cmp(&a.slot).then_with(|| a.id.0.cmp(&b.id.0)));
        records.truncate(limit);
        Ok(records)
    }

    /// Get a specific confirmed transaction.
    pub async fn get_transaction(
        &self,
        wallet_id: &WalletId,
        tx_id: &TxHash,
    ) -> Result<TxRecord, WalletError> {
        let _ = self.get_wallet(wallet_id).await?;
        if let Some(record) = self.db.get_tx(wallet_id, tx_id)? {
            return Ok(record);
        }
        if let Some(pending) = self
            .db
            .list_pending(wallet_id)?
            .into_iter()
            .find(|pending| pending.id == *tx_id)
        {
            return crate::pending::record(
                &pending,
                &self.db.get_addresses(wallet_id)?,
                &self.db.get_utxos(wallet_id)?,
                quetzal_primitives::TxStatus::Pending,
            );
        }
        Err(WalletError::TransactionNotFound(tx_id.to_hex()))
    }

    /// Get all pending (submitted but not confirmed) transactions.
    pub async fn get_pending_txs(&self, id: &WalletId) -> Result<Vec<PendingTx>, WalletError> {
        let _ = self.get_wallet(id).await?;
        self.db.list_pending(id)
    }

    /// Update the wallet name.
    pub async fn update_wallet_name(
        &self,
        id: &WalletId,
        name: String,
    ) -> Result<WalletMeta, WalletError> {
        let mut meta = self.get_wallet(id).await?;
        meta.name = name;
        self.db.update_wallet(&meta)?;
        Ok(meta)
    }

    /// Sign a transaction body hash with the address keys for the given derivation paths.
    ///
    /// Returns a list of `(vk_bytes_32, sig_bytes_64)` pairs – one per input path.
    /// Callers can inject these into the pallas-txbuilder witness set via
    /// `BuiltTransaction::add_signature`.
    pub async fn sign_transaction_inputs(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
        input_paths: &[DerivationPath],
        tx_body_hash: &[u8; 32],
    ) -> Result<Vec<([u8; 32], [u8; 64])>, WalletError> {
        // 1. Load and decrypt the root key.
        let root_bytes = self.decrypt_root(wallet_id, passphrase).await?;

        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root_bytes)
            .map_err(WalletError::Crypto)?;

        let mut results = Vec::with_capacity(input_paths.len());

        for path in input_paths {
            // Derive: root → account → address key
            let account_key = if self.get_wallet(wallet_id).await?.wallet_type == WalletType::Shared
            {
                shared_account_key(&root, path.account_ix)
            } else {
                shelley_account_key(&root, path.account_ix)
            };
            let addr_key = shelley_address_key(&account_key, path.role.index(), path.address_ix);
            let vk_bytes = addr_key.public_key().to_vk_bytes(); // [u8; 32]
            let sig_bytes = quetzal_crypto::sign(&addr_key, tx_body_hash); // [u8; 64]
            results.push((vk_bytes, sig_bytes));
        }

        Ok(results)
    }

    /// Delete a pending transaction by ID.
    pub async fn delete_pending(&self, wid: &WalletId, tx_id: &TxHash) -> Result<(), WalletError> {
        self.db.forget_submission(wid, tx_id)
    }

    /// Store a raw transaction CBOR as a pending (submitted but not yet confirmed) tx.
    ///
    /// Returns the ledger transaction body hash.
    pub async fn submit_pending(
        &self,
        wid: &WalletId,
        cbor: Vec<u8>,
    ) -> Result<TxHash, WalletError> {
        use pallas_primitives::{conway, Fragment};
        let wallet = self.get_wallet(wid).await?;
        let tx =
            conway::Tx::decode_fragment(&cbor).map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let tx_id =
            TxHash(crate::txsize::body_hash(&cbor).map_err(|e| WalletError::TxBuild(e.into()))?);
        let expiry_slot = tx.transaction_body.ttl;
        let addresses = self.db.get_addresses(wid)?;
        let multi = pallas_traverse::MultiEraTx::decode(&cbor)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let mut change = Vec::new();
        for output in multi.outputs() {
            let address = output
                .address()
                .map_err(|e| WalletError::TxBuild(e.to_string()))?
                .to_string();
            if let Some(entry) = addresses.iter().find(|entry| {
                entry.address.as_str() == address && entry.path.role == AddressRole::Internal
            }) {
                change.push(entry.address.clone());
            }
        }

        let pending = PendingTx {
            id: tx_id.clone(),
            cbor,
            submitted_at: chrono_now(),
            expiry_slot,
            submitted_slot: wallet.tip_slot,
            submitted_height: wallet.tip_height,
            latest_submission_slot: None,
        };
        self.db.insert_pending_and_change(wid, &pending, &change)?;
        self.db.flush()?;
        Ok(tx_id)
    }

    /// Return the stored 64-byte account extended public key (no passphrase needed).
    pub async fn get_account_xpub_bytes(&self, id: &WalletId) -> Result<[u8; 64], WalletError> {
        let _ = self.get_wallet(id).await?;
        self.db
            .get_account_xpub(id)?
            .ok_or_else(|| WalletError::NotFound(id.to_hex()))
    }

    /// Decrypt root key and derive an account key at a specific (hardened) account index.
    /// Returns the 64-byte extended public key of that account.
    pub async fn derive_account_xpub_at(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
        account_ix: u32,
    ) -> Result<[u8; 64], WalletError> {
        let root_bytes = self.decrypt_root(wallet_id, passphrase).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root_bytes)
            .map_err(WalletError::Crypto)?;
        let acct = shelley_account_key(&root, account_ix);
        Ok(acct.public_key().to_raw_bytes())
    }

    /// Decrypt root key and derive the CIP-1855 policy account key (m/1855'/1815'/0').
    /// Returns the 64-byte extended public key.
    pub async fn derive_policy_xpub(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
    ) -> Result<[u8; 64], WalletError> {
        let root_bytes = self.decrypt_root(wallet_id, passphrase).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root_bytes)
            .map_err(WalletError::Crypto)?;
        let policy = quetzal_crypto::policy_account_key(&root);
        Ok(policy.public_key().to_raw_bytes())
    }

    pub async fn write_policy_xpub(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
    ) -> Result<[u8; 64], WalletError> {
        let public = self.derive_policy_xpub(wallet_id, passphrase).await?;
        self.db.store_policy_xpub(wallet_id, &public)?;
        Ok(public)
    }

    pub async fn policy_xpub(&self, wallet: &WalletId) -> Result<[u8; 64], WalletError> {
        self.get_wallet(wallet).await?;
        self.db
            .get_policy_xpub(wallet)?
            .ok_or(WalletError::MissingPolicyPublicKey)
    }

    /// Sign arbitrary data (e.g. a message hash) with the address key at role/ix.
    /// Returns `(vk_bytes_32, sig_bytes_64)`.
    pub async fn sign_with_key(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
        role: u32,
        ix: u32,
        data: &[u8],
    ) -> Result<([u8; 32], [u8; 64]), WalletError> {
        let root_bytes = self.decrypt_root(wallet_id, passphrase).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root_bytes)
            .map_err(WalletError::Crypto)?;
        let account = shelley_account_key(&root, 0);
        let addr_key = shelley_address_key(&account, role, ix);
        // sign expects a 32-byte hash; if data.len() != 32 hash it first
        let hash: [u8; 32] = if data.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(data);
            arr
        } else {
            let h: pallas_crypto::hash::Hash<32> = pallas_crypto::hash::Hasher::<256>::hash(data);
            *h
        };
        let vk = addr_key.public_key().to_vk_bytes();
        let sig = quetzal_crypto::sign(&addr_key, &hash);
        Ok((vk, sig))
    }

    /// Create a new Icarus (Byron BIP-44) wallet from mnemonic entropy.
    ///
    /// Uses the BIP-44 derivation path `m/44'/1815'/0'/0/ix` and encodes
    /// addresses as base58 Byron PubKey addresses.
    pub async fn create_byron_wallet(
        &self,
        name: String,
        entropy: &[u8],
        passphrase: &[u8],
        network: Network,
    ) -> Result<WalletMeta, WalletError> {
        let magic = if matches!(network, Network::Mainnet) {
            None
        } else {
            Some(1097911063)
        };
        self.create_byron_wallet_style(name, entropy, passphrase, magic, "icarus")
            .await
    }

    pub async fn create_byron_wallet_style(
        &self,
        name: String,
        entropy: &[u8],
        passphrase: &[u8],
        magic: Option<u32>,
        style: &str,
    ) -> Result<WalletMeta, WalletError> {
        let random = style == "random";
        let root = match style {
            "random" => quetzal_crypto::derivation::derive_byron_master_key(entropy)?,
            "icarus" | "trezor" => derive_icarus_master_key(entropy, b"")?,
            "ledger" => {
                let mnemonic = bip39::Mnemonic::from_entropy(entropy)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?;
                quetzal_crypto::derivation::derive_ledger_master_key(&mnemonic.to_string())?
            }
            _ => {
                return Err(WalletError::TxBuild(
                    "unsupported Byron wallet style".into(),
                ))
            }
        };
        let id = WalletId(*Hasher::<160>::hash(&root.public_key().to_raw_bytes()));
        if self.db.get_wallet(&id)?.is_some() {
            return Err(WalletError::AlreadyExists(id.to_hex()));
        }
        let encrypted = EncryptedKey::encrypt_byron(&root.to_raw_bytes(), passphrase)?;
        self.db.store_encrypted_key(&id, &encrypted)?;
        let public = if random {
            root.public_key()
        } else {
            icarus_account_key(&root, 0).public_key()
        };
        self.db.store_account_xpub(&id, &public.to_raw_bytes())?;
        if !random {
            for index in 0..20 {
                let address =
                    quetzal_address::byron::address(&root, false, 0x80000000, index, magic)?;
                self.db.insert_address(
                    &id,
                    &AddressEntry {
                        address: quetzal_primitives::Address(address.to_base58()),
                        path: DerivationPath::shelley_external(0, index),
                        state: AddressState::Unused,
                    },
                )?;
            }
            for index in 0..20 {
                let address =
                    quetzal_address::byron::icarus_from_account_role(&public, 1, index, magic)?;
                self.db.insert_address(
                    &id,
                    &AddressEntry {
                        address: address.to_base58().into(),
                        path: DerivationPath::shelley_internal(0, index),
                        state: AddressState::Unused,
                    },
                )?;
            }
        }
        let now = chrono_now();
        let meta = WalletMeta {
            id: id.clone(),
            name,
            account_index: 0,
            address_pool_gap: if random { 0 } else { 20 },
            status: WalletStatus::Syncing,
            passphrase_scheme: PassphraseScheme::Pbkdf2HmacSha512,
            passphrase_last_updated_at: Some(now.clone()),
            tip_slot: 0,
            tip_height: 0,
            created_at: now,
            balance_total: Coin::ZERO,
            balance_available: Coin::ZERO,
            balance_reward: Coin::ZERO,
            assets_available: vec![],
            assets_total: vec![],
            delegation_status: Default::default(),
            stake_pool_id: None,
            reward_account_balance: Coin::ZERO,
            wallet_type: if random {
                WalletType::Byron
            } else {
                WalletType::Icarus
            },
        };
        self.db.insert_wallet(&meta)?;
        Ok(meta)
    }

    /// List all Byron/Icarus wallets.
    pub async fn get_byron_wallets(&self) -> Result<Vec<WalletMeta>, WalletError> {
        let all = self.list_wallets().await?;
        Ok(all
            .into_iter()
            .filter(|w| matches!(w.wallet_type, WalletType::Byron | WalletType::Icarus))
            .collect())
    }

    /// Get a Byron/Icarus wallet by ID. Returns NotFound if the wallet exists
    /// but is not a Byron/Icarus wallet.
    pub async fn get_byron_wallet(&self, id: &WalletId) -> Result<WalletMeta, WalletError> {
        let meta = self.get_wallet(id).await?;
        if !matches!(meta.wallet_type, WalletType::Byron | WalletType::Icarus) {
            return Err(WalletError::NotFound(id.to_hex()));
        }
        Ok(meta)
    }

    /// Delete a Byron/Icarus wallet and all associated data.
    pub async fn delete_byron_wallet(&self, id: &WalletId) -> Result<(), WalletError> {
        let _ = self.get_byron_wallet(id).await?;
        self.db.delete_wallet(id)
    }

    /// Create a [`ChainSyncWorker`] that shares the same database handle.
    pub fn chain_sync_worker(&self) -> crate::sync::ChainSyncWorker {
        crate::sync::ChainSyncWorker::new(Arc::clone(&self.db))
    }

    pub fn wallet_sync_worker(&self, wallet: WalletId) -> crate::sync::ChainSyncWorker {
        crate::sync::ChainSyncWorker::for_wallet(Arc::clone(&self.db), wallet)
    }

    // ── Pool cache ────────────────────────────────────────────────────────────

    pub fn observe_pool_block(
        &self,
        slot: u64,
        hash: [u8; 32],
        pool: Option<[u8; 28]>,
        registrations: &[[u8; 28]],
    ) -> Result<(), WalletError> {
        self.db.pool_block(slot, hash, pool, registrations)
    }

    pub fn rollback_pool_blocks(&self, slot: u64) -> Result<(), WalletError> {
        self.db.rollback_pool_blocks(slot)
    }

    pub fn pool_block_counts(
        &self,
    ) -> Result<std::collections::BTreeMap<Vec<u8>, u64>, WalletError> {
        self.db.pool_block_counts()
    }

    pub fn registered_pools(&self) -> Result<std::collections::BTreeSet<Vec<u8>>, WalletError> {
        self.db.registered_pools()
    }

    pub fn pool_order_seed(&self) -> Result<u64, WalletError> {
        self.db.pool_order_seed()
    }

    /// Bulk-upsert pool metadata into the local cache.
    pub async fn update_pool_cache(&self, pools: Vec<PoolMeta>) -> Result<(), WalletError> {
        for pool in &pools {
            self.db.upsert_pool(pool)?;
        }
        Ok(())
    }

    /// List all cached pools.
    pub async fn list_pools(&self) -> Result<Vec<PoolMeta>, WalletError> {
        self.db.list_pools()
    }

    /// Look up a single pool by ID.
    pub async fn get_pool(&self, id: &PoolId) -> Result<Option<PoolMeta>, WalletError> {
        self.db.get_pool(id)
    }

    // ── Shared wallets (CIP-1854) ─────────────────────────────────────────────

    /// Create a new shared wallet from mnemonic entropy.
    ///
    /// `cosigners` maps cosigner IDs (e.g. "cosigner#0") to raw 64-byte xpub bytes;
    /// `None` means "self" — the wallet's own CIP-1854 derived key.
    pub async fn create_shared_wallet(
        &self,
        creation: SharedWalletCreation<'_>,
    ) -> Result<WalletMeta, WalletError> {
        let SharedWalletCreation {
            name,
            entropy,
            passphrase,
            account_ix,
            cosigners,
            payment_template,
            delegation_template,
            network,
        } = creation;
        let root = derive_icarus_master_key(entropy, b"")?;
        let account_key = shared_account_key(&root, account_ix);
        let account_pub = account_key.public_key();
        let acct_pub_bytes = account_pub.to_raw_bytes();

        let script_node = payment_template.get("template").ok_or_else(|| {
            WalletError::TxBuild("missing template in payment_script_template".into())
        })?;

        let mut cosigner_xpubs: HashMap<String, quetzal_crypto::Bip32PublicKey> = HashMap::new();
        for (id, xpub_opt) in &cosigners {
            let pub_key = match xpub_opt {
                None => account_pub.clone(),
                Some(bytes) => quetzal_crypto::Bip32PublicKey::from_raw_bytes(bytes)?,
            };
            cosigner_xpubs.insert(id.clone(), pub_key);
        }

        let mut identity = acct_pub_bytes.to_vec();
        identity.extend_from_slice(script_template_text(script_node)?.as_bytes());
        if let Some(delegation) = delegation_template {
            identity.extend_from_slice(script_template_text(&delegation["template"])?.as_bytes());
        }
        let wid_hash = Hasher::<160>::hash(&identity);
        let wid = WalletId(*wid_hash);

        if self.db.get_wallet(&wid)?.is_some() {
            return Err(WalletError::AlreadyExists(wid.to_hex()));
        }

        let resolve = |template: &serde_json::Value,
                       credential: &str|
         -> Result<serde_json::Value, WalletError> {
            let mut template = template.clone();
            let cosigners = template["cosigners"]
                .as_object_mut()
                .ok_or_else(|| WalletError::TxBuild("missing cosigners".into()))?;
            for value in cosigners.values_mut() {
                if value == "self" {
                    *value = serde_json::Value::String(quetzal_address::codec::bech32_encode(
                        "acct_shared_xvk",
                        &acct_pub_bytes,
                    )?);
                } else {
                    let encoded = value
                        .as_str()
                        .ok_or_else(|| WalletError::TxBuild("invalid cosigner key".into()))?;
                    let bytes = match hex::decode(encoded) {
                        Ok(bytes) => bytes,
                        Err(_) => quetzal_address::codec::bech32_decode(encoded)?.1,
                    };
                    let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&bytes)?;
                    *value = serde_json::Value::String(quetzal_address::codec::bech32_encode(
                        "acct_shared_xvk",
                        &public.to_raw_bytes(),
                    )?);
                }
            }
            fn contains_own(
                node: &serde_json::Value,
                cosigners: &serde_json::Value,
                own: &str,
            ) -> bool {
                node.as_str().is_some_and(|name| cosigners[name] == own)
                    || node
                        .as_array()
                        .is_some_and(|nodes| nodes.iter().any(|n| contains_own(n, cosigners, own)))
                    || node.as_object().is_some_and(|nodes| {
                        nodes.values().any(|n| contains_own(n, cosigners, own))
                    })
            }
            let own = quetzal_address::codec::bech32_encode("acct_shared_xvk", &acct_pub_bytes)?;
            if !contains_own(&template["template"], &template["cosigners"], &own) {
                return Err(WalletError::SharedWallet { code: "shared_wallet_script_template_invalid", message: format!("It looks like you've tried to create a shared wallet with a template script for {credential} credential that does not pass validation. The problem is: The wallet's account key must be always present for the script template.") });
            }
            Ok(template)
        };
        let resolved_payment = resolve(payment_template, "payment")?;
        let resolved_delegation = delegation_template
            .map(|template| resolve(template, "delegation"))
            .transpose()?;
        let delegation_keys = resolved_delegation
            .as_ref()
            .map(template_cosigners)
            .transpose()?;
        let complete = template_is_complete(&resolved_payment)
            && resolved_delegation
                .as_ref()
                .is_none_or(template_is_complete);
        let enc_key = EncryptedKey::encrypt(&root.to_raw_bytes(), passphrase)?;
        self.db.store_encrypted_key(&wid, &enc_key)?;
        self.db.store_account_xpub(&wid, &acct_pub_bytes)?;
        let payment_json = serde_json::to_vec(&resolved_payment)
            .map_err(|e| WalletError::TxBuild(format!("serialize template: {e}")))?;
        self.db
            .store_script_template(&wid, b"payment_script", &payment_json)?;
        if let Some(del) = &resolved_delegation {
            let del_json = serde_json::to_vec(del)
                .map_err(|e| WalletError::TxBuild(format!("serialize template: {e}")))?;
            self.db
                .store_script_template(&wid, b"delegation_script", &del_json)?;
        }

        let address_pool_gap = 20u32;
        let now = chrono_now();
        for ix in 0..if complete { address_pool_gap } else { 0 } {
            for role in [0u32, 1u32] {
                let key_hashes = derive_cosigner_key_hashes(&cosigner_xpubs, role, ix)?;
                let sh = native_script_hash(script_node, &key_hashes)?;
                let addr = if let (Some(delegation), Some(keys)) =
                    (&resolved_delegation, &delegation_keys)
                {
                    let hashes = derive_cosigner_key_hashes(keys, 2, ix)?;
                    let stake = native_script_hash(&delegation["template"], &hashes)?;
                    let address = pallas_addresses::ShelleyAddress::new(
                        network,
                        pallas_addresses::ShelleyPaymentPart::script_hash(sh),
                        pallas_addresses::ShelleyDelegationPart::Script(stake),
                    );
                    quetzal_primitives::Address(
                        address
                            .to_bech32()
                            .map_err(|error| WalletError::TxBuild(error.to_string()))?,
                    )
                } else {
                    script_enterprise_address(sh, network)?
                };
                let entry = AddressEntry {
                    address: addr,
                    path: if role == 0 {
                        DerivationPath::shelley_external(account_ix, ix)
                    } else {
                        DerivationPath::shelley_internal(account_ix, ix)
                    },
                    state: AddressState::Unused,
                };
                self.db.insert_address(&wid, &entry)?;
            }
        }

        let meta = WalletMeta {
            id: wid.clone(),
            name,
            account_index: account_ix,
            address_pool_gap,
            status: WalletStatus::Syncing,
            passphrase_scheme: PassphraseScheme::Argon2idV2,
            passphrase_last_updated_at: Some(now.clone()),
            tip_slot: 0,
            tip_height: 0,
            created_at: now,
            balance_total: Coin::ZERO,
            balance_available: Coin::ZERO,
            balance_reward: Coin::ZERO,
            assets_available: vec![],
            assets_total: vec![],
            delegation_status: DelegationStatus::NotDelegating,
            stake_pool_id: None,
            reward_account_balance: Coin::ZERO,
            wallet_type: WalletType::Shared,
        };
        self.db.insert_wallet(&meta)?;

        Ok(meta)
    }

    /// List all shared wallets.
    pub async fn get_shared_wallets(&self) -> Result<Vec<WalletMeta>, WalletError> {
        let all = self.db.list_wallets()?;
        Ok(all
            .into_iter()
            .filter(|w| w.wallet_type == WalletType::Shared)
            .collect())
    }

    /// Get a shared wallet by ID. Returns NotFound if the wallet is not of type Shared.
    pub async fn get_shared_wallet(&self, id: &WalletId) -> Result<WalletMeta, WalletError> {
        let meta = self.get_wallet(id).await?;
        if meta.wallet_type != WalletType::Shared {
            return Err(WalletError::NotFound(id.to_hex()));
        }
        Ok(meta)
    }

    /// Delete a shared wallet and all associated data.
    pub async fn delete_shared_wallet(&self, id: &WalletId) -> Result<(), WalletError> {
        let _ = self.get_shared_wallet(id).await?;
        self.db.delete_wallet(id)
    }

    /// Get a stored script template as a JSON value.
    pub async fn get_script_template(
        &self,
        id: &WalletId,
        key: &[u8],
    ) -> Result<Option<serde_json::Value>, WalletError> {
        let _ = self.get_shared_wallet(id).await?;
        match self.db.get_script_template(id, key)? {
            Some(bytes) => {
                let v: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|e| WalletError::TxBuild(format!("deserialize template: {e}")))?;
                Ok(Some(v))
            }
            None => Ok(None),
        }
    }

    /// Decrypt root key and derive the CIP-1854 shared account key at `account_ix`.
    pub async fn derive_shared_account_xpub_at(
        &self,
        wallet_id: &WalletId,
        passphrase: &[u8],
        account_ix: u32,
    ) -> Result<[u8; 64], WalletError> {
        let root_bytes = self.decrypt_root(wallet_id, passphrase).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root_bytes)?;
        let acct = shared_account_key(&root, account_ix);
        Ok(acct.public_key().to_raw_bytes())
    }
}

// ── Native script helpers (CIP-1854 / shared wallet addresses) ───────────────

fn script_template_text(node: &serde_json::Value) -> Result<String, WalletError> {
    if let Some(cosigner) = node.as_str() {
        let index = cosigner
            .strip_prefix("cosigner#")
            .and_then(|index| index.parse::<u8>().ok())
            .ok_or_else(|| WalletError::TxBuild("invalid cosigner".into()))?;
        return Ok(format!("cosigner#{index}"));
    }
    for name in ["all", "any"] {
        if let Some(nodes) = node[name].as_array() {
            return Ok(format!(
                "{name} [{}]",
                nodes
                    .iter()
                    .map(script_template_text)
                    .collect::<Result<Vec<_>, _>>()?
                    .join(",")
            ));
        }
    }
    if let Some(some) = node.get("some") {
        let count = some["at_least"]
            .as_u64()
            .ok_or_else(|| WalletError::TxBuild("invalid threshold".into()))?;
        let nodes = some["from"]
            .as_array()
            .ok_or_else(|| WalletError::TxBuild("invalid threshold scripts".into()))?;
        return Ok(format!(
            "at_least {count} [{}]",
            nodes
                .iter()
                .map(script_template_text)
                .collect::<Result<Vec<_>, _>>()?
                .join(",")
        ));
    }
    for name in ["active_from", "active_until"] {
        if let Some(slot) = node[name].as_u64() {
            return Ok(format!("{name} {slot}"));
        }
    }
    Err(WalletError::TxBuild("invalid script template".into()))
}

pub fn template_is_complete(template: &serde_json::Value) -> bool {
    fn complete(node: &serde_json::Value, cosigners: &serde_json::Value) -> bool {
        if let Some(cosigner) = node.as_str() {
            return cosigners[cosigner]
                .as_str()
                .is_some_and(|value| value != "self");
        }
        for name in ["all", "any"] {
            if let Some(nodes) = node[name].as_array() {
                return nodes.iter().all(|node| complete(node, cosigners));
            }
        }
        if let Some(nodes) = node["some"]["from"].as_array() {
            return nodes.iter().all(|node| complete(node, cosigners));
        }
        node["active_from"].is_u64() || node["active_until"].is_u64()
    }
    complete(&template["template"], &template["cosigners"])
}

pub(crate) fn template_cosigners(
    template: &serde_json::Value,
) -> Result<HashMap<String, quetzal_crypto::Bip32PublicKey>, WalletError> {
    template["cosigners"]
        .as_object()
        .ok_or_else(|| WalletError::TxBuild("missing cosigners".into()))?
        .iter()
        .map(|(name, value)| {
            let value = value
                .as_str()
                .ok_or_else(|| WalletError::TxBuild("invalid cosigner key".into()))?;
            let (_, bytes) = quetzal_address::codec::bech32_decode(value)?;
            Ok((
                name.clone(),
                quetzal_crypto::Bip32PublicKey::from_raw_bytes(&bytes)?,
            ))
        })
        .collect()
}

pub(crate) fn derive_cosigner_key_hashes(
    cosigner_xpubs: &HashMap<String, quetzal_crypto::Bip32PublicKey>,
    role: u32,
    ix: u32,
) -> Result<HashMap<String, [u8; 28]>, WalletError> {
    let mut hashes = HashMap::new();
    for (id, xpub) in cosigner_xpubs {
        let role_key = xpub.derive(role).map_err(WalletError::Crypto)?;
        let addr_key = role_key.derive(ix).map_err(WalletError::Crypto)?;
        let vk = addr_key.to_vk_bytes();
        let h = pallas_crypto::hash::Hasher::<224>::hash(&vk);
        let mut arr = [0u8; 28];
        arr.copy_from_slice(h.as_ref());
        hashes.insert(id.clone(), arr);
    }
    Ok(hashes)
}

/// CBOR-encode a Cardano native script derived from a script template JSON node.
///
/// Substitutes `cosigner#N` placeholders with the 28-byte key hashes provided
/// in `key_hashes`.  Encoding follows the ledger spec:
///   ScriptPubkey → [0, h"28b"]
///   ScriptAll    → [1, [scripts…]]
///   ScriptAny    → [2, [scripts…]]
///   ScriptNofK   → [3, n, [scripts…]]
pub(crate) fn encode_native_script(
    node: &serde_json::Value,
    key_hashes: &HashMap<String, [u8; 28]>,
    buf: &mut Vec<u8>,
) -> Result<(), WalletError> {
    fn cbor_uint(n: u64, buf: &mut Vec<u8>) {
        match n {
            0..=23 => buf.push(n as u8),
            24..=0xFF => {
                buf.push(0x18);
                buf.push(n as u8);
            }
            0x100..=0xFFFF => {
                buf.push(0x19);
                buf.extend_from_slice(&(n as u16).to_be_bytes());
            }
            _ => {
                buf.push(0x1b);
                buf.extend_from_slice(&n.to_be_bytes());
            }
        }
    }
    fn cbor_bytes(data: &[u8], buf: &mut Vec<u8>) {
        let n = data.len();
        match n {
            0..=23 => buf.push(0x40 | n as u8),
            24..=0xFF => {
                buf.push(0x58);
                buf.push(n as u8);
            }
            _ => {
                buf.push(0x59);
                buf.extend_from_slice(&(n as u16).to_be_bytes());
            }
        }
        buf.extend_from_slice(data);
    }
    fn cbor_arr(n: usize, buf: &mut Vec<u8>) {
        match n {
            0..=23 => buf.push(0x80 | n as u8),
            _ => {
                buf.push(0x98);
                buf.push(n as u8);
            }
        }
    }

    if let Some(cosigner) = node.as_str() {
        let h = key_hashes
            .get(cosigner)
            .ok_or_else(|| WalletError::TxBuild(format!("unknown cosigner: {cosigner}")))?;
        cbor_arr(2, buf);
        cbor_uint(0, buf);
        cbor_bytes(h, buf);
    } else if let Some(arr) = node.get("all").and_then(|v| v.as_array()) {
        cbor_arr(2, buf);
        cbor_uint(1, buf);
        cbor_arr(arr.len(), buf);
        for n in arr {
            encode_native_script(n, key_hashes, buf)?;
        }
    } else if let Some(arr) = node.get("any").and_then(|v| v.as_array()) {
        cbor_arr(2, buf);
        cbor_uint(2, buf);
        cbor_arr(arr.len(), buf);
        for n in arr {
            encode_native_script(n, key_hashes, buf)?;
        }
    } else if let Some(some_obj) = node.get("some") {
        let k = some_obj
            .get("at_least")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| WalletError::TxBuild("missing at_least in script".into()))?;
        let from = some_obj
            .get("from")
            .and_then(|v| v.as_array())
            .ok_or_else(|| WalletError::TxBuild("missing from in script".into()))?;
        cbor_arr(3, buf);
        cbor_uint(3, buf);
        cbor_uint(k, buf);
        cbor_arr(from.len(), buf);
        for n in from {
            encode_native_script(n, key_hashes, buf)?;
        }
    } else if let Some(slot) = node.get("active_from").and_then(|v| v.as_u64()) {
        cbor_arr(2, buf);
        cbor_uint(4, buf);
        cbor_uint(slot, buf);
    } else if let Some(slot) = node.get("active_until").and_then(|v| v.as_u64()) {
        cbor_arr(2, buf);
        cbor_uint(5, buf);
        cbor_uint(slot, buf);
    } else {
        return Err(WalletError::TxBuild(
            "unsupported script template node".into(),
        ));
    }
    Ok(())
}

/// Compute Blake2b-224 of (0x00 || CBOR(native_script)) — the Cardano native script hash.
pub(crate) fn native_script_hash(
    node: &serde_json::Value,
    key_hashes: &HashMap<String, [u8; 28]>,
) -> Result<pallas_crypto::hash::Hash<28>, WalletError> {
    let mut cbor = Vec::new();
    encode_native_script(node, key_hashes, &mut cbor)?;
    let mut preimage = vec![0u8]; // 0x00 = native script type prefix
    preimage.extend_from_slice(&cbor);
    Ok(pallas_crypto::hash::Hasher::<224>::hash(&preimage))
}

/// Return the current UTC time as an ISO-8601 string.
///
/// Uses a simple hand-rolled approach to avoid pulling in `chrono` or `time`
/// as a dependency.  In production this can be replaced with a proper time crate.
pub(crate) fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = elapsed.as_secs();
    // Format as ISO-8601 UTC with second precision
    let s = secs % 60;
    let m = (secs / 60) % 60;
    let h = (secs / 3600) % 24;
    let days = secs / 86400;
    // Days since 1970-01-01 → calendar date (Gregorian proleptic)
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:09}Z",
        year,
        month,
        day,
        h,
        m,
        s,
        elapsed.subsec_nanos()
    )
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
fn days_to_ymd(mut days: u64) -> (u64, u8, u8) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    days += 719468;
    let era = days / 146097;
    let doe = days % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u8, d as u8)
}
