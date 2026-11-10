mod cli;
mod config;
mod error;
mod handlers;
mod json;
mod metadata;
mod router;
mod shutdown;
mod state;
#[cfg(test)]
mod test_pki;
mod transport;
mod ui;
mod workers;

use std::sync::Arc;
use std::time::Duration;

use config::Config;
use quetzal_network::chainsync::ChainSyncDriver;
use quetzal_network::{ChainSyncEvent, NodeClient};
use quetzal_wallet::WalletService;
use state::AppState;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments: Vec<_> = std::env::args().skip(1).collect();
    if arguments.as_slice() == ["version"] || arguments.as_slice() == ["--version"] {
        println!("Quetzal {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if arguments.iter().any(|s| s == "--help-tracing") {
        print!("{}", cli::TRACING_HELP);
        return Ok(());
    }
    if arguments.iter().any(|s| s == "--help" || s == "-h") {
        print!("{}", cli::HELP);
        return Ok(());
    }
    let mut config = Config::from_args(arguments)?;
    fmt()
        .with_env_filter(EnvFilter::new(cli::logging_filter(&config)))
        .json()
        .init();

    if let Some(host) = &config.listen_address {
        config.host = match host.as_str() {
            "*" | "*6" | "!6" => std::net::Ipv6Addr::UNSPECIFIED.into(),
            "*4" | "!4" => std::net::Ipv4Addr::UNSPECIFIED.into(),
            _ => match host.parse::<std::net::IpAddr>() {
                Ok(ip) => ip,
                Err(_) => tokio::net::lookup_host((host.as_str(), config.port))
                    .await?
                    .min_by_key(|addr| !addr.is_ipv4())
                    .ok_or_else(|| anyhow::anyhow!("listen hostname has no address"))?
                    .ip(),
            },
        };
    }
    config.validate_transport()?;
    let temporary = if config.temporary_database {
        Some(tempfile::tempdir()?)
    } else {
        None
    };
    if let Some(directory) = &temporary {
        config.db_path = directory
            .path()
            .join("wallet")
            .to_string_lossy()
            .into_owned();
    }
    // Fail closed before opening wallet state or starting workers.
    let tls = transport::load(&config)?;
    let addr = std::net::SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let ui_listener = if let Some(port) = config.ui_port {
        Some(tokio::net::TcpListener::bind((config.host, port)).await?)
    } else {
        None
    };
    // Query the node's own clock, with no neighboring genesis filename required.
    if std::path::Path::new(&config.socket_path).exists() {
        if let Ok(Ok(snapshot)) = tokio::time::timeout(
            Duration::from_secs(3),
            NodeClient::network_snapshot(&config.socket_path, config.network_magic),
        )
        .await
        {
            config.set_native_eras(snapshot.timings);
            config.set_native_genesis(snapshot.genesis);
        }
    }
    let wallet_service = WalletService::new(&config.db_path)?;
    if let Some(source) = &config.pool_metadata_source {
        wallet_service.set_pool_metadata_source(source)?;
    }
    let state = AppState::new(wallet_service, config.clone())?;
    for wallet in state.wallet_service.list_wallets().await? {
        state.start_wallet_worker(wallet.id).await;
    }
    let byron = state.for_byron();
    for wallet in byron.wallet_service.list_wallets().await? {
        byron.start_wallet_worker(wallet.id).await;
    }

    // Spawn the background chain-sync task.  It connects (and reconnects) to
    // the node socket independently of the HTTP server so that a missing or
    // temporarily unavailable node never blocks startup.
    let stop = CancellationToken::new();
    let tasks = [
        spawn_chain_sync(state.clone(), stop.clone()),
        spawn_drep_refresh(state.clone(), stop.clone()),
        spawn_pool_gc(state.clone(), stop.clone()),
    ];

    let router = router::build_router(state.clone());
    tracing::info!(address = %addr, mutual_tls = tls.is_some(), "Quetzal wallet backend starting");

    let serving = async {
        if let Some(ui_listener) = ui_listener {
            tracing::info!(address=%ui_listener.local_addr()?,"Quetzal personal wallet UI starting");
            let ui = ui::router(state.clone());
            tokio::try_join!(
                transport::serve(listener, router, tls.clone(), stop.clone()),
                transport::serve(ui_listener, ui, tls, stop.clone()),
            )
            .map(|_| ())
        } else {
            transport::serve(listener, router, tls, stop.clone()).await
        }
    };
    tokio::pin!(serving);
    let result = tokio::select! {
        result = &mut serving => result,
        reason = shutdown::wait(config.shutdown_handler) => {
            tracing::info!(reason = ?reason, "Quetzal shutdown requested");
            stop.cancel();
            let served = serving.await;
            reason.map(|_| ()).and(served)
        },
    };
    stop.cancel();
    let byron = state.for_byron();
    let stopped = tokio::time::timeout(Duration::from_secs(10), async {
        let mut failure = None;
        for task in tasks {
            if let Err(error) = task.await {
                failure.get_or_insert(anyhow::Error::from(error));
            }
        }
        let (shelley, byron) = tokio::join!(
            state.stop_all_wallet_workers(),
            byron.stop_all_wallet_workers(),
        );
        if let Some(error) = failure {
            return Err(error);
        }
        shelley.and(byron)
    })
    .await
    .map_err(|_| anyhow::anyhow!("wallet shutdown exceeded its grace period"))?;
    stopped?;
    // No handlers or workers may write after this point. Flush both families,
    // attempting the second store even if the first reports an I/O error.
    let flushed = state.wallet_service.flush();
    let byron_flushed = byron.wallet_service.flush();
    result
        .and(flushed.map_err(Into::into))
        .and(byron_flushed.map_err(Into::into))?;
    tracing::info!("Quetzal shutdown complete");
    Ok(())
}

