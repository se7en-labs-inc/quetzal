//! Opt-in benchmark driver; deliberately excluded from normal wallet packages.
#[path = "runtime_bench/fixture.rs"]
mod fixture;
#[path = "runtime_bench/probe.rs"]
mod probe;

use anyhow::{bail, ensure, Context, Result};
use fixture::Manifest;
use reqwest::{Client, Method};
use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::BTreeMap, io::Write, path::Path, sync::Arc, time::Duration};
use tokio::{sync::Semaphore, task::JoinSet, time::Instant};

const HELP: &str = "quetzal-runtime-bench (opt-in synthetic scale benchmark)
  fixtures DEST [--small]
  run BASE_URL MANIFEST OUTPUT [SECONDS WARMUP_SECONDS RATE SCENARIO SOURCE_IDENTITY]
  probe BASE_URL OUTPUT_DIR [ROUNDS INTERVAL_MS DEADLINE_MS]

Defaults: 1,000 seconds, 60 seconds warmup, 50 requests/second.
Run is read-only except repeated name updates to the first Shelley fixture wallet.
Use only disposable fixture state. No node or candidate is started by this driver.
TLS: optional QUETZAL_BENCH_CA and QUETZAL_BENCH_IDENTITY PEM file paths.
QUETZAL_BENCH_PID enables once-per-second Linux process resource sampling.
SCENARIO and SOURCE_IDENTITY are required evidence labels; default is unqualified.
Every scheduled request, including skipped sends at the client bound, is recorded.
Probe mode only issues GETs and writes incremental per-endpoint diagnostics.
";

#[derive(Clone)]
struct Request {
    class: String,
    method: Method,
    path: String,
    body: Option<Value>,
    expected: Expected,
}

#[derive(Clone)]
enum Expected {
    Summary {
        id: String,
        total: u64,
        available: u64,
    },
    List {
        ids: Vec<String>,
    },
    History,
    Array(usize),
    Utxos(usize),
    Network,
    Rename {
        id: String,
    },
    AccountKey(String),
}

impl Expected {
    fn check(&self, body: &Value) -> bool {
        match self {
            Self::Summary {
                id,
                total,
                available,
            } => {
                body["id"] == *id
                    && body
                        .pointer("/balance/total/quantity")
                        .and_then(Value::as_u64)
                        == Some(*total)
                    && body
                        .pointer("/balance/available/quantity")
                        .and_then(Value::as_u64)
                        == Some(*available)
            }
            Self::List { ids } => body.as_array().is_some_and(|items| {
                items.len() == ids.len()
                    && ids.iter().all(|id| items.iter().any(|v| v["id"] == *id))
            }),
            Self::History => body.as_array().is_some_and(|items| {
                items.len() == 100
                    && items
                        .iter()
                        .all(|v| v["id"].as_str().is_some_and(|s| s.len() == 64))
            }),
            Self::Array(count) => body.as_array().is_some_and(|items| items.len() == *count),
            Self::Utxos(count) => body["entries"]
                .as_array()
                .is_some_and(|items| items.len() == *count),
            Self::Network => body["sync_progress"]["status"]
                .as_str()
                .is_some_and(|s| matches!(s, "ready" | "syncing" | "not_responding")),
            Self::Rename { id } => body["id"] == *id && body["name"] == "runtime-benchmark",
            Self::AccountKey(expected) => body
                .as_str()
                .and_then(|s| quetzal_address::codec::bech32_decode(s).ok())
                .is_some_and(|(_, bytes)| hex::encode(bytes) == *expected),
        }
    }
}

