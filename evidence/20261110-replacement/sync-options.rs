//! Supplemental live option-effect check; not the deferred lagging/ahead profile.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    net::{TcpListener, TcpStream},
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, Instant},
};
use wallet_conformance::{
    api::{Contract, Response},
    cluster_control::Cluster,
    node::NodeOracle,
    owned_process::{Launch, WalletProcess},
    transport,
};
const TIMEOUT: Duration = Duration::from_secs(60);
struct Api {
    client: reqwest::blocking::Client,
    label: String,
}
struct Server {
    process: WalletProcess,
    api: Api,
    ui: u16,
}
fn network(api: &Api) -> Result<Value> {
    let result = api
        .client
        .get(format!("{}network/information", api.label))
        .send()?;
    let status = result.status().as_u16();
    let media_type = result
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_owned());
    let raw = result.bytes()?.to_vec();
    let response = Response {
        status,
        media_type,
        json: serde_json::from_slice(&raw).ok(),
        raw,
    };
    ensure!(status == 200, "network status {status}");
    Contract::pinned()?.validate("getNetworkInformation", &response)?;
    response.json.context("network response missing")
}
fn start(
    config: &Value,
    root: &Path,
    pki: &transport::Pki,
    role: &str,
    port: u16,
    tolerance: u64,
    node: &NodeOracle,
) -> Result<Server> {
    let source: Launch = serde_json::from_value(
        config[if role == "reference" {
            "reference_launch"
        } else {
            "candidate_launch"
        }]
        .clone(),
    )?;
    let genesis = node
        .socket
        .parent()
        .context("node fixture root missing")?
        .join("cluster/byron-genesis.json");
    let mut arguments = vec![
        "serve".into(),
        "--node-socket".into(),
        node.socket.to_string_lossy().into_owned(),
        "--testnet".into(),
        genesis.to_string_lossy().into_owned(),
        "--database".into(),
        "{database}".into(),
        "--listen-address".into(),
        "127.0.0.1".into(),
        "--port".into(),
        port.to_string(),
        "--ui-port".into(),
        (port + 100).to_string(),
        "--sync-tolerance".into(),
        format!("{tolerance}s"),
        "--shutdown-handler".into(),
    ];
    for (flag, path) in [
        ("--tls-ca-cert", pki.path("ca.crt")),
        ("--tls-sv-cert", pki.path("server.crt")),
        ("--tls-sv-key", pki.path("server.key")),
    ] {
        arguments.extend([flag.into(), path.to_string_lossy().into_owned()]);
    }
    let url = format!("https://127.0.0.1:{port}/v2/");
    let mut process = WalletProcess::start(
        Launch {
            executable: source.executable,
            arguments,
            environment: source.environment,
        },
        &root.join(format!("{role}-{tolerance}")),
    )?;
    process.wait_listener(&url, TIMEOUT)?;
    let mut identity = fs::read(pki.path("client.crt"))?;
    identity.push(b'\n');
    identity.extend(fs::read(pki.path("client.key"))?);
    let api = Api {
        label: url,
        client: reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .tls_built_in_root_certs(false)
            .add_root_certificate(reqwest::Certificate::from_pem(&fs::read(
                pki.path("ca.crt"),
            )?)?)
            .identity(reqwest::Identity::from_pem(&identity)?)
            .build()?,
    };
    let deadline = Instant::now() + TIMEOUT;
    while network(&api)?["sync_progress"]["status"] == "not_responding" {
        ensure!(
            Instant::now() < deadline,
            "{role} never observed its native node"
        );
        sleep(Duration::from_millis(20));
    }
    Ok(Server {
        process,
        api,
        ui: port + 100,
    })
}
fn main() -> Result<()> {
    let config: Value = serde_json::from_slice(&fs::read(
        "evidence/20261110-replacement/full-execution-config.json",
    )?)?;
    let root = PathBuf::from("/tmp/qz-repl-sync-options2-20261110");
    fs::DirBuilder::new().mode(0o700).create(&root)?;
    let pki = transport::generate(
        Path::new(config["openssl"].as_str().context("openssl missing")?),
        &root.join("pki"),
    )?;
    let runtime = root.join("runtime");
    let mut launch: Launch = serde_json::from_value(config["cluster_launch"].clone())?;
    for a in &mut launch.arguments {
        if a == "{cluster_configs}" {
            *a = config["base_configs"]
                .as_str()
                .context("configs missing")?
                .into();
        }
        if a == "19293" {
            *a = "19295".into();
        }
    }
    let mut cluster = Cluster::start(launch, &runtime)?;
    cluster.discover(
        Path::new(config["cardano_node"].as_str().context("node missing")?),
        &runtime,
        TIMEOUT,
    )?;
    let (lease, _) = cluster.pause(TIMEOUT)?.resume_one(TIMEOUT)?;
    let node = NodeOracle {
        executable: config["cardano_cli"]
            .as_str()
            .context("CLI missing")?
            .into(),
        socket: runtime.join("node.socket"),
        network_magic: 42,
    };
    node.tip(TIMEOUT)?;
    let mut proofs = Vec::new();
    for tolerance in [0, 300] {
        let mut reference = start(&config, &root, &pki, "reference", 19241, tolerance, &node)?;
        let mut candidate = start(&config, &root, &pki, "candidate", 19240, tolerance, &node)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            ensure!(
                Instant::now() < deadline,
                "could not obtain stable reference/native bracket"
            );
            let before = node.tip(TIMEOUT)?;
            let first = network(&reference.api)?;
            let actual = network(&candidate.api)?;
            let last = network(&reference.api)?;
            let after = node.tip(TIMEOUT)?;
            // A moving native tip or wall-slot boundary is not a comparison of
            // one fixture state. Retry only these independently observed races.
            if before.hash != after.hash
                || first["node_tip"] != last["node_tip"]
                || first["network_tip"] != last["network_tip"]
                || first["sync_progress"]["status"] != last["sync_progress"]["status"]
            {
                sleep(Duration::from_millis(10));
                continue;
            }
            let expected = if tolerance == 0 { "syncing" } else { "ready" };
            if first["sync_progress"]["status"] != expected {
                sleep(Duration::from_millis(20));
                continue;
            }
            ensure!(
                first["node_tip"]["absolute_slot_number"] == before.slot
                    && actual["node_tip"]["absolute_slot_number"] == before.slot,
                "API node tip differs from independent native tip"
            );
            ensure!(
                actual["sync_progress"]["status"] == expected,
                "configured tolerance differs from reference"
            );
            if tolerance == 0 {
                let quantity = |v: &Value| {
                    v.pointer("/sync_progress/progress/quantity")
                        .and_then(Value::as_f64)
                        .context("syncing percentage absent")
                };
                let (a, b, c) = (quantity(&first)?, quantity(&last)?, quantity(&actual)?);
                ensure!(
                    0.0 < c
                        && c < 100.0
                        && actual["sync_progress"]["progress"]["unit"] == "percent",
                    "invalid/intermediate percentage"
                );
                ensure!(
                    a.min(b) <= c && c <= a.max(b),
                    "candidate percentage outside reference bracket: {a}/{c}/{b}"
                );
            } else {
                ensure!(
                    actual["sync_progress"].get("progress").is_none(),
                    "ready response has a percentage"
                );
            }
            proofs.push(json!({"tolerance_seconds":tolerance,"native":{"height":before.height,"slot":before.slot,"hash":before.hash},"reference_before":first,"candidate":actual,"reference_after":last}));
            break;
        }
        for server in [&mut candidate, &mut reference] {
            ensure!(
                TcpStream::connect(("127.0.0.1", server.ui)).is_ok(),
                "UI listener missing"
            );
            server.process.shutdown_eof(&server.api.label, TIMEOUT)?;
            ensure!(
                TcpStream::connect(("127.0.0.1", server.ui)).is_err(),
                "UI listener survived EOF"
            );
            drop(TcpListener::bind(("127.0.0.1", server.ui))?);
        }
    }
    drop(lease);
    cluster.stop()?;
    cluster.wait_stopped_nodes(Duration::from_secs(10))?;
    fs::write(
        "evidence/20261110-replacement/sync-options.json",
        serde_json::to_vec_pretty(
            &json!({"passed":true,"scope":"configured tolerance and exact percent on one independently advancing native fixture; not the deferred lagging/ahead qualification","transport":"mutual TLS","tolerances":[0,300],"observations":proofs,"owned_service_eof_cleanup":true,"owned_nodes_absent":true}),
        )?,
    )?;
    println!("reference/candidate live tolerance and percent comparison passed");
    Ok(())
}
