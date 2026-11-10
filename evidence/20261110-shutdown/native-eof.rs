//! Candidate-specific real-node EOF validation; external T110 is still spec-only.
//! Compile against the unchanged conformance library's owned fixture/oracle helpers.
use anyhow::{ensure, Context, Result};
use reqwest::{blocking::Client, Certificate, Identity, Method};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    thread::sleep,
    time::{Duration, Instant},
};
use wallet_conformance::{
    cluster_control::Cluster, node::NodeOracle, owned_process::Launch, transport,
};
const TIMEOUT: Duration = Duration::from_secs(180);
const PASSWORD: &str = "shutdown-test-passphrase";
const PORT: u16 = 19102;

fn text(v: &Value, p: &str) -> Result<String> {
    Ok(v.pointer(p)
        .and_then(Value::as_str)
        .with_context(|| format!("missing {p}"))?
        .into())
}
fn call(
    client: &Client,
    base: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    status: u16,
) -> Result<Value> {
    let mut request = client.request(
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
fn api(
    client: &Client,
    method: &str,
    path: &str,
    body: Option<Value>,
    status: u16,
) -> Result<Value> {
    call(
        client,
        &format!("https://127.0.0.1:{PORT}/v2"),
        method,
        path,
        body,
        status,
    )
}
fn wait(mut check: impl FnMut() -> Result<bool>, label: &str) -> Result<()> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if check()? {
            return Ok(());
        }
        ensure!(Instant::now() < deadline, "timed out: {label}");
        sleep(Duration::from_millis(100));
    }
}
struct Service {
    child: Child,
    input: Option<ChildStdin>,
    log: PathBuf,
}
impl Drop for Service {
    fn drop(&mut self) {
        // Failure cleanup cannot count as successful EOF shutdown.
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}
impl Service {
    fn start(root: &Path, pki: &transport::Pki, label: &str, executable: &Path) -> Result<Self> {
        let reservation = TcpListener::bind(("127.0.0.1", PORT))?;
        drop(reservation);
        let log = root.join(format!("{label}.log"));
        let output = fs::File::create(&log)?;
        let mut child = Command::new(executable)
            .args(["serve", "--shutdown-handler", "--database"])
            .arg(root.join("wallet"))
            .arg("--node-socket")
            .arg(root.join("runtime/node.socket"))
            .arg("--testnet")
            .arg(root.join("runtime/cluster/byron-genesis.json"))
            .args(["--port", &PORT.to_string(), "--tls-ca-cert"])
            .arg(pki.path("ca.crt"))
            .arg("--tls-sv-cert")
            .arg(pki.path("server.crt"))
            .arg("--tls-sv-key")
            .arg(pki.path("server.key"))
            .stdin(Stdio::piped())
            .stdout(output.try_clone()?)
            .stderr(output)
            .spawn()?;
        let input = child.stdin.take();
        Ok(Self { child, input, log })
    }
    fn ready(&mut self, client: &Client) -> Result<()> {
        wait(
            || {
                ensure!(
                    self.child.try_wait()?.is_none(),
                    "service exited before readiness: {}",
                    fs::read_to_string(&self.log)?
                );
                Ok(api(client, "GET", "/wallets", None, 200).is_ok())
            },
            "service readiness",
        )
    }
    fn eof(&mut self) -> Result<u128> {
        let start = Instant::now();
        drop(self.input.take());
        loop {
            if let Some(status) = self.child.try_wait()? {
                ensure!(
                    status.success(),
                    "EOF exit {status}: {}",
                    fs::read_to_string(&self.log)?
                );
                break;
            }
            ensure!(
                start.elapsed() < Duration::from_secs(30),
                "EOF shutdown exceeded 30 seconds"
            );
            sleep(Duration::from_millis(10));
        }
        ensure!(
            TcpStream::connect(("127.0.0.1", PORT)).is_err(),
            "listener still accepts"
        );
        drop(TcpListener::bind(("127.0.0.1", PORT))?);
        ensure!(
            fs::read_to_string(&self.log)?.contains("Quetzal shutdown complete"),
            "cleanup completion absent"
        );
        Ok(start.elapsed().as_millis())
    }
}
fn create(client: &Client, family: &str, name: &str) -> Result<String> {
    let mnemonic: Vec<_> = bip39::Mnemonic::generate(15)?
        .words()
        .map(str::to_owned)
        .collect();
    let mut body = json!({"name":name,"mnemonic_sentence":mnemonic,"passphrase":PASSWORD});
    if family == "byron-wallets" {
        body["style"] = json!("icarus");
    }
    let created = api(client, "POST", &format!("/{family}"), Some(body), 201)?;
    Ok(format!("/{family}/{}", text(&created, "/id")?))
}
fn address(client: &Client, path: &str) -> Result<String> {
    text(
        &api(client, "GET", &format!("{path}/addresses"), None, 200)?,
        "/0/id",
    )
}
fn balance(client: &Client, path: &str, amount: u64) -> Result<()> {
    wait(
        || {
            let w = api(client, "GET", path, None, 200)?;
            Ok(w["state"]["status"] == "ready" && w["balance"]["available"]["quantity"] == amount)
        },
        "ready balance",
    )
}
fn payment(client: &Client, path: &str, destination: &str, amount: u64) -> Result<String> {
    let sent = api(
        client,
        "POST",
        &format!("{path}/transactions"),
        Some(json!({
            "payments":[{"address":destination,"amount":{"quantity":amount,"unit":"lovelace"}}],
            "passphrase":PASSWORD,"time_to_live":{"quantity":600,"unit":"second"}
        })),
        202,
    )?;
    text(&sent, "/id")
}
fn transaction(client: &Client, path: &str, id: &str, status: &str) -> Result<Value> {
    wait(
        || {
            Ok(api(
                client,
                "GET",
                &format!("{path}/transactions/{id}"),
                None,
                200,
            )?["status"]
                == status)
        },
        "transaction status",
    )?;
    api(
        client,
        "GET",
        &format!("{path}/transactions/{id}"),
        None,
        200,
    )
}
fn snapshot(client: &Client, path: &str) -> Result<Value> {
    let w = api(client, "GET", path, None, 200)?;
    let history = api(client, "GET", &format!("{path}/transactions"), None, 200)?;
    let fields = [
        "id",
        "status",
        "direction",
        "amount",
        "fee",
        "inputs",
        "outputs",
        "metadata",
        "inserted_at",
        "pending_since",
        "expires_at",
        "deposit_taken",
        "deposit_returned",
        "withdrawals",
        "script_validity",
    ];
    let history: Vec<Value> = history
        .as_array()
        .context("history not array")?
        .iter()
        .map(|row| {
            Value::Object(
                fields
                    .iter()
                    .map(|field| ((*field).into(), row[*field].clone()))
                    .collect(),
            )
        })
        .collect();
    Ok(
        json!({"id":w["id"],"name":w["name"],"balance":w["balance"],"assets":w["assets"],"passphrase":w["passphrase"],
        "addresses":api(client,"GET",&format!("{path}/addresses"),None,200)?,
        "utxo":api(client,"GET",&format!("{path}/utxo"),None,200)?,"history":history}),
    )
}
fn mempool(node: &NodeOracle, id: &str) -> Result<Value> {
    let before = node.mempool_info(TIMEOUT)?;
    let mut child = Command::new(&node.executable)
        .args([
            "latest",
            "query",
            "tx-mempool",
            "tx-exists",
            id,
            "--socket-path",
        ])
        .arg(&node.socket)
        .args(["--testnet-magic", "42"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let output = child.stdout.take().context("missing node query output")?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = Instant::now() + TIMEOUT;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("mempool query timed out");
        }
        sleep(Duration::from_millis(10));
    };
    ensure!(status.success(), "mempool query failed");
    let bytes = reader
        .join()
        .map_err(|_| anyhow::anyhow!("mempool reader panicked"))??;
    ensure!(bytes.len() <= 1_048_576, "mempool output too large");
    let value: Value = serde_json::from_slice(&bytes)?;
    let slot = value["slot"].as_u64().context("mempool slot absent")?;
    let after = node.mempool_info(TIMEOUT)?;
    ensure!(
        before["slot"].as_u64().context("before slot absent")? <= slot
            && slot <= after["slot"].as_u64().context("after slot absent")?,
        "mempool bracket differs"
    );
    ensure!(
        value == json!({"exists":true,"slot":slot,"txId":id}),
        "pending not in independent node mempool: {value}"
    );
    Ok(value)
}
fn main() -> Result<()> {
    let config: Value = serde_json::from_slice(&fs::read("target/shutdown-conformance.json")?)?;
    let root = PathBuf::from("/tmp/quetzal-shutdown-native-20261110");
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let runtime = root.join("runtime");
    let mut launch: Launch = serde_json::from_value(config["cluster_launch"].clone())?;
    for value in &mut launch.arguments {
        if value == "{cluster_configs}" {
            *value = text(&config, "/base_configs")?;
        }
        if value == "19192" {
            *value = "19193".into();
        }
    }
    let mut cluster = Cluster::start(launch, &runtime)?;
    cluster.discover(
        Path::new(&text(&config, "/cardano_node")?),
        &runtime,
        TIMEOUT,
    )?;
    let node = NodeOracle {
        executable: text(&config, "/cardano_cli")?.into(),
        socket: runtime.join("node.socket"),
        network_magic: 42,
    };
    node.advancing(0, TIMEOUT)?;
    let (other_producers, producer_proof) = cluster.pause(TIMEOUT)?.resume_one(TIMEOUT)?;
    let pki = transport::generate(Path::new(&text(&config, "/openssl")?), &root.join("pki"))?;
    let mut identity = fs::read(pki.path("client.crt"))?;
    identity.push(b'\n');
    identity.extend(fs::read(pki.path("client.key"))?);
    let client = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .tls_built_in_root_certs(false)
        .add_root_certificate(Certificate::from_pem(&fs::read(pki.path("ca.crt"))?)?)
        .identity(Identity::from_pem(&identity)?)
        .build()?;
    let executable = Path::new(
        config["candidate_launch"]["executable"]
            .as_str()
            .context("candidate missing")?,
    );
    let mut service = Service::start(&root, &pki, "before", executable)?;
    service.ready(&client)?;
    service
        .input
        .as_mut()
        .context("stdin pipe missing")?
        .write_all(b"still running\n\0")?;
    let shelley = create(&client, "wallets", "EOF Shelley")?;
    let byron = create(&client, "byron-wallets", "EOF Icarus")?;
    let recipient = create(&client, "wallets", "EOF recipient")?;
    let destination = address(&client, &recipient)?;
    let shelley_address = address(&client, &shelley)?;
    let byron_address = address(&client, &byron)?;
    balance(&client, &shelley, 0)?;
    balance(&client, &byron, 0)?;
    let plain = Client::builder().no_proxy().timeout(TIMEOUT).build()?;
    call(
        &plain,
        "http://127.0.0.1:19193",
        "POST",
        "/send/faucet-assets",
        Some(
            json!({"batch-size":1,"assets":[{"address":shelley_address,"bundle":{"coin":300_000_000,"assets":[]},"metadata":[]}]}),
        ),
        204,
    )?;
    balance(&client, &shelley, 300_000_000)?;
    let funding = node.utxos(&shelley_address, TIMEOUT)?;
    ensure!(
        funding
            .as_object()
            .context("funding missing")?
            .values()
            .any(|v| v["address"] == shelley_address
                && v["value"] == json!({"lovelace":300_000_000})),
        "independent funding differs"
    );
    let bridge = payment(&client, &shelley, &byron_address, 100_000_000)?;
    node.wait_payment(&bridge, &byron_address, 100_000_000, TIMEOUT)?;
    transaction(&client, &shelley, &bridge, "in_ledger")?;
    balance(&client, &byron, 100_000_000)?;
    let paths = [&shelley, &byron];
    let mut confirmed = Vec::new();
    for path in paths {
        let id = payment(&client, path, &destination, 2_000_000)?;
        node.wait_payment(&id, &destination, 2_000_000, TIMEOUT)?;
        transaction(&client, path, &id, "in_ledger")?;
        confirmed.push(id);
    }
    let paused = cluster.pause(TIMEOUT)?;
    sleep(Duration::from_secs(1));
    let tip = node.tip(TIMEOUT)?;
    let mut pending = Vec::new();
    let mut proofs = Vec::new();
    for path in paths {
        let id = payment(&client, path, &destination, 3_000_000)?;
        transaction(&client, path, &id, "pending")?;
        proofs.push(mempool(&node, &id)?);
        ensure!(
            !node
                .utxos(&destination, TIMEOUT)?
                .as_object()
                .context("node UTxOs invalid")?
                .keys()
                .any(|k| k.starts_with(&format!("{id}#"))),
            "pending already on ledger"
        );
        pending.push(id);
    }
    let before = paths
        .map(|path| snapshot(&client, path))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let key_before = api(&client, "GET", &format!("{shelley}/keys"), None, 200)?;
    let first_exit_ms = service.eof()?;
    drop(service);
    let mut restarted = Service::start(&root, &pki, "after", executable)?;
    restarted.ready(&client)?;
    for (index, path) in paths.iter().enumerate() {
        transaction(&client, path, &pending[index], "pending")?;
        ensure!(
            snapshot(&client, path)? == before[index],
            "{path} durable state changed across EOF restart"
        );
        proofs.push(mempool(&node, &pending[index])?);
    }
    ensure!(
        api(&client, "GET", &format!("{shelley}/keys"), None, 200)? == key_before,
        "account public key changed"
    );
    let after_tip = node.tip(TIMEOUT)?;
    ensure!(
        after_tip.hash == tip.hash && after_tip.height == tip.height && after_tip.slot == tip.slot,
        "ledger advanced during EOF recovery"
    );
    paused.resume()?;
    // No submission after restart: the original IDs must confirm from retained state.
    let mut final_wallets = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        node.wait_payment(&pending[index], &destination, 3_000_000, TIMEOUT)?;
        let tx = transaction(&client, path, &pending[index], "in_ledger")?;
        let old = &before[index];
        let confirmed_fee = transaction(&client, path, &confirmed[index], "in_ledger")?["fee"]
            ["quantity"]
            .as_u64()
            .context("confirmed fee absent")?;
        ensure!(
            confirmed_fee > 0
                && tx["fee"]["quantity"]
                    .as_u64()
                    .context("pending fee absent")?
                    > 0,
            "invalid fee"
        );
        let final_snapshot = snapshot(&client, path)?;
        ensure!(
            final_snapshot["balance"]["total"] == old["balance"]["total"],
            "pending confirmation changed total balance"
        );
        ensure!(
            final_snapshot["history"]
                .as_array()
                .context("final history absent")?
                .iter()
                .filter(|row| row["id"] == pending[index])
                .count()
                == 1,
            "duplicate original transaction"
        );
        final_wallets.push(final_snapshot);
    }
    let second_exit_ms = restarted.eof()?;
    drop(restarted);
    other_producers.resume()?;
    cluster.stop()?;
    cluster.wait_stopped_nodes(TIMEOUT)?;
    let report = json!({"passed":true,"scope":"candidate-specific native EOF validation; external T110 runner pending","transport":"mutual TLS","families":["Shelley","Icarus Byron"],"confirmed_ids":confirmed,"pending_ids":pending,"node_mempool_before_and_after":proofs,"paused_tip":{"height":tip.height,"slot":tip.slot,"hash":tip.hash},"producer_control":producer_proof,"before":before,"after_confirmation":final_wallets,"eof_exit_ms":[first_exit_ms,second_exit_ms],"same_database":true,"exact_restart_snapshots":true,"same_public_key":true,"no_resubmission_after_restart":true,"original_ids_independently_confirmed":true,"owned_cluster_cleanup":true});
    fs::write(
        "evidence/20261110-shutdown/native-eof.json",
        serde_json::to_vec_pretty(&report)?,
    )?;
    println!(
        "native EOF qualification passed; clean exit timings {first_exit_ms}ms, {second_exit_ms}ms"
    );
    Ok(())
}