fn schedule(manifest: &Manifest) -> Result<Vec<Request>> {
    ensure!(
        manifest.version == 1 && manifest.synthetic,
        "unsupported benchmark manifest"
    );
    ensure!(
        manifest.wallets.len() == 5,
        "expected five fixture families"
    );
    let shelley = manifest
        .wallets
        .iter()
        .find(|w| w.family == "shelley")
        .context("missing Shelley fixture")?;
    let mut requests = Vec::new();
    // Repeat a fixed 20-slot mix over all five families. No random state drift.
    for offset in 0..5 {
        let wallet = &manifest.wallets[offset];
        let path = format!("/v2/{}/{}", wallet.path, wallet.id);
        let summary = Request {
            class: format!("summary/{}", wallet.family),
            method: Method::GET,
            path: path.clone(),
            body: None,
            expected: Expected::Summary {
                id: wallet.id.clone(),
                total: wallet.total,
                available: wallet.available,
            },
        };
        for _ in 0..4 {
            requests.push(summary.clone());
        }
        let list = |family: &str| Request {
            class: format!("wallet_list/{family}"),
            method: Method::GET,
            path: format!("/v2/{family}"),
            body: None,
            expected: Expected::List {
                ids: manifest
                    .wallets
                    .iter()
                    .filter(|w| w.path == family)
                    .map(|w| w.id.clone())
                    .collect(),
            },
        };
        requests.push(list(&wallet.path));
        requests.push(list(&wallet.path));
        for _ in 0..4 {
            requests.push(Request {
                class: "network".into(),
                method: Method::GET,
                path: "/v2/network/information".into(),
                body: None,
                expected: Expected::Network,
            });
        }
        for _ in 0..4 {
            requests.push(Request {
                class: format!("history_100/{}", wallet.family),
                method: Method::GET,
                path: format!("{path}/transactions?max_count=100&order=descending"),
                body: None,
                expected: Expected::History,
            });
        }
        // The public shared API has addresses/history, but no shared UTxO route.
        // Use Shelley for that class; do not benchmark a routing error as a read.
        let utxo = if wallet.path == "shared-wallets" {
            shelley
        } else {
            wallet
        };
        for _ in 0..2 {
            requests.push(Request {
                class: format!("addresses_complete/{}", wallet.family),
                method: Method::GET,
                path: format!("{path}/addresses"),
                body: None,
                expected: Expected::Array(wallet.addresses),
            });
        }
        for _ in 0..2 {
            requests.push(Request {
                class: format!("utxos_complete/{}", utxo.family),
                method: Method::GET,
                path: format!("/v2/{}/{}/utxo", utxo.path, utxo.id),
                body: None,
                expected: Expected::Utxos(utxo.utxos - 1),
            });
        }
        requests.push(Request {
            class: "rename".into(),
            method: Method::PUT,
            path: format!("/v2/wallets/{}", shelley.id),
            body: Some(json!({"name":"runtime-benchmark"})),
            expected: Expected::Rename {
                id: shelley.id.clone(),
            },
        });
        requests.push(Request {
            class: "derive_account".into(),
            method: Method::POST,
            path: format!("/v2/wallets/{}/keys/0H", shelley.id),
            body: Some(json!({"passphrase":fixture::PASSWORD,"format":"extended"})),
            expected: Expected::AccountKey(shelley.account_xpub.clone()),
        });
    }
    Ok(requests)
}

#[derive(Serialize)]
struct Sample {
    class: String,
    scheduled_us: u64,
    latency_us: u64,
    status: Option<u16>,
    bytes: usize,
    outcome: String,
}

#[derive(Default, Serialize)]
struct Distribution {
    scheduled: usize,
    completed: usize,
    successful: usize,
    failures: usize,
    p50_us: u64,
    p95_us: u64,
    p99_us: u64,
    max_us: u64,
}

#[derive(Serialize)]
struct ResourceSample {
    elapsed_ms: u64,
    rss_kib: Option<u64>,
    threads: Option<u64>,
    user_ticks: Option<u64>,
    system_ticks: Option<u64>,
    read_bytes: Option<u64>,
    write_bytes: Option<u64>,
}

fn named_number(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        line.strip_prefix(name)?
            .split_whitespace()
            .next()?
            .parse()
            .ok()
    })
}

fn cpu_ticks(stat: &str) -> (Option<u64>, Option<u64>) {
    // Linux /proc/PID/stat: the comm field can itself contain spaces or ')'.
    // After its final ')' the first token is field 3 (state); CPU fields are 14/15.
    let fields: Vec<_> = stat
        .rsplit_once(')')
        .map_or("", |(_, tail)| tail)
        .split_whitespace()
        .collect();
    (
        fields.get(11).and_then(|v| v.parse().ok()),
        fields.get(12).and_then(|v| v.parse().ok()),
    )
}

