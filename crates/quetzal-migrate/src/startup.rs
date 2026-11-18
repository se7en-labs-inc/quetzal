//! Resolve persistent startup state without creating an empty store over SQLite.
use crate::{execute, MigrationNetwork, Options};
use anyhow::{bail, ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

const MANAGED: &str = "quetzal.sled";

pub struct PreparedDatabase {
    pub path: PathBuf,
    pub imported_wallets: Option<usize>,
}

enum Layout {
    Direct(PathBuf),
    Managed(PathBuf),
    Sqlite { source: PathBuf, target: PathBuf },
}

// Reject symlinks instead of following them during layout selection.
fn metadata(path: &Path) -> Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            ensure!(
                !meta.file_type().is_symlink(),
                "database layout must not contain symlinks: {}",
                path.display()
            );
            Ok(Some(meta))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn discover(directory: &Path, network: &MigrationNetwork) -> Result<Layout> {
    let Some(meta) = metadata(directory)? else {
        return Ok(Layout::Direct(directory.to_owned()));
    };
    ensure!(meta.is_dir(), "--database must name a directory");
    let directory = directory.canonicalize()?;
    let direct = metadata(&directory.join("conf"))?.is_some();
    let target = directory.join(MANAGED);
    if let Some(meta) = metadata(&target)? {
        ensure!(
            !direct,
            "conflicting direct and managed sled databases; keep one authoritative database layout"
        );
        ensure!(
            meta.is_dir(),
            "managed sled destination must be a directory"
        );
        ensure!(metadata(&target.join("conf"))?.is_some_and(|m| m.is_file()), "managed sled database is incomplete: missing conf; restore the completed store or remove the incomplete destination before retrying migration");
        let receipt_path = target.join("migration-receipt.json");
        ensure!(
            metadata(&receipt_path)?.is_some_and(|m| m.is_file()),
            "managed sled database is missing its migration receipt"
        );
        let receipt: serde_json::Value = serde_json::from_slice(&fs::read(receipt_path)?)
            .context("invalid startup migration receipt")?;
        ensure!(
            receipt["format_version"] == 1
                && receipt["mode"] == "complete-state-import"
                && receipt["destination_reopened_and_verified"] == true
                && receipt["wallets"].as_array().is_some_and(|w| !w.is_empty()),
            "incomplete startup migration receipt"
        );
        ensure!(
            receipt["network_magic"] == network.magic
                && receipt["genesis_hash"] == network.genesis_hash,
            "migrated database belongs to a different network"
        );
        return Ok(Layout::Managed(target));
    }
    let mut sqlite = false;
    for entry in fs::read_dir(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name != "stake-pools.sqlite" && name.ends_with(".sqlite") {
            ensure!(
                entry.file_type()?.is_file(),
                "wallet SQLite source must be a regular file"
            );
            sqlite = true;
        }
    }
    ensure!(!(direct && sqlite), "database contains both direct sled and SQLite wallet state; resolve the authoritative store before startup (SQLite files have not been changed)");
    if sqlite {
        Ok(Layout::Sqlite {
            source: directory,
            target,
        })
    } else {
        Ok(Layout::Direct(directory))
    }
}

/// Startup conversion is synchronous; async callers must use a blocking thread.
/// The source backend must be stopped, and the matching node must be running.
pub fn prepare_database(
    directory: &Path,
    socket: &str,
    network: MigrationNetwork,
) -> Result<PreparedDatabase> {
    match discover(directory, &network)? {
        Layout::Direct(path) | Layout::Managed(path) => Ok(PreparedDatabase {
            path,
            imported_wallets: None,
        }),
        Layout::Sqlite { source, target } => {
            let mainnet = network.mainnet;
            let count = execute(Options {
                command: "import".into(),
                source,
                destination: Some(target.clone()),
                sqlite: PathBuf::from(option_env!("QUETZAL_SQLITE3").unwrap_or("sqlite3")),
                socket: Some(socket.into()),
                genesis: None,
                mainnet,
            }, Some(network), true)
                .context("automatic SQLite wallet migration failed; startup stopped without creating an empty wallet database")?;
            Ok(PreparedDatabase {
                path: target,
                imported_wallets: Some(count),
            })
        }
    }
}

/// Only automatic startup may publish this exact child inside its SQLite source.
pub(crate) fn destination(source: &Path, target: &Path) -> Result<PathBuf> {
    let source = source.canonicalize()?;
    ensure!(
        source.is_dir() && target == source.join(MANAGED),
        "invalid managed migration destination"
    );
    if metadata(target)?.is_some() {
        bail!("startup migration destination already exists; no data was overwritten");
    }
    Ok(target.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn network() -> MigrationNetwork {
        MigrationNetwork {
            magic: 2,
            genesis_start: 0,
            genesis_hash: "preview-test".into(),
            mainnet: false,
        }
    }

    #[test]
    fn fresh_and_existing_direct_sled_and_pool_cache_need_no_node() {
        let root = tempfile::tempdir().unwrap();
        let absent = root.path().join("new");
        assert_eq!(
            prepare_database(&absent, "absent", network()).unwrap().path,
            absent
        );
        assert!(!absent.exists());
        let direct = root.path().join("direct");
        drop(quetzal_wallet::db::WalletDb::open(direct.to_str().unwrap()).unwrap());
        fs::write(direct.join("stake-pools.sqlite"), b"pool cache only").unwrap();
        assert_eq!(
            prepare_database(&direct, "absent", network()).unwrap().path,
            direct
        );
    }

    #[test]
    fn sqlite_selected_and_failure_never_creates_sled_or_changes_source() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("wallet.sqlite");
        fs::write(&source, b"unsupported source").unwrap();
        assert!(matches!(
            discover(root.path(), &network()).unwrap(),
            Layout::Sqlite { .. }
        ));
        assert!(prepare_database(root.path(), "absent", network()).is_err());
        assert_eq!(fs::read(source).unwrap(), b"unsupported source");
        assert!(!root.path().join("conf").exists());
        assert!(!root.path().join(MANAGED).exists());
        // An abandoned private stage is never interpreted as a published store.
        fs::create_dir(root.path().join(".quetzal-migration-interrupted")).unwrap();
        assert!(matches!(
            discover(root.path(), &network()).unwrap(),
            Layout::Sqlite { .. }
        ));
    }

    #[test]
    fn completed_managed_store_is_reused_without_sqlite_or_node_and_checks_network() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join(MANAGED);
        let db = quetzal_wallet::db::WalletDb::open(target.to_str().unwrap()).unwrap();
        db.flush().unwrap();
        drop(db);
        let receipt = serde_json::json!({"format_version":1,"mode":"complete-state-import",
            "network_magic":2,"genesis_hash":"preview-test","destination_reopened_and_verified":true,"wallets":[{}]});
        fs::write(
            target.join("migration-receipt.json"),
            serde_json::to_vec(&receipt).unwrap(),
        )
        .unwrap();
        let prepared = prepare_database(root.path(), "absent", network()).unwrap();
        assert_eq!(prepared.path, target);
        assert_eq!(prepared.imported_wallets, None);
        let mut wrong = network();
        wrong.magic = 1;
        assert!(prepare_database(root.path(), "absent", wrong).is_err());
        // Reuse must not parse now stale or corrupt SQLite sources.
        fs::write(root.path().join("old.sqlite"), b"corrupt").unwrap();
        assert!(prepare_database(root.path(), "absent", network()).is_ok());
    }

    #[test]
    fn incomplete_conflicting_or_redirected_layouts_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join(MANAGED)).unwrap();
        assert!(prepare_database(root.path(), "absent", network()).is_err());
        fs::remove_dir(root.path().join(MANAGED)).unwrap();
        fs::write(root.path().join("conf"), b"direct sled").unwrap();
        fs::write(root.path().join("wallet.sqlite"), b"sqlite").unwrap();
        assert!(prepare_database(root.path(), "absent", network()).is_err());
        fs::remove_file(root.path().join("conf")).unwrap();
        fs::remove_file(root.path().join("wallet.sqlite")).unwrap();
        std::os::unix::fs::symlink("elsewhere", root.path().join("wallet.sqlite")).unwrap();
        assert!(prepare_database(root.path(), "absent", network()).is_err());
    }
}
