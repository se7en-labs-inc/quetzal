//! Read-only diagnostics for real instances; no synthetic balance assumptions.
use super::{distributions, http_client, Sample};
use anyhow::{ensure, Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs::OpenOptions, io::Write, path::Path, time::Duration};
use tokio::{task::JoinSet, time::Instant};

#[derive(Clone)]
struct Target {
    label: String,
    path: String,
    kind: Kind,
}

#[derive(Clone, Copy)]
enum Kind {
    Network,
    Header,
    Object,
    List,
    Summary,
    Array,
}

struct Observed {
    sample: Sample,
    projection: Value,
    wallets: Vec<(String, String)>,
}

fn event(output: &mut std::fs::File, value: &Value) -> Result<()> {
    serde_json::to_writer(&mut *output, value)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}

async fn request(
    client: Client,
    base: String,
    target: Target,
    deadline: Duration,
    origin: Instant,
) -> Observed {
    let started = Instant::now();
    let mut status = None;
    let mut bytes = 0;
    let mut projection = json!({});
    let mut wallets = Vec::new();
    let result=tokio::time::timeout(deadline,async {
        let mut response=client.get(format!("{base}{}",target.path)).send().await?;
        status=Some(response.status().as_u16());
        let mut body=Vec::new();
        while let Some(chunk)=response.chunk().await? {
            bytes+=chunk.len();
            ensure!(bytes<=4*1024*1024,"probe response byte bound");body.extend_from_slice(&chunk);
        }
        let body:Value=serde_json::from_slice(&body)?;
        if !status.is_some_and(|s|(200..300).contains(&s)) {
            if let Some(code)=body["code"].as_str().filter(|s|s.len()<=80 && s.bytes().all(|b|b.is_ascii_alphanumeric() || b==b'_')) {projection=json!({"api_error_code":code});}
            return Ok::<_,anyhow::Error>("http_error");
        }
        let valid=match target.kind {
            Kind::Network=>{
                projection=json!({"network_magic":body.pointer("/network_info/protocol_magic"),"node_era":body["node_era"],"sync_status":body.pointer("/sync_progress/status"),"node_slot":body.pointer("/node_tip/absolute_slot_number"),"node_height":body.pointer("/node_tip/height/quantity")});
                body.pointer("/network_info/protocol_magic").and_then(Value::as_u64)==Some(2)
            }
            Kind::Header=>{projection=json!({"block_height":body.pointer("/block_height/quantity")});body["header_hash"].as_str().is_some_and(|s|s.len()==64)}
            Kind::Object=>body.is_object(),
            Kind::Array=>body.is_array(),
            Kind::Summary=>{
                projection=json!({"wallet_tip_slot":body.pointer("/tip/absolute_slot_number"),"wallet_sync_status":body.pointer("/state/status"),"wallet_sync_progress":body.pointer("/state/progress/quantity")});
                body["id"].as_str().is_some_and(|id|quetzal_primitives::WalletId::from_hex(id).is_ok())
            }
            Kind::List=>{
                if let Some(items)=body.as_array() {
                    let family=target.path.trim_start_matches("/v2/").to_owned();
                    for item in items.iter().take(5) {
                        if let Some(id)=item["id"].as_str().filter(|id|quetzal_primitives::WalletId::from_hex(id).is_ok()) {wallets.push((family.clone(),id.to_owned()));}
                    }
                    projection=json!({"wallet_count":items.len(),"sampled_wallets":wallets.len()});true
                } else {false}
            }
        };
        Ok(if valid {"ok"} else {"invalid_response_or_wrong_network"})
    }).await;
    let outcome = match result {
        Err(_) => "deadline",
        Ok(Err(_)) => "transport_or_decode_error",
        Ok(Ok(outcome)) => outcome,
    };
    Observed {
        sample: Sample {
            class: target.label,
            scheduled_us: started.duration_since(origin).as_micros() as u64,
            latency_us: started.elapsed().as_micros() as u64,
            status,
            bytes,
            outcome: outcome.into(),
        },
        projection,
        wallets,
    }
}

async fn batch(
    client: &Client,
    base: &str,
    targets: Vec<Target>,
    deadline: Duration,
    origin: Instant,
    round: u64,
    output: &mut std::fs::File,
) -> Result<Vec<Observed>> {
    let mut tasks = JoinSet::new();
    let mut observations = Vec::new();
    let mut sequence = 0u64;
    let batch_kind = if targets.iter().any(|target| target.label.contains("{id}")) {
        "wallet"
    } else {
        "network"
    };
    let mut pending = targets.into_iter();
    loop {
        while tasks.len() < 4 {
            let Some(target) = pending.next() else {
                break;
            };
            sequence += 1;
            let request_id = sequence;
            event(
                output,
                &json!({"event":"request_started","round":round,"batch":batch_kind,"request_id":request_id,"endpoint":target.label}),
            )?;
            let client = client.clone();
            let base = base.to_owned();
            tasks.spawn(async move {
                (
                    request_id,
                    request(client, base, target, deadline, origin).await,
                )
            });
        }
        let Some(observed) = tasks.join_next().await else {
            break;
        };
        let (request_id, observed) = observed.context("probe worker panicked")?;
        event(
            output,
            &json!({"event":"request_finished","round":round,"batch":batch_kind,"request_id":request_id,"sample":observed.sample,"progress":observed.projection}),
        )?;
        observations.push(observed);
    }
    Ok(observations)
}