async fn resources(pid: u32, stop: tokio_util::sync::CancellationToken) -> Vec<ResourceSample> {
    let started = Instant::now();
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut samples = Vec::new();
    loop {
        tokio::select! {
            biased;
            _=stop.cancelled()=>break,
            _=interval.tick()=>{},
        }
        let (status, stat, io) = tokio::join!(
            tokio::fs::read_to_string(format!("/proc/{pid}/status")),
            tokio::fs::read_to_string(format!("/proc/{pid}/stat")),
            tokio::fs::read_to_string(format!("/proc/{pid}/io")),
        );
        let status = status.unwrap_or_default();
        let stat = stat.unwrap_or_default();
        let io = io.unwrap_or_default();
        let (user_ticks, system_ticks) = cpu_ticks(&stat);
        samples.push(ResourceSample {
            elapsed_ms: started.elapsed().as_millis() as u64,
            rss_kib: named_number(&status, "VmRSS:"),
            threads: named_number(&status, "Threads:"),
            user_ticks,
            system_ticks,
            read_bytes: named_number(&io, "read_bytes:"),
            write_bytes: named_number(&io, "write_bytes:"),
        });
    }
    samples
}

fn distributions(samples: &[Sample]) -> BTreeMap<String, Distribution> {
    let mut groups: BTreeMap<String, Vec<&Sample>> = BTreeMap::new();
    for sample in samples {
        groups.entry(sample.class.clone()).or_default().push(sample);
    }
    groups
        .into_iter()
        .map(|(class, values)| {
            // Include all scheduled outcomes. Failures additionally prevent qualification.
            let mut times: Vec<_> = values.iter().map(|s| s.latency_us).collect();
            times.sort_unstable();
            let percentile = |p: usize| times[(times.len() * p).div_ceil(100).saturating_sub(1)];
            let successful = values.iter().filter(|s| s.outcome == "ok").count();
            let completed = values.iter().filter(|s| s.status.is_some()).count();
            (
                class,
                Distribution {
                    scheduled: values.len(),
                    completed,
                    successful,
                    failures: values.len() - successful,
                    p50_us: percentile(50),
                    p95_us: percentile(95),
                    p99_us: percentile(99),
                    max_us: times.last().copied().unwrap_or(0),
                },
            )
        })
        .collect()
}

async fn execute(
    client: Client,
    base: String,
    request: Request,
    scheduled: Instant,
    origin: Instant,
    connections: Arc<Semaphore>,
) -> Sample {
    let class = request.class;
    let response = tokio::time::timeout_at(scheduled + Duration::from_secs(10), async {
        let _connection = connections.acquire_owned().await?;
        let mut builder = client.request(request.method, format!("{base}{}", request.path));
        if let Some(body) = request.body {
            builder = builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(serde_json::to_vec(&body)?);
        }
        let mut response = builder.send().await?;
        let status = response.status().as_u16();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 32 * 1024 * 1024,
                "response exceeded benchmark byte limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        let body: Value = serde_json::from_slice(&bytes)?;
        let outcome = if !(200..300).contains(&status) {
            "http_error"
        } else if !request.expected.check(&body) {
            "invalid_response"
        } else {
            "ok"
        };
        Ok::<_, anyhow::Error>((status, bytes.len(), outcome.to_owned()))
    })
    .await;
    let (status, bytes, outcome) = match response {
        Ok(Ok((status, bytes, outcome))) => (Some(status), bytes, outcome),
        Ok(Err(_)) => (None, 0, "transport_or_decode_error".into()),
        Err(_) => (None, 0, "deadline".into()),
    };
    Sample {
        class,
        scheduled_us: scheduled.duration_since(origin).as_micros() as u64,
        latency_us: scheduled.elapsed().as_micros() as u64,
        status,
        bytes,
        outcome,
    }
}

