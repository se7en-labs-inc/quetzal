//! Synthetic scale fixtures. These are not native funded-wallet evidence.
use anyhow::{ensure, Context, Result};
use pallas_addresses::Network;
use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
use quetzal_crypto::{derive_icarus_master_key, shelley_account_key};
use quetzal_primitives::{
    tx::{BlockHeight, CoinBundle, TxBlock, TxOutput, TxTime},
    AddressEntry, AddressRole, AddressState, Checkpoint, Coin, DerivationPath, PendingTx,
    TxDirection, TxHash, TxRecord, TxStatus, UtxoEntry, UtxoId, Value, WalletType,
};
use quetzal_wallet::{
    db::import::ImportedWallet, db::WalletDb, wallet::SharedWalletCreation, WalletService,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{collections::HashMap, path::Path};

pub const PASSWORD: &str = "public-runtime-benchmark-password";
const COIN: u64 = 2_000_000;
const TIME: &str = "2023-11-14T22:13:20Z";

#[derive(Debug, Serialize, Deserialize)]
pub struct WalletFixture {
    pub family: String,
    pub path: String,
    pub id: String,
    pub total: u64,
    pub available: u64,
    pub utxos: usize,
    pub history: usize,
    pub addresses: usize,
    pub account_xpub: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub version: u32,
    pub synthetic: bool,
    pub wallets: Vec<WalletFixture>,
}

pub async fn create(destination: &Path, small: bool) -> Result<()> {
    ensure!(!destination.exists(), "fixture destination already exists");
    let parent = destination
        .parent()
        .context("fixture needs a parent directory")?;
    let staging = tempfile::Builder::new()
        .prefix(".runtime-fixture-")
        .tempdir_in(parent)?;
    let source_path = staging.path().join("source");
    let target_path = staging.path().join("database");
    let source = WalletService::new(source_path.to_str().context("non-UTF8 fixture path")?)?;
    let byron = source.byron_service()?;
    let mut wallets = Vec::new();
    wallets.push((
        "shelley",
        source
            .create_wallet(
                "runtime-shelley".into(),
                &[101; 20],
                PASSWORD.as_bytes(),
                20,
                Network::Testnet,
            )
            .await?,
    ));
    let watch = shelley_account_key(&derive_icarus_master_key(&[102; 20], b"")?, 0).public_key();
    wallets.push((
        "watch-only",
        source
            .create_watch_wallet(
                "runtime-watch".into(),
                &watch.to_raw_bytes(),
                20,
                Network::Testnet,
            )
            .await?,
    ));
    wallets.push((
        "icarus",
        byron
            .create_byron_wallet_style(
                "runtime-icarus".into(),
                &[103; 20],
                PASSWORD.as_bytes(),
                Some(42),
                "icarus",
            )
            .await?,
    ));
    wallets.push((
        "random-byron",
        byron
            .create_byron_wallet_style(
                "runtime-random".into(),
                &[104; 20],
                PASSWORD.as_bytes(),
                Some(42),
                "random",
            )
            .await?,
    ));
    let template = json!({"cosigners":{"cosigner#0":"self"},"template":{"all":["cosigner#0"]}});
    wallets.push((
        "shared",
        source
            .create_shared_wallet(SharedWalletCreation {
                name: "runtime-shared".into(),
                entropy: &[105; 20],
                passphrase: PASSWORD.as_bytes(),
                account_ix: 0,
                cosigners: HashMap::from([("cosigner#0".into(), None)]),
                payment_template: &template,
                delegation_template: Some(&template),
                network: Network::Testnet,
            })
            .await?,
    ));
    source.flush()?;
    byron.flush()?;
    drop(byron);
    drop(source);
    let source = WalletDb::open(source_path.to_str().context("non-UTF8 path")?)?;
    let source_byron = WalletDb::open(
        source_path
            .join("byron")
            .to_str()
            .context("non-UTF8 path")?,
    )?;
    let target = WalletDb::open(target_path.to_str().context("non-UTF8 path")?)?;
    let target_byron = WalletDb::open(
        target_path
            .join("byron")
            .to_str()
            .context("non-UTF8 path")?,
    )?;
    let (utxo_count, history_count, address_count) = if small {
        (10, 100, 20)
    } else {
        (1_000, 10_000, 200)
    };
    let mut manifest = Manifest {
        version: 1,
        synthetic: true,
        wallets: Vec::new(),
    };
    for (family, mut metadata) in wallets {
        let legacy = matches!(metadata.wallet_type, WalletType::Byron | WalletType::Icarus);
        let src = if legacy { &source_byron } else { &source };
        let dst = if legacy { &target_byron } else { &target };
        let public = src
            .get_account_xpub(&metadata.id)?
            .context("missing account public key")?;
        let key = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&public)?;
        let credentials = src.load_encrypted_key(&metadata.id)?;
        let payment: Option<serde_json::Value> = src
            .get_script_template(&metadata.id, b"payment_script")?
            .map(|b| serde_json::from_slice(&b))
            .transpose()?;
        let delegation: Option<serde_json::Value> = src
            .get_script_template(&metadata.id, b"delegation_script")?
            .map(|b| serde_json::from_slice(&b))
            .transpose()?;
        let root = credentials
            .as_ref()
            .map(|key| key.decrypt(PASSWORD.as_bytes()))
            .transpose()?;
        let root = root
            .as_ref()
            .map(|raw| quetzal_crypto::Bip32PrivateKey::from_raw_bytes(raw))
            .transpose()?;
        let mut addresses = Vec::new();
        for n in 0..address_count {
            let role = if n % 2 == 0 {
                AddressRole::External
            } else {
                AddressRole::Internal
            };
            let index = (n / 2) as u32;
            let (address, path) = match metadata.wallet_type {
                WalletType::Shelley => (
                    quetzal_address::shelley::base_address_for_role(
                        &key,
                        role.index(),
                        index,
                        0,
                        Network::Testnet,
                    )?,
                    DerivationPath {
                        account_ix: 0,
                        role,
                        address_ix: index,
                    },
                ),
                WalletType::Icarus => (
                    quetzal_address::byron::icarus_from_account_role(
                        &key,
                        role.index(),
                        index,
                        Some(42),
                    )?
                    .to_base58()
                    .into(),
                    DerivationPath {
                        account_ix: 0,
                        role,
                        address_ix: index,
                    },
                ),
                WalletType::Byron => {
                    let index = 0x80000000 + n as u32;
                    (
                        quetzal_address::byron::address(
                            root.as_ref().context("missing Byron root")?,
                            true,
                            0x80000000,
                            index,
                            Some(42),
                        )?
                        .to_base58()
                        .into(),
                        DerivationPath::shelley_external(0x80000000, index),
                    )
                }
                WalletType::Shared => (
                    quetzal_wallet::shared::address_for_path(
                        payment.as_ref().context("missing shared template")?,
                        delegation.as_ref(),
                        role,
                        index,
                        Network::Testnet,
                    )?,
                    DerivationPath {
                        account_ix: 0,
                        role,
                        address_ix: index,
                    },
                ),
            };
            addresses.push(AddressEntry {
                address,
                path,
                state: AddressState::Used,
            });
        }
        let mut history = Vec::new();
        let mut utxos = Vec::new();
        for n in 0..history_count {
            let address = addresses[n % addresses.len()].address.clone();
            let slot = n as u64 + 1;
            let cbor = StagingTransaction::new()
                .network_id(0)
                .fee(0)
                .invalid_from_slot(slot)
                .output(Output::new(address.as_str().parse()?, COIN))
                .build_conway_raw()
                .map_err(|e| anyhow::anyhow!("fixture encoding: {e}"))?
                .tx_bytes
                .0;
            let id = TxHash(quetzal_wallet::txsize::body_hash(&cbor).map_err(anyhow::Error::msg)?);
            if n < utxo_count {
                utxos.push(UtxoEntry {
                    id: UtxoId::new(id.clone(), 0),
                    address: address.clone(),
                    value: Value::lovelace(Coin(COIN)),
                });
            }
            history.push(TxRecord {
                id,
                slot,
                block_hash: [6; 32],
                block_height: slot,
                direction: TxDirection::Incoming,
                amount: Coin(COIN),
                fee: Some(Coin::ZERO),
                inputs: vec![],
                outputs: vec![TxOutput {
                    address,
                    amount: CoinBundle::lovelace(Coin(COIN)),
                    assets: vec![],
                }],
                status: TxStatus::InLedger,
                inserted_at: TxTime {
                    time: TIME.into(),
                    block: TxBlock {
                        slot_no: slot,
                        epoch_no: 0,
                        height: BlockHeight::new(slot),
                    },
                },
                metadata: None,
                withdrawals: vec![],
                expires_at_slot: Some(slot),
                certificates: vec![],
                deposit_taken: Coin::ZERO,
                deposit_returned: Coin::ZERO,
                stake_derivation_path: None,
                ledger_details: quetzal_primitives::tx::TxLedgerDetails {
                    raw_cbor: Some(cbor),
                    ..Default::default()
                },
            });
        }
        // One retained local transaction reserves an input and creates owned change.
        let change = addresses[1].address.clone();
        let cbor = StagingTransaction::new()
            .network_id(0)
            .fee(100_000)
            .invalid_from_slot(1_000_000)
            .input(Input::new(utxos[0].id.tx_hash.0.into(), 0))
            .output(Output::new(change.as_str().parse()?, COIN - 100_000))
            .build_conway_raw()
            .map_err(|e| anyhow::anyhow!("pending encoding: {e}"))?
            .tx_bytes
            .0;
        let pending = PendingTx {
            id: TxHash(quetzal_wallet::txsize::body_hash(&cbor).map_err(anyhow::Error::msg)?),
            cbor,
            submitted_at: TIME.into(),
            expiry_slot: Some(1_000_000),
            submitted_slot: history_count as u64,
            submitted_height: history_count as u64,
            latest_submission_slot: None,
        };
        let ledger_total = utxo_count as u64 * COIN;
        metadata.tip_slot = history_count as u64;
        metadata.tip_height = history_count as u64;
        metadata.created_at = TIME.into();
        metadata.passphrase_last_updated_at = credentials.as_ref().map(|_| TIME.into());
        metadata.balance_total = Coin(ledger_total);
        metadata.balance_available = Coin(ledger_total);
        let expected_available = ledger_total - COIN;
        let expected_total = ledger_total - 100_000;
        dst.import_wallet(&ImportedWallet {
            metadata: metadata.clone(),
            account_public_key: public,
            policy_public_key: None,
            credentials,
            payment_script: payment,
            delegation_script: delegation,
            addresses,
            allocated_change: if metadata.wallet_type == WalletType::Byron {
                vec![]
            } else {
                vec![change]
            },
            checkpoint: Checkpoint {
                slot: history_count as u64,
                block_hash: [6; 32],
                block_height: history_count as u64,
            },
            utxos,
            history,
            pending: vec![pending.clone()],
            submissions: vec![pending],
            staking: Default::default(),
        })?;
        let path = if legacy {
            "byron-wallets"
        } else if family == "shared" {
            "shared-wallets"
        } else {
            "wallets"
        };
        manifest.wallets.push(WalletFixture {
            family: family.into(),
            path: path.into(),
            id: metadata.id.to_hex(),
            total: expected_total,
            available: expected_available,
            utxos: utxo_count,
            history: history_count,
            addresses: address_count,
            account_xpub: hex::encode(public),
        });
    }
    target.flush()?;
    target_byron.flush()?;
    drop(target);
    drop(target_byron);
    drop(source);
    drop(source_byron);
    // Reopen through ordinary service methods; check sums independently of metadata.
    let reopened = WalletService::new(target_path.to_str().context("non-UTF8 path")?)?;
    let legacy = reopened.byron_service()?;
    for wallet in &manifest.wallets {
        let service = if wallet.path == "byron-wallets" {
            &legacy
        } else {
            &reopened
        };
        let id = quetzal_primitives::WalletId::from_hex(&wallet.id)?;
        let meta = service.get_wallet(&id).await?;
        ensure!(
            meta.balance_total.0 == wallet.total && meta.balance_available.0 == wallet.available,
            "fixture balance mismatch"
        );
        ensure!(
            service.get_addresses(&id).await?.len() == wallet.addresses,
            "fixture address count mismatch"
        );
        ensure!(
            service.get_utxos(&id).await?.len() == wallet.utxos,
            "fixture UTxO count mismatch"
        );
        ensure!(
            service.get_transactions(&id, None, usize::MAX).await?.len() == wallet.history + 1,
            "fixture history count mismatch"
        );
    }
    drop(legacy);
    drop(reopened);
    std::fs::write(
        staging.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    std::fs::write(
        staging.path().join("byron-genesis.json"),
        br#"{"protocolConsts":{"protocolMagic":42},"startTime":1700000000}"#,
    )?;
    // Remove only the temporary seed store; publish neither it nor its secrets.
    std::fs::remove_dir_all(&source_path)?;
    std::fs::rename(staging.path(), destination)?;
    println!("{}", destination.join("manifest.json").display());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn all_families_reopen_with_pending_reservations_and_deterministic_manifest() {
        let root = tempfile::tempdir().unwrap();
        let first = root.path().join("first");
        let second = root.path().join("second");
        super::create(&first, true).await.unwrap();
        super::create(&second, true).await.unwrap();
        assert_eq!(
            std::fs::read(first.join("manifest.json")).unwrap(),
            std::fs::read(second.join("manifest.json")).unwrap()
        );
        assert!(super::create(&first, true).await.is_err());
    }
}