/// Refresh actual Conway registry state, then authenticate off-chain anchors.
fn spawn_drep_refresh(state: AppState, stop: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
        _ = stop.cancelled() => {},
        _ = async {
        loop {
            let delay = match handlers::dreps::refresh(&state).await {
                Err(error) => {
                    tracing::warn!(%error, "DRep registry refresh failed");
                    5
                }
                Ok(()) => 900,
            };
            tokio::time::sleep(Duration::from_secs(delay)).await;
        }
        } => {},
        }
    })
}

/// Refresh the configured SMASH delisted set on startup and every five minutes.
fn spawn_pool_gc(state: AppState, stop: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
        _ = stop.cancelled() => {},
        _ = async {
        loop {
            if let Err(error) = handlers::pool_catalogue::refresh_gc(&state).await {
                tracing::warn!(%error, "Pool metadata GC refresh failed");
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
        } => {},
        }
    })
}

/// Spawn a background task that connects to cardano-node and drives chain-sync.
///
/// On any error the task logs a warning and retries after 5 seconds, so the
/// HTTP server continues to operate even when the node is temporarily down.
fn spawn_chain_sync(state: AppState, stop: CancellationToken) -> JoinHandle<()> {
    let chain_tip = Arc::clone(&state.chain_tip);
    let chain_hash = Arc::clone(&state.chain_hash);
    let chain_ready = Arc::clone(&state.chain_ready);
    let node_client_slot = Arc::clone(&state.node_client);
    let socket_path = state.config.socket_path.clone();
    let network_magic = state.config.network_magic;

    tokio::spawn(async move {
        loop {
            tracing::info!(socket = %socket_path, "Connecting to cardano-node");

            // ── Connect ──────────────────────────────────────────────────────
            let connection = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                result = NodeClient::connect(&socket_path, network_magic) => result,
            };
            let mut client = match connection {
                Ok(c) => {
                    tracing::info!("Connected to cardano-node");
                    c
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect to cardano-node; retrying in 5s");
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    }
                    continue;
                }
            };

            let clock = tokio::select! {
                _ = stop.cancelled() => None,
                result = async {
                    let mut query = quetzal_network::StateQuerier::new(client.statequery());
                    let eras = query.era_timings().await?;
                    let genesis = query.genesis_config().await?;
                    Ok::<_,quetzal_network::NetworkError>((eras,genesis))
                } => Some(result),
            };
            match clock {
                Some(Ok((eras, genesis))) => {
                    state.config.set_native_eras(eras);
                    state.config.set_native_genesis(genesis);
                }
                Some(Err(error)) => tracing::warn!(%error,"Cannot read native era clock"),
                None => {
                    client.abort().await;
                    break;
                }
            }

            // ── Find intersect ───────────────────────────────────────────────
            {
                let mut driver = ChainSyncDriver::new(client.chainsync());
                // Start from genesis (empty checkpoints).
                let intersection = tokio::select! {
                    _ = stop.cancelled() => None,
                    result = driver.find_intersect(&[]) => Some(result),
                };
                let Some(intersection) = intersection else {
                    client.abort().await;
                    break;
                };
                if let Err(e) = intersection {
                    tracing::warn!(error = %e, "find_intersect failed; retrying in 5s");
                    client.abort().await;
                    tokio::select! {
                        _ = stop.cancelled() => break,
                        _ = tokio::time::sleep(Duration::from_secs(5)) => {},
                    }
                    continue;
                }

                // ── Publish the live client into shared state ─────────────────
                // We do this after a successful intersect so handlers know the
                // connection is ready.  Note: the client is consumed by the sync
                // loop below, so we only store a sentinel `None` here.  Handlers
                // that need a direct query handle should use a separate connection.
                // For now the `node_client` slot holds `None` while syncing is
                // in progress; a future PR will expose a dedicated query client.
                {
                    let guard = node_client_slot.write().await;
                    // We cannot store `client` here while `driver` holds a
                    // mutable borrow of it, so we leave it `None` for now.
                    let _ = guard; // silence unused warning
                }

                // ── Event loop ────────────────────────────────────────────────
                loop {
                    let event = tokio::select! {
                        biased;
                        _ = stop.cancelled() => break,
                        result = driver.next() => result,
                    };
                    match event {
                        Ok(ChainSyncEvent::Await) => {
                            chain_ready.store(true, std::sync::atomic::Ordering::Release);
                        }
                        Ok(ChainSyncEvent::RollForward {
                            block_cbor,
                            tip_slot,
                        }) => {
                            let block = match pallas_traverse::MultiEraBlock::decode(&block_cbor) {
                                Ok(block) => block,
                                Err(error) => {
                                    tracing::warn!(%error, "Cannot decode chain-sync block");
                                    break;
                                }
                            };
                            let issuer = block
                                .header()
                                .issuer_vkey()
                                .map(|key| *pallas_crypto::hash::Hasher::<224>::hash(key));
                            let mut registrations = Vec::new();
                            for tx in block.txs() {
                                if !tx.is_valid() {
                                    continue;
                                }
                                for certificate in tx.certs() {
                                    if let Some(
                                        pallas_primitives::conway::Certificate::PoolRegistration {
                                            operator,
                                            ..
                                        },
                                    ) = certificate.as_conway()
                                    {
                                        registrations.push(**operator);
                                    } else if let Some(
                                        pallas_primitives::alonzo::Certificate::PoolRegistration {
                                            operator,
                                            ..
                                        },
                                    ) = certificate.as_alonzo()
                                    {
                                        registrations.push(**operator);
                                    }
                                }
                            }
                            if let Err(error) = state.wallet_service.observe_pool_block(
                                block.slot(),
                                *block.hash(),
                                issuer,
                                &registrations,
                            ) {
                                tracing::warn!(%error, "Cannot persist pool block production");
                                break;
                            }
                            {
                                let mut tip = chain_tip.write().await;
                                tip.0 = block.slot();
                                tip.1 = block.number();
                                *chain_hash.write().await = *block.hash();
                            }

                            chain_ready.store(
                                block.slot() >= tip_slot,
                                std::sync::atomic::Ordering::Release,
                            );
                        }
                        Ok(ChainSyncEvent::RollBackward { to_slot, to_hash }) => {
                            if let Err(error) = state.wallet_service.rollback_pool_blocks(to_slot) {
                                tracing::warn!(%error, "Cannot roll back pool block production");
                                break;
                            }
                            *chain_hash.write().await = to_hash;
                            chain_ready.store(false, std::sync::atomic::Ordering::Release);
                            {
                                let mut tip = chain_tip.write().await;
                                tip.0 = to_slot;
                            }
                        }
                        Err(e) => {
                            chain_ready.store(false, std::sync::atomic::Ordering::Release);
                            tracing::warn!(error = %e, "ChainSync error; reconnecting in 5s");
                            break;
                        }
                    }
                }
            } // driver dropped here, releasing the borrow on `client`

            client.abort().await;

            // Clear the stored client so handlers know the connection is down.
            {
                let mut guard = node_client_slot.write().await;
                *guard = None;
            }

            tokio::select! {
                _ = stop.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(5)) => {},
            }
        }
    })
}