async fn phase(
    client: &Client,
    base: &str,
    requests: &[Request],
    seconds: u64,
    rate: u64,
) -> Result<Vec<Sample>> {
    let mut tasks = JoinSet::new();
    let connections = Arc::new(Semaphore::new(16));
    let origin = Instant::now();
    let mut samples = Vec::new();
    let count = seconds
        .checked_mul(rate)
        .context("request count overflow")?;
    ensure!(count <= 1_000_000, "too many samples in one phase");
    for n in 0..count {
        let scheduled = origin + Duration::from_secs_f64(n as f64 / rate as f64);
        tokio::time::sleep_until(scheduled).await;
        while let Some(sample) = tasks.try_join_next() {
            samples.push(sample.context("load job panicked")?);
        }
        let request = requests[n as usize % requests.len()].clone();
        if tasks.len() >= 64 {
            samples.push(Sample {
                class: request.class,
                scheduled_us: scheduled.duration_since(origin).as_micros() as u64,
                latency_us: scheduled.elapsed().as_micros() as u64,
                status: None,
                bytes: 0,
                outcome: "client_inflight_bound".into(),
            });
        } else {
            tasks.spawn(execute(
                client.clone(),
                base.into(),
                request,
                scheduled,
                origin,
                connections.clone(),
            ));
        }
    }
    while let Some(sample) = tasks.join_next().await {
        samples.push(sample.context("load job panicked")?);
    }
    samples.sort_by_key(|s| s.scheduled_us);
    Ok(samples)
}

async fn run(args: &[String]) -> Result<()> {
    ensure!(args.len() >= 3 && args.len() <= 8, "{HELP}");
    let base = args[0].trim_end_matches('/');
    let url = reqwest::Url::parse(base)?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.username().is_empty()
            && url.password().is_none(),
        "invalid benchmark URL"
    );
    let manifest_bytes = std::fs::read(&args[1])?;
    let manifest: Manifest = serde_json::from_slice(&manifest_bytes)?;
    let requests = schedule(&manifest)?;
    let value = |index: usize, default: u64| -> Result<u64> {
        Ok(args
            .get(index)
            .map(|s| s.parse())
            .transpose()?
            .unwrap_or(default))
    };
    let seconds = value(3, 1_000)?;
    let warmup = value(4, 60)?;
    let rate = value(5, 50)?;
    ensure!(
        seconds > 0 && seconds <= 3600 && warmup <= 600 && (1..=1000).contains(&rate),
        "invalid duration or request rate"
    );
    let scenario = args.get(6).map(String::as_str).unwrap_or("unqualified");
    let identity = args.get(7).map(String::as_str).unwrap_or("unqualified");
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&args[2])
        .context("output must not exist")?;
    let client = http_client()?;
    let process_id = std::env::var("QUETZAL_BENCH_PID")
        .ok()
        .map(|s| s.parse::<u32>())
        .transpose()?;
    let resource_stop = tokio_util::sync::CancellationToken::new();
    let resource_task = process_id.map(|pid| tokio::spawn(resources(pid, resource_stop.clone())));
    // Validate every route before measuring; errors remain in the receipt.
    let now = Instant::now();
    let mut preflight = Vec::new();
    for request in &requests {
        preflight.push(
            execute(
                client.clone(),
                base.into(),
                request.clone(),
                Instant::now(),
                now,
                Arc::new(Semaphore::new(1)),
            )
            .await,
        );
    }
    let preflight_ok = preflight.iter().all(|s| s.outcome == "ok");
    let warmup_samples = if preflight_ok {
        phase(&client, base, &requests, warmup, rate).await?
    } else {
        vec![]
    };
    let samples = if preflight_ok {
        phase(&client, base, &requests, seconds, rate).await?
    } else {
        vec![]
    };
    let complete = preflight_ok && samples.len() as u64 == seconds * rate;
    let all_responses_valid = complete
        && samples.iter().all(|s| s.outcome == "ok")
        && warmup_samples.iter().all(|s| s.outcome == "ok");
    resource_stop.cancel();
    let resources = match resource_task {
        Some(task) => task.await.context("resource sampler panicked")?,
        None => vec![],
    };
    let receipt = json!({"version":1,"scenario":scenario,"source_identity":identity,"synthetic":true,"native_sync_qualified":false,"performance_qualified":false,
        "manifest_hash":hex::encode(pallas_crypto::hash::Hasher::<256>::hash(&manifest_bytes)),"seconds":seconds,"warmup_seconds":warmup,"offered_rate":rate,"connection_limit":16,"inflight_limit":64,
        "complete":complete,"all_responses_valid":all_responses_valid,"resources":resources,"distributions":distributions(&samples),"preflight":preflight,"warmup_distributions":distributions(&warmup_samples),"samples":samples});
    output.write_all(&serde_json::to_vec_pretty(&receipt)?)?;
    output.sync_all()?;
    println!(
        "{}",
        json!({"receipt":args[2],"complete":complete,"all_responses_valid":all_responses_valid})
    );
    ensure!(
        all_responses_valid,
        "benchmark encountered invalid/missing responses; inspect receipt"
    );
    Ok(())
}

