//! Real reference SQLite -> typed sled -> public wallet/native-chain verification.
use anyhow::{ensure, Context, Result};
use reqwest::{blocking::Client, Method};
use serde_json::{json, Value};
use std::{
    fs,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};
use wallet_conformance::{
    cluster_control::Cluster,
    node::NodeOracle,
    owned_process::{Launch, WalletProcess},
};
const ROOT: &str = "/tmp/qz-sqlite-migration-20261116-10";
const FAUCET: &str = "/home/sam/work/se7en/quetzal/master/target/debug/quetzal-faucet";
// Keep migration binaries pinned independently of builds in sibling worktrees.
const CANDIDATE: &str = "/home/sam/work/se7en/quetzal/restart-context/20261116-funded-discovery/history-inputs-release-package/bin/quetzal";
const MIGRATOR: &str = "/home/sam/work/se7en/quetzal/restart-context/20261116-funded-discovery/history-inputs-release-package/bin/quetzal-migrate";
const PASSWORD: &str = "public-native-migration-password";
const TIMEOUT: Duration = Duration::from_secs(180);
const REFERENCE: &str = "http://127.0.0.1:19420/v2";
const TARGET: &str = "http://127.0.0.1:19419/v2";
struct FaucetProcess(Child);
impl Drop for FaucetProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start_faucet(c: &Client, config: &Value, root: &Path) -> Result<FaucetProcess> {
    let funds = PathBuf::from(string(config, "/base_configs")?).join("faucet-addrs");
    // Use untouched test genesis funding sources; address/key material is input
    // to our Pallas service, not a call to the reference faucet or wallet API.
    let mut sources = Vec::new();
    for index in 10..30 {
        let raw = wallet_conformance::encoding::base58_decode(
            fs::read_to_string(funds.join(format!("faucet{index}.addr")))?.trim(),
        )?;
        sources.push(json!({
            "address": wallet_conformance::encoding::bech32_encode("addr_test", &raw)?,
            "signing_key_file": funds.join(format!("faucet{index}.shelley.key")),
        }));
    }
    let path = root.join("faucet-config.json");
    fs::write(
        &path,
        serde_json::to_vec_pretty(&json!({
            "node_socket": root.join("runtime/node.socket"), "network_magic":42,
            "listen":"127.0.0.1:19494", "state_directory":root.join("faucet-reservations"),
            "max_lovelace":1_000_000_000u64, "sources":sources,
        }))?,
    )?;
    let mut faucet = FaucetProcess(
        Command::new(FAUCET)
            .arg("--config")
            .arg(&path)
            .stdout(fs::File::create(root.join("faucet.stdout"))?)
            .stderr(fs::File::create(root.join("faucet.stderr"))?)
            .spawn()?,
    );
    wait(
        || {
            ensure!(
                faucet.0.try_wait()?.is_none(),
                "Quetzal faucet exited; inspect private faucet.stderr"
            );
            Ok(call(c, "http://127.0.0.1:19494", "GET", "/health", None, 200).is_ok())
        },
        "Quetzal faucet listener",
    )?;
    Ok(faucet)
}
struct Candidate {
    child: Child,
    input: Option<ChildStdin>,
}
impl Candidate {
    fn start(launch: Launch, logs: &Path) -> Result<Self> {
        fs::create_dir(logs)?;
        let out = fs::File::create(logs.join("stdout"))?;
        let err = fs::File::create(logs.join("stderr"))?;
        let mut child = Command::new(launch.executable)
            .args(launch.arguments)
            .envs(launch.environment)
            .stdin(Stdio::piped())
            .stdout(out)
            .stderr(err)
            .spawn()?;
        let input = child.stdin.take();
        let c = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()?;
        wait(
            || {
                ensure!(
                    child.try_wait()?.is_none(),
                    "candidate exited during startup"
                );
                Ok(call(&c, TARGET, "GET", "/wallets", None, 200).is_ok())
            },
            "candidate listener",
        )?;
        Ok(Self { child, input })
    }
    fn stop(&mut self) -> Result<()> {
        self.input.take();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(status.success(), "candidate EOF failed: {status}");
                break;
            }
            ensure!(Instant::now() < deadline, "candidate EOF timed out");
            sleep(Duration::from_millis(10));
        }
        ensure!(
            std::net::TcpStream::connect(("127.0.0.1", 19419)).is_err(),
            "API survived EOF"
        );
        Ok(())
    }
}
impl Drop for Candidate {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
fn string(v: &Value, p: &str) -> Result<String> {
    Ok(v.pointer(p)
        .and_then(Value::as_str)
        .with_context(|| format!("missing {p}"))?
        .into())
}
fn call(
    c: &Client,
    base: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    status: u16,
) -> Result<Value> {
    let mut request = c.request(
        Method::from_bytes(method.as_bytes())?,
        format!("{base}{path}"),
    );
    if let Some(body) = body {
        request = request
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body)?);
    }
    let response = request.send()?;
    let actual = response.status().as_u16();
    let bytes = response.bytes()?;
    ensure!(
        actual == status,
        "{method} {path}: expected {status}, got {actual}: {}",
        String::from_utf8_lossy(&bytes)
    );
    if bytes.is_empty() {
        Ok(Value::Null)
    } else {
        Ok(serde_json::from_slice(&bytes)?)
    }
}
fn wait(mut check: impl FnMut() -> Result<bool>, label: &str) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "timeout: {label}");
        sleep(Duration::from_millis(100));
    }
}
fn launch(executable: PathBuf, root: &Path, port: u16, database: Option<&Path>) -> Launch {
    let mut arguments = vec![
        "serve".into(),
        "--database".into(),
        database
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "{database}".into()),
        "--node-socket".into(),
        root.join("runtime/node.socket")
            .to_string_lossy()
            .into_owned(),
        "--testnet".into(),
        root.join("runtime/cluster/byron-genesis.json")
            .to_string_lossy()
            .into_owned(),
        "--port".into(),
        port.to_string(),
        "--ui-port".into(),
        (port + 100).to_string(),
        "--pool-metadata-fetching".into(),
        "none".into(),
        "--shutdown-handler".into(),
    ];
    if port == 19420 {
        arguments.extend(["+RTS".into(), "-N2".into(), "-RTS".into()]);
    }
    Launch {
        executable,
        arguments,
        environment: Default::default(),
    }
}
fn create(
    c: &Client,
    family: &str,
    name: &str,
    words: Vec<String>,
    style: Option<&str>,
) -> Result<String> {
    let mut body = json!({"name":name,"mnemonic_sentence":words,"passphrase":PASSWORD});
    if let Some(style) = style {
        body["style"] = json!(style);
    }
    if family == "shared-wallets" {
        body["account_index"] = json!("0H");
        let template = json!({"cosigners":{"cosigner#0":"self"},"template":{"all":["cosigner#0"]}});
        body["payment_script_template"] = template.clone();
        body["delegation_script_template"] = template;
    }
    let value = call(c, REFERENCE, "POST", &format!("/{family}"), Some(body), 201)?;
    Ok(format!("/{family}/{}", string(&value, "/id")?))
}
fn address(c: &Client, path: &str, random: bool) -> Result<String> {
    if random {
        return string(
            &call(
                c,
                REFERENCE,
                "POST",
                &format!("{path}/addresses"),
                Some(json!({"passphrase":PASSWORD,"address_index":2147495993u64})),
                201,
            )?,
            "/id",
        );
    }
    string(
        &call(c, REFERENCE, "GET", &format!("{path}/addresses"), None, 200)?,
        "/0/id",
    )
}
fn ready(c: &Client, base: &str, path: &str, coin: u64) -> Result<()> {
    wait(
        || {
            let w = call(c, base, "GET", path, None, 200)?;
            Ok(w["state"]["status"] == "ready" && w["balance"]["available"]["quantity"] == coin)
        },
        "ready balance",
    )
}
fn payment(c: &Client, base: &str, path: &str, to: &str) -> Result<String> {
    string(
        &call(
            c,
            base,
            "POST",
            &format!("{path}/transactions"),
            Some(
                json!({"payments":[{"address":to,"amount":{"quantity":2_000_000,"unit":"lovelace"}}],"passphrase":PASSWORD,"time_to_live":{"quantity":600,"unit":"second"},"metadata":{"42":{"string":"migration-preserved-metadata"}}}),
            ),
            202,
        )?,
        "/id",
    )
}
fn ordered(mut value: Value) -> Value {
    if let Some(a) = value.as_array_mut() {
        a.sort_by_key(|x| x["id"].to_string());
    }
    value
}
fn snapshot(c: &Client, base: &str, path: &str) -> Result<Value> {
    let wallet = call(c, base, "GET", path, None, 200)?;
    let selected = [
        "id",
        "name",
        "address_pool_gap",
        "balance",
        "assets",
        "delegation",
        "passphrase",
    ];
    let mut meta = serde_json::Map::new();
    for field in selected {
        if let Some(value) = wallet.get(field) {
            meta.insert(field.into(), value.clone());
        }
    }
    let mut history = call(c, base, "GET", &format!("{path}/transactions"), None, 200)?;
    // All producers are paused, so timing, depths, statuses, amounts and full
    // transaction fields can be compared directly against the stopped reference.
    if let Some(txs) = history.as_array_mut() {
        txs.sort_by_key(|t| t["id"].to_string());
    }
    Ok(
        json!({"wallet":meta,"addresses":ordered(call(c,base,"GET",&format!("{path}/addresses"),None,200)?),"history":history,"utxo":call(c,base,"GET",&format!("{path}/utxo"),None,200)?}),
    )
}
fn main() -> Result<()> {
    let config:Value=serde_json::from_slice(&fs::read("/home/sam/work/se7en/quetzal/master/evidence/20261110-replacement/full-execution-config.json")?)?;
    let root = Path::new(ROOT);
    fs::DirBuilder::new().mode(0o700).create(root)?;
    let runtime = root.join("runtime");
    let mut factory: Launch = serde_json::from_value(config["cluster_launch"].clone())?;
    for arg in &mut factory.arguments {
        if arg == "{cluster_configs}" {
            *arg = string(&config, "/base_configs")?
        }
        if arg == "19293" {
            *arg = "19493".into();
        }
    }
    let mut cluster = Cluster::start(factory, &runtime)?;
    cluster.discover(
        Path::new(&string(&config, "/cardano_node")?),
        &runtime,
        TIMEOUT,
    )?;
    let node = NodeOracle {
        executable: string(&config, "/cardano_cli")?.into(),
        socket: runtime.join("node.socket"),
        network_magic: 42,
    };
    node.advancing(0, TIMEOUT)?;
    let (other_producers, _) = cluster.pause(TIMEOUT)?.resume_one(TIMEOUT)?;
    let c = Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()?;
    let _faucet = start_faucet(&c, &config, root)?;
    let mut reference = WalletProcess::start(
        launch(
            string(&config, "/reference_launch/executable")?.into(),
            root,
            19420,
            None,
        ),
        &root.join("reference"),
    )?;
    reference.wait_listener(&format!("{REFERENCE}/"), TIMEOUT)?;
    let words = bip39::Mnemonic::generate(15)?
        .words()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let shelley = create(&c, "wallets", "Migration Shelley", words.clone(), None)?;
    let icarus = create(
        &c,
        "byron-wallets",
        "Migration Icarus",
        words.clone(),
        Some("icarus"),
    )?;
    let random = create(
        &c,
        "byron-wallets",
        "Migration random",
        bip39::Mnemonic::generate(12)?
            .words()
            .map(str::to_owned)
            .collect(),
        Some("random"),
    )?;
    let shared = create(&c, "shared-wallets", "Migration shared", words, None)?;
    ensure!(
        shelley.rsplit('/').next() == icarus.rsplit('/').next(),
        "expected same-ID cross-family fixture"
    );
    let key = call(
        &c,
        REFERENCE,
        "GET",
        &format!("{shelley}/keys?format=extended"),
        None,
        200,
    )?;
    let (_, raw) =
        wallet_conformance::encoding::bech32(key.as_str().context("account key missing")?)?;
    let watch = call(
        &c,
        REFERENCE,
        "POST",
        "/wallets",
        Some(json!({"name":"Migration watch","account_public_key":hex::encode(&raw)})),
        201,
    )?;
    let watch = format!("/wallets/{}", string(&watch, "/id")?);
    let paths = vec![
        shelley.clone(),
        icarus.clone(),
        random.clone(),
        shared.clone(),
        watch.clone(),
    ];
    let mut destinations = Vec::new();
    for path in &paths[..4] {
        destinations.push(address(&c, path, *path == random)?);
    }
    for path in &paths {
        ready(&c, REFERENCE, path, 0)?;
    }
    let mut funding = Vec::new();
    for address in &destinations {
        funding.push(call(
            &c,
            "http://127.0.0.1:19494",
            "POST",
            "/fund",
            Some(json!({"payments":[{"address":address,"lovelace":30_000_000}]})),
            202,
        )?);
    }
    for path in &paths {
        ready(&c, REFERENCE, path, 30_000_000)?;
    }
    fs::write(
        root.join("faucet-funding-receipts.json"),
        serde_json::to_vec_pretty(&funding)?,
    )?;
    println!("Quetzal Pallas faucet funded all five wallet families");
    let confirmed = payment(&c, REFERENCE, &shelley, &destinations[1])?;
    wait(
        || {
            Ok(call(
                &c,
                REFERENCE,
                "GET",
                &format!("{shelley}/transactions/{confirmed}"),
                None,
                200,
            )?["status"]
                == "in_ledger")
        },
        "confirmed history",
    )?;
    let paused = cluster.pause(TIMEOUT)?;
    let tip = node.tip(TIMEOUT)?;
    // Wait for each source wallet to catch the paused native point before capture.
    for path in &paths {
        wait(
            || {
                let w = call(&c, REFERENCE, "GET", path, None, 200)?;
                Ok(w["tip"]["absolute_slot_number"] == tip.slot)
            },
            "source paused-tip catchup",
        )?;
    }
    let pending = vec![
        payment(&c, REFERENCE, &shelley, &destinations[2])?,
        payment(&c, REFERENCE, &icarus, &destinations[2])?,
    ];
    for (path, name) in [
        (&shelley, "latest acknowledged Shelley"),
        (&icarus, "latest acknowledged Icarus"),
    ] {
        call(&c, REFERENCE, "PUT", path, Some(json!({"name":name})), 200)?;
    }
    let mut before = Vec::new();
    for path in &paths {
        before.push(snapshot(&c, REFERENCE, path)?);
    }
    fs::write(
        root.join("reference-snapshots.json"),
        serde_json::to_vec_pretty(&json!({"paths":&paths,"snapshots":&before}))?,
    )?;
    reference.stop()?;
    ensure!(
        node.tip(TIMEOUT)?.hash == tip.hash,
        "native tip moved during capture"
    );
    let target = root.join("migrated");
    let source = root.join("reference/database");
    let output = Command::new(MIGRATOR)
        .args(["import", "--source"])
        .arg(&source)
        .arg("--destination")
        .arg(&target)
        .arg("--node-socket")
        .arg(&node.socket)
        .arg("--testnet")
        .arg(runtime.join("cluster/byron-genesis.json"))
        .args([
            "--sqlite3",
            "/nix/store/hfi7wfmjsap0l5jzjgphz4m76w3s9sm4-sqlite-3.50.4-bin/bin/sqlite3",
        ])
        .output()?;
    fs::write(root.join("migration.stdout"), &output.stdout)?;
    fs::write(root.join("migration.stderr"), &output.stderr)?;
    ensure!(
        output.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: Value = serde_json::from_slice(&fs::read(target.join("migration-receipt.json"))?)?;
    let mut candidate = Candidate::start(
        launch(CANDIDATE.into(), root, 19419, Some(&target)),
        &root.join("candidate"),
    )?;
    wait(
        || Ok(call(&c, TARGET, "GET", "/wallets", None, 200).is_ok()),
        "target API readiness",
    )?;
    let mut after = Vec::new();
    let mut differences = Vec::new();
    for (path, expected) in paths.iter().zip(&before) {
        let actual = snapshot(&c, TARGET, path)?;
        if &actual != expected {
            fs::write(
                root.join("difference.json"),
                serde_json::to_vec_pretty(
                    &json!({"path":path,"expected":expected,"actual":actual}),
                )?,
            )?;
            differences.push(json!({"path":path,"expected":expected,"actual":&actual}));
        }
        after.push(actual);
    }
    if !differences.is_empty() {
        fs::write(
            root.join("differences.json"),
            serde_json::to_vec_pretty(&differences)?,
        )?;
        anyhow::bail!("public migrated state differs for {} wallet families; private difference receipts saved", differences.len());
    }
    println!("all five complete public snapshots match the stopped reference");
    candidate.stop()?;
    let mut restarted = Candidate::start(
        launch(CANDIDATE.into(), root, 19419, Some(&target)),
        &root.join("restart"),
    )?;
    for (path, expected) in paths.iter().zip(&before) {
        ensure!(
            snapshot(&c, TARGET, path)? == *expected,
            "state differs after same-DB restart"
        );
    }
    println!("all five complete public snapshots match after same-database restart");
    let account_after = call(
        &c,
        TARGET,
        "GET",
        &format!("{shelley}/keys?format=extended"),
        None,
        200,
    )?;
    ensure!(account_after == key, "account key changed");
    let wrong = call(
        &c,
        TARGET,
        "PUT",
        &format!("{shelley}/passphrase"),
        Some(
            json!({"old_passphrase":"incorrect-migration-password","new_passphrase":"replacement-migration-password"}),
        ),
        403,
    )?;
    ensure!(
        wrong["code"] == "wrong_encryption_passphrase",
        "wrong password unexpectedly accepted"
    );
    drop(paused);
    node.advancing(tip.height, TIMEOUT)?;
    for (path, id) in [(&shelley, &pending[0]), (&icarus, &pending[1])] {
        wait(
            || {
                Ok(call(
                    &c,
                    TARGET,
                    "GET",
                    &format!("{path}/transactions/{id}"),
                    None,
                    200,
                )?["status"]
                    == "in_ledger")
            },
            "original pending confirmation after import",
        )?;
        node.payment(id, &destinations[2], 2_000_000, TIMEOUT)?;
        println!("original pending transaction independently confirmed: {id}");
    }
    let sent = payment(&c, TARGET, &icarus, &destinations[0])?;
    // HTTP acceptance precedes ledger inclusion. Wait only for absence, then
    // enforce the same exact node address/amount assertion used above.
    wait(
        || {
            let utxos = node.utxos(&destinations[0], TIMEOUT)?;
            Ok(utxos
                .as_object()
                .context("node UTxO result is not an object")?
                .keys()
                .any(|key| {
                    key.strip_prefix(&sent)
                        .is_some_and(|suffix| suffix.starts_with('#'))
                }))
        },
        "new post-migration payment inclusion",
    )?;
    node.payment(&sent, &destinations[0], 2_000_000, TIMEOUT)?;
    println!("new post-migration spending independently confirmed: {sent}");
    restarted.stop()?;
    drop(other_producers);
    cluster.stop()?;
    cluster.wait_stopped_nodes(Duration::from_secs(10))?;
    fs::write(
        "evidence/20261110-sqlite-migration/native-result.json",
        serde_json::to_vec_pretty(
            &json!({"passed":true,"families":["Shelley","Icarus","random Byron","shared","watch-only"],"same_id_across_families":true,"reference_sqlite_created_by_pinned_wallet":true,"source_metadata_latest_acknowledged":true,"complete_public_snapshot_equal":true,"same_database_restart_equal":true,"wrong_password_rejected":true,"account_public_key_preserved":true,"original_pending_ids":pending,"original_pending_independently_confirmed":true,"post_migration_spending_independently_confirmed":sent,"no_submission_api_call_for_original_pending_after_import":true,"owned_native_cleanup":true,"migration_receipt":receipt}),
        )?,
    )?;
    println!("native migration verification passed");
    Ok(())
}
