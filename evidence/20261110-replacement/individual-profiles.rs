//! Invoke unchanged public conformance commands with exclusively owned fixtures.
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};
use wallet_conformance::{
    cluster_control::Cluster,
    node::NodeOracle,
    owned_process::{Launch, WalletProcess},
};
const ROOT: &str = "/tmp/qz-repl-individual-20261110";
const RUNNER: &str =
    "/home/sam/work/se7en/quetzal/master/target/conformance/debug/wallet-conformance";
const TIMEOUT: Duration = Duration::from_secs(300);
fn s(p: impl AsRef<Path>) -> String {
    p.as_ref().to_string_lossy().into_owned()
}
fn val(c: &Value, k: &str) -> String {
    c[k].as_str().unwrap().into()
}
fn save(p: impl AsRef<Path>, v: &Value) -> Result<()> {
    fs::write(p, serde_json::to_vec_pretty(v)?)?;
    Ok(())
}
fn launch(c: &Value, k: &str) -> Result<Launch> {
    Ok(serde_json::from_value(c[k].clone())?)
}
fn cluster_launch(c: &Value, configs: &Path) -> Result<Launch> {
    let mut l = launch(c, "cluster_launch")?;
    for a in &mut l.arguments {
        if a == "{cluster_configs}" {
            *a = s(configs)
        }
    }
    Ok(l)
}
fn wallet(c: &Value, k: &str, r: &Path) -> Result<Launch> {
    let mut l = launch(c, k)?;
    for a in &mut l.arguments {
        if a == "{node_socket}" {
            *a = s(r.join("node.socket"))
        }
        if a == "{byron_genesis}" {
            *a = s(r.join("cluster/byron-genesis.json"))
        }
    }
    if !l.arguments.iter().any(|a| a == "--shutdown-handler") {
        l.arguments.push("--shutdown-handler".into())
    }
    Ok(l)
}
fn manifest(root: &Path, name: &str, l: &Launch) -> Result<String> {
    let p = root.join(format!("{name}.json"));
    save(
        &p,
        &json!({"executable":l.executable,"arguments":l.arguments,"environment":l.environment}),
    )?;
    Ok(s(p))
}
fn invoke(root: &Path, name: &str, args: Vec<String>) -> Result<bool> {
    println!("individual: {name}");
    let f = fs::File::create(root.join(format!("{name}.log")))?;
    save(
        root.join(format!("{name}-command.json")),
        &json!({"executable":RUNNER,"arguments":args}),
    )?;
    let status = Command::new(RUNNER)
        .args(args)
        .stdin(Stdio::null())
        .stdout(f.try_clone()?)
        .stderr(f)
        .status()?;
    save(
        root.join(format!("{name}-exit.json")),
        &json!({"passed":status.success(),"code":status.code()}),
    )?;
    println!("individual: {name}: {status}");
    Ok(status.success())
}
fn common(c: &Value, root: &Path, name: &str, cmd: &str) -> Vec<String> {
    vec![
        cmd.into(),
        "--target".into(),
        val(c, "target"),
        "--environment".into(),
        "quetzal-individual-20261110".into(),
        "--report".into(),
        s(root.join("reports").join(format!("{name}.json"))),
        "--timeout-seconds".into(),
        "300".into(),
    ]
}
fn live(c: &Value, root: &Path, name: &str, r: &Path, m: &str) -> Vec<String> {
    let mut a = common(c, root, name, "live");
    a.extend([
        "--node-socket".into(),
        s(r.join("node.socket")),
        "--network-magic".into(),
        "42".into(),
        "--cardano-cli".into(),
        val(c, "cardano_cli"),
        "--faucet".into(),
        val(c, "faucet"),
        "--wallet-launch".into(),
        m.into(),
        "--wallet-workspace".into(),
        s(root.join(format!("owned-{name}"))),
        "--shelley-genesis".into(),
        s(r.join("cluster/shelley-genesis.json")),
        "--byron-genesis".into(),
        s(r.join("cluster/byron-genesis.json")),
    ]);
    a
}
fn cluster(c: &Value, root: &Path, phase: &str, configs: &Path) -> Result<(Cluster, PathBuf)> {
    let r = root.join(format!("{phase}-runtime"));
    let mut x = Cluster::start(cluster_launch(c, configs)?, &r)?;
    x.discover(Path::new(&val(c, "cardano_node")), &r, TIMEOUT)?;
    NodeOracle {
        executable: val(c, "cardano_cli").into(),
        socket: r.join("node.socket"),
        network_magic: 42,
    }
    .advancing(0, TIMEOUT)?;
    Ok((x, r))
}
fn standard(c: &Value, root: &Path) -> Result<()> {
    let (mut x, r) = cluster(c, root, "standard", Path::new(&val(c, "base_configs")))?;
    let (lease, proof) = x
        .pause(Duration::from_secs(10))?
        .resume_one(Duration::from_secs(10))?;
    save(root.join("producer-lease.json"), &proof)?;
    let candidate = wallet(c, "candidate_launch", &r)?;
    let m = manifest(root, "standard-wallet", &candidate)?;
    let mut target = WalletProcess::start(candidate, &root.join("http-candidate"))?;
    target.wait_listener(&val(c, "target"), TIMEOUT)?;
    let mut reference = WalletProcess::start(
        wallet(c, "reference_launch", &r)?,
        &root.join("http-reference"),
    )?;
    reference.wait_listener(&val(c, "reference_target"), TIMEOUT)?;
    for name in [
        "http",
        "http-keys",
        "http-signatures",
        "http-families",
        "http-rejections",
        "http-network",
        "http-statuses",
        "http-extended",
        "http-governance-media",
    ] {
        let mut a = common(c, root, name, "http");
        a.extend([
            "--suite".into(),
            format!("/tmp/qz-repl-full-20261110/bundle/fixtures/{name}.json"),
            "--reference".into(),
            val(c, "reference_target"),
        ]);
        invoke(root, name, a)?;
    }
    let pool = root.join("reward-pool");
    ensure!(
        invoke(
            root,
            "prepare-reward-pool",
            vec![
                "prepare-pool-fixture".into(),
                "--owned-cluster-pool".into(),
                s(r.join("cluster/pool-1")),
                "--node-socket".into(),
                s(r.join("node.socket")),
                "--cardano-cli".into(),
                val(c, "cardano_cli"),
                "--target".into(),
                val(c, "target"),
                "--faucet".into(),
                val(c, "faucet"),
                "--output-directory".into(),
                s(&pool),
                "--pledge".into(),
                "0".into(),
                "--timeout-seconds".into(),
                "300".into()
            ]
        )?,
        "pool fixture failed"
    );
    let drep = root.join("drep-fixture");
    ensure!(
        invoke(
            root,
            "prepare-drep",
            vec![
                "prepare-drep-fixture".into(),
                "--node-socket".into(),
                s(r.join("node.socket")),
                "--faucet".into(),
                val(c, "faucet"),
                "--cardano-cli".into(),
                val(c, "cardano_cli"),
                "--workspace".into(),
                s(&drep),
                "--timeout-seconds".into(),
                "300".into()
            ]
        )?,
        "DRep fixture failed"
    );
    target.stop()?;
    let receipt: Value = serde_json::from_slice(&fs::read(pool.join("public/receipt.json"))?)?;
    let mut a = live(c, root, "live-main", &r, &m);
    a.extend([
        "--external-signer".into(),
        val(c, "reference_target"),
        "--stake-pool".into(),
        receipt["pool_id"].as_str().context("pool missing")?.into(),
        "--openssl".into(),
        val(c, "openssl"),
    ]);
    for flag in [
        "families",
        "shared",
        "assets",
        "payments",
        "concurrency",
        "metadata",
        "precedence",
        "validity",
        "balance",
        "collateral",
        "voting",
        "delegation-fees",
        "input-limits",
        "history",
        "discovery",
    ] {
        a.push(format!("--{flag}"));
    }
    invoke(root, "live-main", a)?;
    let mut disconnected = wallet(c, "candidate_launch", &r)?;
    for a in &mut disconnected.arguments {
        if a == &s(r.join("node.socket")) {
            *a = s(root.join("disconnect.socket"))
        }
    }
    let dm = manifest(root, "disconnect-wallet", &disconnected)?;
    let mut a = live(c, root, "disconnect", &r, &dm);
    a.extend([
        "--node-disconnect-socket".into(),
        s(root.join("disconnect.socket")),
    ]);
    invoke(root, "disconnect", a)?;
    let mut a = common(c, root, "drep", "verify-drep-lookup");
    a.extend([
        "--node-socket".into(),
        s(r.join("node.socket")),
        "--cardano-cli".into(),
        val(c, "cardano_cli"),
        "--fixture-receipt".into(),
        s(drep.join("public/receipt.json")),
        "--wallet-launch".into(),
        m,
        "--wallet-workspace".into(),
        s(root.join("drep-owned")),
    ]);
    invoke(root, "drep", a)?;
    reference.stop()?;
    drop(lease);
    x.stop()?;
    x.wait_stopped_nodes(Duration::from_secs(10))?;
    save(
        root.join("standard-cleanup.json"),
        &json!({"owned_nodes_absent":true,"owned_wallets_stopped_and_reaped":true}),
    )?;
    Ok(())
}
fn committee(c: &Value, root: &Path) -> Result<()> {
    let fixture = root.join("committee-fixture");
    ensure!(
        invoke(
            root,
            "prepare-committee",
            vec![
                "prepare-parameter-cluster".into(),
                "--base-configs".into(),
                val(c, "base_configs"),
                "--workspace".into(),
                s(&fixture),
                "--cardano-cli".into(),
                val(c, "cardano_cli")
            ]
        )?,
        "committee fixture failed"
    );
    let (mut x, r) = cluster(c, root, "committee", &fixture.join("config"))?;
    let m = manifest(
        root,
        "committee-wallet",
        &wallet(c, "candidate_launch", &r)?,
    )?;
    let mut a = live(c, root, "pools", &r, &m);
    a.extend([
        "--pool-directory".into(),
        s(r.join("cluster")),
        "--pool-workspace".into(),
        s(root.join("pool-proofs")),
    ]);
    invoke(root, "pools", a)?;
    let mut a = live(c, root, "parameters", &r, &m);
    a.extend([
        "--pool-join-guard".into(),
        "--parameter-fixture".into(),
        s(fixture),
        "--parameter-workspace".into(),
        s(root.join("parameter-update")),
    ]);
    invoke(root, "parameters", a)?;
    x.stop()?;
    x.wait_stopped_nodes(Duration::from_secs(10))?;
    save(
        root.join("committee-cleanup.json"),
        &json!({"owned_nodes_absent":true}),
    )?;
    Ok(())
}
fn owned(c: &Value, root: &Path) -> Result<()> {
    let cm = manifest(
        root,
        "owned-cluster",
        &cluster_launch(c, Path::new(&val(c, "base_configs")))?,
    )?;
    let wm = manifest(root, "owned-wallet", &launch(c, "candidate_launch")?)?;
    let rm = manifest(root, "owned-adversary", &launch(c, "reference_launch")?)?;
    for (name, flag) in [
        ("pending-normal", None),
        ("pending-expiry", Some("--expiry")),
        ("pending-byron", Some("--byron")),
        ("pending-retry", Some("--retry")),
        ("checkpoint", Some("--checkpoint")),
    ] {
        let mut a = common(c, root, name, "verify-pending-state");
        a.extend([
            "--cluster-launch".into(),
            cm.clone(),
            "--wallet-launch".into(),
            wm.clone(),
            "--workspace".into(),
            s(root.join(name)),
            "--cardano-node".into(),
            val(c, "cardano_node"),
            "--cardano-cli".into(),
            val(c, "cardano_cli"),
            "--faucet".into(),
            val(c, "faucet"),
        ]);
        if let Some(f) = flag {
            a.push(f.into());
        }
        if name == "pending-retry" {
            a.extend([
                "--adversary-wallet-launch".into(),
                rm.clone(),
                "--adversary-target".into(),
                val(c, "reference_target"),
            ]);
        }
        invoke(root, name, a)?;
    }
    for (name, cmd) in [
        ("rollback", "verify-rollback"),
        ("era", "verify-era-boundary"),
    ] {
        let mut a = common(c, root, name, cmd);
        a.extend([
            "--base-cluster".into(),
            s(root.join("standard-runtime/cluster")),
            "--workspace".into(),
            s(root.join(name)),
            "--cardano-node".into(),
            val(c, "cardano_node"),
            "--cardano-cli".into(),
            val(c, "cardano_cli"),
            "--wallet-launch".into(),
            wm.clone(),
        ]);
        if name == "rollback" {
            a.extend(["--key-oracle".into(), val(c, "key_oracle")]);
        } else {
            a.extend(["--conway-epoch".into(), "3".into()]);
        }
        invoke(root, name, a)?;
    }
    Ok(())
}
fn main() -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let c: Value = serde_json::from_slice(&fs::read(
        "evidence/20261110-replacement/full-execution-config.json",
    )?)?;
    let root = Path::new(ROOT);
    let phase = std::env::args().nth(1).context("phase required")?;
    if phase == "standard" {
        fs::create_dir(root)?;
        fs::create_dir(root.join("reports"))?;
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    match phase.as_str() {
        "standard" => standard(&c, root),
        "committee" => committee(&c, root),
        "owned" => owned(&c, root),
        "disconnect" => {
            let (mut x,r) = cluster(&c,root,"disconnect",Path::new(&val(&c,"base_configs")))?;
            let mut w = wallet(&c,"candidate_launch",&r)?;
            for a in &mut w.arguments { if a == &s(r.join("node.socket")) { *a = s(root.join("disconnect.socket")); } }
            let m = manifest(root,"disconnect-wallet-repeat",&w)?;
            let mut a = live(&c,root,"disconnect-repeat",&r,&m);
            a.extend(["--node-disconnect-socket".into(),s(root.join("disconnect.socket"))]);
            let passed = invoke(root,"disconnect-repeat",a)?;
            x.stop()?;
            x.wait_stopped_nodes(Duration::from_secs(10))?;
            save(root.join("disconnect-repeat-cleanup.json"),&json!({"owned_nodes_absent":true}))?;
            ensure!(passed,"disconnect profile failed");
            Ok(())
        },
        _ => anyhow::bail!("unknown phase"),
    }
}