fn http_client() -> Result<Client> {
    let mut builder = Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .pool_max_idle_per_host(16);
    if let Some(path) = std::env::var_os("QUETZAL_BENCH_CA") {
        builder =
            builder.add_root_certificate(reqwest::Certificate::from_pem(&std::fs::read(path)?)?);
    }
    if let Some(path) = std::env::var_os("QUETZAL_BENCH_IDENTITY") {
        builder = builder.identity(reqwest::Identity::from_pem(&std::fs::read(path)?)?);
    }
    Ok(builder.build()?)
}

#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("fixtures") if args.len() == 2 || (args.len() == 3 && args[2] == "--small") => {
            fixture::create(Path::new(&args[1]), args.len() == 3).await
        }
        Some("run") => run(&args[1..]).await,
        Some("probe") => probe::run(&args[1..]).await,
        None | Some("--help") => {
            print!("{HELP}");
            Ok(())
        }
        _ => bail!("{HELP}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_fields_handle_spaces_and_parentheses_without_exposing_command_lines() {
        assert_eq!(
            named_number("VmRSS:\t123 kB\nThreads:\t7\n", "VmRSS:"),
            Some(123)
        );
        assert_eq!(
            named_number("VmRSS:\t123 kB\nThreads:\t7\n", "Threads:"),
            Some(7)
        );
        assert_eq!(
            cpu_ticks("42 (a name) with ) characters) S 1 2 3 4 5 6 7 8 9 10 111 222"),
            (Some(111), Some(222))
        );
        assert_eq!(cpu_ticks("unavailable"), (None, None));
    }

    #[tokio::test]
    async fn http_failures_and_incorrect_success_bodies_cannot_pass_preflight() {
        use axum::{routing::get, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener,Router::new()
                .route("/overload",get(||async {(axum::http::StatusCode::SERVICE_UNAVAILABLE,axum::Json(json!({"code":"service_unavailable","message":"Queue capacity exhausted"})))}))
                .route("/wrong",get(||async {axum::Json(json!({"sync_progress":{"status":"invented"}}))})))
                .await.unwrap();
        });
        for (path, status, outcome) in [
            ("/overload", 503, "http_error"),
            ("/wrong", 200, "invalid_response"),
        ] {
            let now = Instant::now();
            let sample = execute(
                Client::new(),
                format!("http://{address}"),
                Request {
                    class: "network".into(),
                    method: Method::GET,
                    path: path.into(),
                    body: None,
                    expected: Expected::Network,
                },
                now,
                now,
                Arc::new(Semaphore::new(1)),
            )
            .await;
            assert_eq!(sample.status, Some(status));
            assert_eq!(sample.outcome, outcome);
        }
        server.abort();
        let _ = server.await;
    }
    #[test]
    fn percentiles_keep_failures_and_use_nearest_rank() {
        let samples: Vec<_> = (1..=100)
            .map(|n| Sample {
                class: "summary".into(),
                scheduled_us: 0,
                latency_us: n,
                status: Some(if n == 100 { 503 } else { 200 }),
                bytes: 0,
                outcome: if n == 100 { "http_error" } else { "ok" }.into(),
            })
            .collect();
        let stats = distributions(&samples);
        let summary = &stats["summary"];
        assert_eq!(
            (
                summary.p50_us,
                summary.p95_us,
                summary.p99_us,
                summary.max_us
            ),
            (50, 95, 99, 100)
        );
        assert_eq!(
            (
                summary.scheduled,
                summary.completed,
                summary.successful,
                summary.failures
            ),
            (100, 100, 99, 1)
        );
        assert!(distributions(&[]).is_empty());
    }
}
