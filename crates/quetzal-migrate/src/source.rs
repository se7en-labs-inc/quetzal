//! Read SQLite only on private copies, retaining committed WAL state.
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;
use zeroize::Zeroizing;

#[derive(Clone, Deserialize)]
pub struct Cell {
    pub kind: String,
    pub value: Option<String>,
}
pub type Row = BTreeMap<String, Cell>;
pub type Tables = BTreeMap<String, Vec<Row>>;

impl Cell {
    pub fn text(&self) -> Result<&str> {
        ensure!(self.kind == "text", "expected text field");
        self.value.as_deref().context("missing text field")
    }
    pub fn bytes(&self) -> Result<Vec<u8>> {
        ensure!(self.kind == "blob", "expected binary field");
        Ok(hex::decode(
            self.value.as_deref().context("missing binary field")?,
        )?)
    }
}
#[derive(Clone, Serialize)]
pub struct Fingerprint {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
fn fingerprint(path: &Path) -> Result<Fingerprint> {
    ensure!(
        fs::symlink_metadata(path)?.is_file(),
        "source must be a regular file, not a symlink"
    );
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut bytes = 0;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        bytes += count as u64;
    }
    Ok(Fingerprint {
        name: path
            .file_name()
            .context("source filename missing")?
            .to_string_lossy()
            .into_owned(),
        bytes,
        sha256: hex::encode(hash.finalize()),
    })
}
fn siblings(path: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![path.to_owned()];
    for suffix in ["-wal", "-shm"] {
        let mut p = path.as_os_str().to_owned();
        p.push(suffix);
        let p = PathBuf::from(p);
        if p.try_exists()? {
            paths.push(p);
        }
    }
    Ok(paths)
}
pub struct Snapshot {
    _directory: TempDir,
    pub database: PathBuf,
    pub fingerprints: Vec<Fingerprint>,
}
impl Snapshot {
    pub fn copy(source: &Path) -> Result<Self> {
        ensure!(
            fs::symlink_metadata(source)?.is_file(),
            "source must be a regular file, not a symlink"
        );
        let source = source.canonicalize()?;
        let directory = tempfile::Builder::new()
            .prefix("quetzal-sqlite-snapshot-")
            .tempdir()?;
        let paths = siblings(&source)?;
        let before = paths
            .iter()
            .map(|p| fingerprint(p))
            .collect::<Result<Vec<_>>>()?;
        for path in &paths {
            let target = directory
                .path()
                .join(path.file_name().context("source filename missing")?);
            fs::copy(path, &target)?;
            let actual = fingerprint(&target)?;
            ensure!(
                actual.sha256
                    == before[paths
                        .iter()
                        .position(|p| p == path)
                        .context("source mapping missing")?]
                    .sha256,
                "source changed while copying; stop cardano-wallet and retry"
            );
        }
        ensure!(
            siblings(&source)? == paths,
            "SQLite source files changed; stop cardano-wallet and retry"
        );
        for (path, expected) in paths.iter().zip(&before) {
            let after = fingerprint(path)?;
            ensure!(
                after.sha256 == expected.sha256 && after.bytes == expected.bytes,
                "source changed while copying; stop cardano-wallet and retry"
            );
        }
        let database = directory
            .path()
            .join(source.file_name().context("source filename missing")?);
        Ok(Self {
            _directory: directory,
            database,
            fingerprints: before,
        })
    }
    fn query(&self, sqlite: &Path, query: &str) -> Result<Vec<serde_json::Value>> {
        let output = Command::new(sqlite)
            .args([
                "-init",
                "/dev/null",
                "-batch",
                "-bail",
                "-readonly",
                "-json",
            ])
            .arg(&self.database)
            .arg(format!(
                "PRAGMA trusted_schema=OFF; PRAGMA query_only=ON; {query}"
            ))
            .output()
            .context("cannot run sqlite3; use the Nix development shell or --sqlite3 PATH")?;
        ensure!(
            output.status.success(),
            "SQLite snapshot query failed (source contents omitted)"
        );
        let bytes = Zeroizing::new(output.stdout);
        if bytes.is_empty() {
            return Ok(Vec::new());
        }
        serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!("invalid SQLite JSON output (contents omitted)"))
    }
    pub fn load(&self, sqlite: &Path) -> Result<Tables> {
        let check = self.query(sqlite, "PRAGMA quick_check;")?;
        ensure!(
            check.len() == 1 && check[0]["quick_check"] == "ok",
            "SQLite integrity check failed"
        );
        let names=self.query(sqlite,"SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name;")?;
        let mut tables = Tables::new();
        for entry in names {
            let name = entry["name"]
                .as_str()
                .context("invalid SQLite table name")?;
            let quoted = identifier(name);
            let columns = self.query(sqlite, &format!("PRAGMA table_info({quoted});"))?;
            let fields=columns.iter().map(|c|->Result<String>{
                let name=c["name"].as_str().context("invalid SQLite column name")?;
                let ident=identifier(name); let literal=literal(name);
                Ok(format!("{literal},json_object('kind',typeof({ident}),'value',CASE typeof({ident}) WHEN 'null' THEN NULL WHEN 'blob' THEN hex({ident}) ELSE CAST({ident} AS TEXT) END)"))
            }).collect::<Result<Vec<_>>>()?;
            ensure!(!fields.is_empty(), "SQLite table without columns");
            let rows = self.query(
                sqlite,
                &format!(
                    "SELECT json_object({}) AS row FROM {quoted};",
                    fields.join(",")
                ),
            )?;
            let rows = rows
                .into_iter()
                .map(|r| -> Result<Row> {
                    let row = r["row"].as_str().context("invalid SQLite row encoding")?;
                    serde_json::from_str(row).map_err(|_| {
                        anyhow::anyhow!("invalid SQLite row encoding (contents omitted)")
                    })
                })
                .collect::<Result<Vec<Row>>>()?;
            tables.insert(name.to_owned(), rows);
        }
        Ok(tables)
    }
}
fn identifier(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}
fn literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}
pub fn inventory(source: &Path) -> Result<Vec<PathBuf>> {
    let metadata = fs::symlink_metadata(source)?;
    ensure!(
        !metadata.file_type().is_symlink(),
        "source must not be a symlink"
    );
    if metadata.is_file() {
        return Ok(vec![source.canonicalize()?]);
    }
    ensure!(
        metadata.is_dir(),
        "source must be a SQLite file or database directory"
    );
    let mut files = Vec::new();
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name == "stake-pools.sqlite" || !name.ends_with(".sqlite") {
            continue;
        }
        ensure!(
            entry.file_type()?.is_file(),
            "wallet SQLite source must be a regular file"
        );
        files.push(entry.path().canonicalize()?);
    }
    files.sort();
    if files.is_empty() {
        bail!("no wallet SQLite databases found")
    };
    Ok(files)
}
pub fn version(tables: &Tables) -> Result<u64> {
    let rows = tables
        .get("database_schema_version")
        .context("unsupported unversioned SQLite schema")?;
    let row = rows
        .iter()
        .find(|r| r.get("name").and_then(|c| c.text().ok()) == Some("schema"))
        .context("missing wallet schema version")?;
    let v = row
        .get("version")
        .and_then(|c| c.value.as_deref())
        .context("missing wallet schema version")?
        .parse()?;
    ensure!(
        v == 5,
        "unsupported wallet SQLite schema version {v}; supported version: 5"
    );
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        process::Stdio,
    };
    fn sqlite() -> PathBuf {
        std::env::var_os("QUETZAL_SQLITE3")
            .map(PathBuf::from)
            .unwrap_or_else(|| "sqlite3".into())
    }
    #[test]
    fn committed_wal_and_binary_values_are_copied_without_mutating_source() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("she.test.sqlite");
        let mut child = Command::new(sqlite())
            .args(["-init", "/dev/null", "-batch"])
            .arg(&file)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let mut output = BufReader::new(child.stdout.take().unwrap());
        input.write_all(b"PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0; CREATE TABLE database_schema_version(name TEXT,version INTEGER); INSERT INTO database_schema_version VALUES('schema',5); CREATE TABLE wallet(wallet_id TEXT, payload BLOB, quantity TEXT, optional TEXT); INSERT INTO wallet VALUES('fixture',X'00FF80','18446744073709551615',NULL); SELECT 'committed';\n").unwrap();
        input.flush().unwrap();
        loop {
            let mut line = String::new();
            assert!(output.read_line(&mut line).unwrap() > 0);
            if line.trim() == "committed" {
                break;
            }
        }
        let files = siblings(&file).unwrap();
        assert!(files.iter().any(|p| p.to_string_lossy().ends_with("-wal")));
        let before = files
            .iter()
            .map(|p| fingerprint(p).unwrap().sha256)
            .collect::<Vec<_>>();
        let snapshot = Snapshot::copy(&file).unwrap();
        let tables = snapshot.load(&sqlite()).unwrap();
        assert_eq!(version(&tables).unwrap(), 5);
        let row = &tables["wallet"][0];
        assert_eq!(row["payload"].bytes().unwrap(), [0, 255, 128]);
        assert_eq!(row["quantity"].text().unwrap(), "18446744073709551615");
        assert!(row["optional"].value.is_none());
        let after = files
            .iter()
            .map(|p| fingerprint(p).unwrap().sha256)
            .collect::<Vec<_>>();
        assert_eq!(before, after);
        drop(input);
        assert!(child.wait().unwrap().success());
    }
    #[test]
    fn future_schema_empty_directory_and_symlink_are_rejected() {
        let temp = tempfile::tempdir().unwrap();
        assert!(inventory(temp.path()).is_err());
        let file = temp.path().join("future.sqlite");
        assert!(Command::new(sqlite()).arg(&file).arg("CREATE TABLE database_schema_version(name TEXT,version INTEGER);INSERT INTO database_schema_version VALUES('schema',99);").status().unwrap().success());
        let snapshot = Snapshot::copy(&file).unwrap();
        assert!(version(&snapshot.load(&sqlite()).unwrap()).is_err());
        let link = temp.path().join("link.sqlite");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert!(inventory(&link).is_err());
    }
    #[test]
    fn source_identifiers_are_quoted_and_service_database_is_not_a_wallet() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("wallet.sqlite");
        assert!(Command::new(sqlite()).arg(&file).arg("CREATE TABLE \"quoted'table\"(\"quoted\"\"column\" TEXT); INSERT INTO \"quoted'table\" VALUES('data');").status().unwrap().success());
        fs::write(temp.path().join("stake-pools.sqlite"), b"not a wallet").unwrap();
        assert_eq!(
            inventory(temp.path()).unwrap(),
            vec![file.canonicalize().unwrap()]
        );
        let tables = Snapshot::copy(&file).unwrap().load(&sqlite()).unwrap();
        assert_eq!(
            tables["quoted'table"][0]["quoted\"column"].text().unwrap(),
            "data"
        );
    }
}