fn target(path: &str, kind: Kind) -> Target {
    Target {
        label: path.into(),
        path: path.into(),
        kind,
    }
}

pub async fn run(args: &[String]) -> Result<()> {
    ensure!(
        (2..=5).contains(&args.len()),
        "probe BASE_URL OUTPUT_DIR [ROUNDS INTERVAL_MS DEADLINE_MS]"
    );
    let base = args[0].trim_end_matches('/');
    let url = reqwest::Url::parse(base)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        "invalid probe URL"
    );
    let number = |index: usize, default: u64| -> Result<u64> {
        Ok(args
            .get(index)
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(default))
    };
    let rounds = number(2, 12)?;
    let interval = number(3, 1000)?;
    let deadline = number(4, 10_000)?;
    ensure!(
        (1..=1000).contains(&rounds) && interval <= 60_000 && (1..=60_000).contains(&deadline),
        "invalid probe bounds"
    );
    let directory = Path::new(&args[1]);
    std::fs::create_dir(directory).context("probe output directory must not exist")?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(directory.join("events.jsonl"))?;
    event(
        &mut output,
        &json!({"event":"run_started","network_expected":"preview","network_magic_expected":2,"rounds":rounds,"deadline_ms":deadline,"concurrency":4,"read_only":true,"tls":url.scheme()=="https"}),
    )?;
    let client = http_client()?;
    let origin = Instant::now();
    let mut all = Vec::new();
    let mut known = BTreeSet::new();
    let mut preview = false;
    let mut native_header = false;
    for round in 0..rounds {
        let mut targets = vec![
            target("/v2/network/information", Kind::Network),
            target("/v2/network/parameters", Kind::Object),
            target("/v2/blocks/latest/header", Kind::Header),
            target("/v2/settings", Kind::Object),
        ];
        for family in ["wallets", "byron-wallets", "shared-wallets"] {
            targets.push(target(&format!("/v2/{family}"), Kind::List));
        }
        let mut observed = batch(
            &client,
            base,
            targets,
            Duration::from_millis(deadline),
            origin,
            round,
            &mut output,
        )
        .await?;
        for observation in &observed {
            if observation.sample.class == "/v2/network/information"
                && observation.sample.outcome == "ok"
            {
                preview = true;
            }
            if observation.sample.class == "/v2/blocks/latest/header"
                && observation.sample.outcome == "ok"
            {
                native_header = true;
            }
            known.extend(observation.wallets.iter().cloned());
        }
        let mut targets = Vec::new();
        for (family, id) in &known {
            for (suffix, kind) in [
                ("", Kind::Summary),
                ("/addresses", Kind::Array),
                ("/transactions?max_count=100&order=descending", Kind::Array),
            ] {
                targets.push(Target {
                    label: format!("/v2/{family}/{{id}}{suffix}"),
                    path: format!("/v2/{family}/{id}{suffix}"),
                    kind,
                });
            }
            if family != "shared-wallets" {
                targets.push(Target {
                    label: format!("/v2/{family}/{{id}}/utxo"),
                    path: format!("/v2/{family}/{id}/utxo"),
                    kind: Kind::Object,
                });
            }
        }
        observed.extend(
            batch(
                &client,
                base,
                targets,
                Duration::from_millis(deadline),
                origin,
                round,
                &mut output,
            )
            .await?,
        );
        let failures = observed.iter().filter(|o| o.sample.outcome != "ok").count();
        println!(
            "{}",
            json!({"round":round,"requests":observed.len(),"failures":failures,"wallets_sampled":known.len()})
        );
        all.extend(observed.into_iter().map(|o| o.sample));
        if round + 1 < rounds {
            tokio::time::sleep(Duration::from_millis(interval)).await;
        }
    }
    let failures = all.iter().filter(|s| s.outcome != "ok").count();
    let summary = json!({"complete":true,"read_only":true,"preview_network_observed":preview,"native_header_observed":native_header,"wallets_sampled":known.len(),"requests":all.len(),"failures":failures,"distributions":distributions(&all),"performance_qualified":false});
    std::fs::write(
        directory.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    event(
        &mut output,
        &json!({"event":"run_finished","summary":summary}),
    )?;
    output.sync_all()?;
    ensure!(
        failures == 0,
        "probe found HTTP errors, invalid responses or deadlines; inspect incremental receipts"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn stalled_endpoint_does_not_hide_fast_control_and_partial_evidence_is_flushed() {
        use axum::{routing::get, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/slow",
                        get(|| async {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            axum::Json(json!({}))
                        }),
                    )
                    .route("/fast", get(|| async { axum::Json(json!({})) })),
            )
            .await
            .unwrap();
        });
        let root = tempfile::tempdir().unwrap();
        let mut output = std::fs::File::create(root.path().join("events.jsonl")).unwrap();
        let observed = batch(
            &Client::new(),
            &format!("http://{address}"),
            vec![target("/slow", Kind::Object), target("/fast", Kind::Object)],
            Duration::from_millis(50),
            Instant::now(),
            0,
            &mut output,
        )
        .await
        .unwrap();
        assert!(observed
            .iter()
            .any(|o| o.sample.class == "/fast" && o.sample.outcome == "ok"));
        assert!(observed
            .iter()
            .any(|o| o.sample.class == "/slow" && o.sample.outcome == "deadline"));
        let journal = std::fs::read_to_string(root.path().join("events.jsonl")).unwrap();
        assert_eq!(journal.lines().count(), 4);
        server.abort();
        let _ = server.await;
    }
}
