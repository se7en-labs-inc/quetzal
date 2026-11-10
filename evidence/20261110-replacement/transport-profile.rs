//! Run the runner's unchanged public mutual-TLS profile on the frozen binary.
use anyhow::{ensure, Result};
use serde_json::{json, Value};
use std::{fs, path::Path, time::Duration};
use wallet_conformance::{
    owned_process::{Launch, WalletProcess},
    report::Report,
    transport,
};
fn main() -> Result<()> {
    let c: Value = serde_json::from_slice(&fs::read(
        "evidence/20261110-replacement/full-execution-config.json",
    )?)?;
    let root = Path::new("/tmp/qz-repl-transport-individual-20261110");
    fs::create_dir(root)?;
    let pki = transport::generate(Path::new(c["openssl"].as_str().unwrap()), &root.join("pki"))?;
    let mut launch: Launch = serde_json::from_value(c["candidate_launch"].clone())?;
    launch.arguments = serde_json::from_value(c["transport"]["arguments"].clone())?;
    for a in &mut launch.arguments {
        if a == "{node_socket}" {
            *a = "/tmp/qz-repl-individual-20261110/standard-runtime/node.socket".into();
        }
        if a == "{byron_genesis}" {
            *a = "/tmp/qz-repl-individual-20261110/standard-runtime/cluster/byron-genesis.json"
                .into();
        }
        for (placeholder, path) in pki.launch_paths() {
            if a == placeholder {
                *a = path.to_string_lossy().into_owned();
            }
        }
    }
    let target = c["transport"]["target"].as_str().unwrap();
    let timeout = Duration::from_secs(60);
    let mut report = Report::new(target, "quetzal-individual-20261110");
    let mut process = WalletProcess::start(launch, &root.join("wallet"))?;
    let checked = (|| {
        process.wait_listener(target, timeout)?;
        transport::check(target, &pki, timeout, &mut report)?;
        process.ensure_running()?;
        Ok(())
    })();
    report.record("transport:owned-service", &[], checked);
    report.record("transport:cleanup", &[], process.stop());
    fs::write(
        "evidence/20261110-replacement/transport-mtls.json",
        serde_json::to_vec_pretty(&json!(report))?,
    )?;
    ensure!(report.passed(), "transport profile failed");
    println!("unchanged transport profile passed");
    Ok(())
}
