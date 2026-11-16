mod convert;
mod publish;
mod source;
use anyhow::{bail, ensure, Context, Result};
use quetzal_primitives::WalletType;
use quetzal_wallet::db::{import::ImportedWallet, WalletDb};
use serde_json::json;
use std::{collections::BTreeSet, fs, path::PathBuf, time::Duration};
struct Options {
    command: String,
    source: PathBuf,
    destination: Option<PathBuf>,
    sqlite: PathBuf,
    socket: Option<String>,
    genesis: Option<PathBuf>,
    mainnet: bool,
}
fn main() {
    if let Err(error) = run() {
        eprintln!("migration failed: {error:#}");
        std::process::exit(1)
    }
}
fn options() -> Result<Option<Options>> {
    let mut args = std::env::args().skip(1);
    let Some(command) = args.next() else {
        help();
        return Ok(None);
    };
    if ["--help", "help", "-h"].contains(&command.as_str()) {
        help();
        return Ok(None);
    }
    ensure!(
        ["inspect", "check", "import"].contains(&command.as_str()),
        "unknown command; use --help"
    );
    let mut source = None;
    let mut destination = None;
    let mut sqlite = PathBuf::from(option_env!("QUETZAL_SQLITE3").unwrap_or("sqlite3"));
    let mut socket = None;
    let mut genesis = None;
    let mut mainnet = false;
    let mut seen = BTreeSet::new();
    while let Some(flag) = args.next() {
        ensure!(seen.insert(flag.clone()), "duplicate option {flag}");
        if flag == "--mainnet" {
            mainnet = true;
            continue;
        }
        let value = args
            .next()
            .filter(|v| !v.starts_with("--"))
            .with_context(|| format!("missing value for {flag}"))?;
        match flag.as_str() {
            "--source" => source = Some(value.into()),
            "--destination" => destination = Some(value.into()),
            "--sqlite3" => sqlite = value.into(),
            "--node-socket" => socket = Some(value),
            "--testnet" => genesis = Some(value.into()),
            _ => bail!("unknown option {flag}"),
        }
    }
    let source = source.context("--source is required")?;
    if command != "inspect" {
        ensure!(
            mainnet != genesis.is_some(),
            "select exactly one of --mainnet or --testnet FILE"
        );
        ensure!(
            socket.is_some(),
            "--node-socket is required for native timing and checkpoint validation"
        );
        if command == "import" {
            ensure!(destination.is_some(), "--destination is required");
        } else {
            ensure!(destination.is_none(), "check does not accept --destination");
        }
    } else {
        ensure!(
            destination.is_none() && socket.is_none() && !mainnet && genesis.is_none(),
            "inspect accepts only --source and --sqlite3"
        );
    }
    Ok(Some(Options {
        command,
        source,
        destination,
        sqlite,
        socket,
        genesis,
        mainnet,
    }))
}
fn run() -> Result<()> {
    let Some(o) = options()? else { return Ok(()) };
    let paths = source::inventory(&o.source)?;
    if o.command == "inspect" {
        let mut wallets = Vec::new();
        for path in paths {
            let snapshot = source::Snapshot::copy(&path)?;
            let tables = snapshot.load(&o.sqlite)?;
            let version = source::version(&tables)?;
            let rows = convert::rows(&tables, "wallet");
            ensure!(
                rows.len() == 1,
                "expected exactly one wallet per SQLite file"
            );
            let row = &rows[0];
            let id = row.get("wallet_id").context("wallet ID missing")?.text()?;
            quetzal_primitives::WalletId::from_hex(id)?;
            wallets.push(json!({"source":path,"schema_version":version,"wallet_id":id,"name":row.get("name").context("wallet name missing")?.text()?,"encrypted_credentials_present":!convert::rows(&tables,"private_key").is_empty(),"table_rows":tables.iter().map(|(name,rows)|(name.clone(),rows.len())).collect::<std::collections::BTreeMap<_,_>>(),"source_files":snapshot.fingerprints}));
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"format_version":1,"mode":"read-only-inspection","destination_created":false,"wallets":wallets})
            )?
        );
        return Ok(());
    }
    let target = o
        .destination
        .as_ref()
        .map(|path| publish::destination(&o.source, path))
        .transpose()?;
    let (magic, start, genesis_hash) = if o.mainnet {
        (
            764824073,
            1506203091,
            "5f20df933584822601f9e3f8c024eb5eb252fe8cefb24d1317dc3d432e940ebb".to_owned(),
        )
    } else {
        let genesis: serde_json::Value =
            serde_json::from_slice(&fs::read(o.genesis.as_ref().context("genesis missing")?)?)?;
        (
            genesis
                .pointer("/protocolConsts/protocolMagic")
                .and_then(serde_json::Value::as_u64)
                .context("invalid Byron protocol magic")?,
            genesis["startTime"]
                .as_u64()
                .context("invalid Byron genesis start")?,
            hex::encode(pallas_crypto::hash::Hasher::<256>::hash(
                &serde_json::to_vec(&genesis)?,
            )),
        )
    };
    let socket = o.socket.as_deref().context("node socket missing")?;
    let runtime = tokio::runtime::Runtime::new()?;
    let native = runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(30),
            quetzal_network::NodeClient::network_snapshot(socket, magic),
        )
        .await
    })??;
    let clock = convert::Clock {
        genesis_start: start,
        eras: native.timings,
    };
    let mut inputs = Vec::new();
    let mut receipts = Vec::new();
    let mut seen = BTreeSet::new();
    for path in paths {
        let snapshot = source::Snapshot::copy(&path)?;
        let tables = snapshot.load(&o.sqlite)?;
        source::version(&tables)?;
        let input = convert::wallet(
            &tables,
            path.file_name()
                .and_then(|n| n.to_str())
                .context("source filename is not UTF-8")?,
            &clock,
            &genesis_hash,
            if o.mainnet {
                pallas_addresses::Network::Mainnet
            } else {
                pallas_addresses::Network::Testnet
            },
        )?;
        let canonical = runtime.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(30),
                quetzal_network::NodeClient::validate_checkpoint(socket, magic, &input.checkpoint),
            )
            .await
        })??;
        ensure!(canonical,"source checkpoint is not on the selected node's chain; resume the reference wallet on that node before migration");
        let family = if matches!(
            input.metadata.wallet_type,
            WalletType::Byron | WalletType::Icarus
        ) {
            "byron"
        } else {
            "modern"
        };
        ensure!(
            seen.insert((family, input.metadata.id.to_hex())),
            "duplicate wallet identity in the same API family"
        );
        receipts.push(json!({"source":path,"source_files":snapshot.fingerprints,"schema_version":5,"wallet_id":input.metadata.id.to_hex(),"wallet_type":input.metadata.wallet_type,"addresses":input.addresses.len(),"utxos":input.utxos.len(),"history":input.history.len(),"pending":input.pending.len(),"local_submissions":input.submissions.len(),"credentials_preserved":input.credentials.is_some(),"checkpoint_slot":input.checkpoint.slot,"checkpoint_hash":hex::encode(input.checkpoint.block_hash),"native_checkpoint_verified":true}));
        inputs.push(input);
    }
    let mut builder = tempfile::Builder::new();
    builder.prefix(".quetzal-migration-");
    let stage = match &target {
        Some(target) => {
            builder.tempdir_in(target.parent().context("destination parent missing")?)?
        }
        None => builder.tempdir()?,
    };
    {
        let modern = WalletDb::open(
            stage
                .path()
                .to_str()
                .context("destination path is not UTF-8")?,
        )?;
        let byron = WalletDb::open(
            stage
                .path()
                .join("byron")
                .to_str()
                .context("destination path is not UTF-8")?,
        )?;
        for input in &inputs {
            let db = if matches!(
                input.metadata.wallet_type,
                WalletType::Byron | WalletType::Icarus
            ) {
                &byron
            } else {
                &modern
            };
            db.import_wallet(input).with_context(|| {
                format!(
                    "importing {:?} wallet {}",
                    input.metadata.wallet_type, input.metadata.id
                )
            })?;
        }
        modern.flush()?;
        byron.flush()?;
    }
    {
        let modern = WalletDb::open(
            stage
                .path()
                .to_str()
                .context("destination path is not UTF-8")?,
        )?;
        let byron = WalletDb::open(
            stage
                .path()
                .join("byron")
                .to_str()
                .context("destination path is not UTF-8")?,
        )?;
        for input in &inputs {
            let db = if matches!(
                input.metadata.wallet_type,
                WalletType::Byron | WalletType::Icarus
            ) {
                &byron
            } else {
                &modern
            };
            verify(db, input)?;
        }
    }
    let receipt = json!({"format_version":1,"mode":"complete-state-import","source":o.source.canonicalize()?,"network_magic":magic,"genesis_hash":genesis_hash,"wallets":receipts,"same_password_preserved":true,"private_keys_decrypted":false,"destination_reopened_and_verified":true,"rollback_before_import_boundary":"reset-derived-state-and-rescan-retaining-local-state"});
    let receipt_path = stage.path().join("migration-receipt.json");
    fs::write(&receipt_path, serde_json::to_vec_pretty(&receipt)?)?;
    fs::File::open(&receipt_path)?.sync_all()?;
    fs::File::open(stage.path())?.sync_all()?;
    let Some(target) = target else {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"mode":"complete-state-check","destination_created":false,"receipt":receipt})
            )?
        );
        return Ok(());
    };
    publish::publish(stage.path(), &target)?;
    println!(
        "Imported {} wallets into {}\nReceipt: {}",
        inputs.len(),
        target.display(),
        target.join("migration-receipt.json").display()
    );
    Ok(())
}
fn verify(db: &WalletDb, input: &ImportedWallet) -> Result<()> {
    let id = &input.metadata.id;
    let actual = db.get_wallet(id)?.context("reopened wallet missing")?;
    ensure!(
        quetzal_primitives::cbor_encode(&actual)?
            == quetzal_primitives::cbor_encode(&input.metadata)?,
        "reopened wallet metadata differs"
    );
    ensure!(
        db.get_account_xpub(id)? == Some(input.account_public_key),
        "reopened account key differs"
    );
    let actual = db.load_encrypted_key(id)?;
    ensure!(
        actual
            .as_ref()
            .map(quetzal_primitives::cbor_encode)
            .transpose()?
            == input
                .credentials
                .as_ref()
                .map(quetzal_primitives::cbor_encode)
                .transpose()?,
        "reopened credentials differ"
    );
    fn encoded<T: serde::Serialize>(values: &[T]) -> Result<Vec<Vec<u8>>> {
        let mut rows = values
            .iter()
            .map(quetzal_primitives::cbor_encode)
            .collect::<Result<Vec<_>, _>>()?;
        rows.sort();
        Ok(rows)
    }
    ensure!(
        encoded(&db.get_addresses(id)?)? == encoded(&input.addresses)?,
        "reopened addresses differ"
    );
    ensure!(
        encoded(&db.get_utxos(id)?)? == encoded(&input.utxos)?,
        "reopened UTxOs differ"
    );
    ensure!(
        encoded(&db.list_pending(id)?)? == encoded(&input.pending)?,
        "reopened pending submissions differ"
    );
    ensure!(
        encoded(&db.get_txs(id, None, usize::MAX)?)? == encoded(&input.history)?,
        "reopened history differs"
    );
    ensure!(
        db.get_policy_xpub(id)? == input.policy_public_key,
        "reopened policy key differs"
    );
    for (key, expected) in [
        (b"payment_script".as_slice(), &input.payment_script),
        (b"delegation_script".as_slice(), &input.delegation_script),
    ] {
        let actual = db
            .get_script_template(id, key)?
            .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes))
            .transpose()?;
        ensure!(&actual == expected, "reopened script template differs");
    }
    ensure!(
        db.latest_checkpoint(id)? == Some(input.checkpoint.clone()),
        "reopened checkpoint differs"
    );
    Ok(())
}
fn help() {
    println!("Quetzal wallet-state migration\n\nUsage:\n  quetzal-migrate inspect --source FILE-OR-DIRECTORY [--sqlite3 PATH]\n  quetzal-migrate check --source FILE-OR-DIRECTORY\n    --node-socket SOCKET (--mainnet | --testnet BYRON-GENESIS) [--sqlite3 PATH]\n  quetzal-migrate import --source FILE-OR-DIRECTORY --destination NEW-DIRECTORY\n    --node-socket SOCKET (--mainnet | --testnet BYRON-GENESIS) [--sqlite3 PATH]\n\nStop cardano-wallet first. Original SQLite files (including WAL) are copied into\na private snapshot and never opened for writing. Import preserves encrypted\ncredentials and requires no spending passwords. A connected matching node is\nrequired to verify checkpoints and historical era timing. Check performs the full\nconversion and sled reopening verification in temporary storage without publishing.\nExisting destinations are never overwritten or merged. Only SQLite schema\nversion 5 is supported.");
}
