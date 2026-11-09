mod config;
mod error;
mod handlers;
mod json;
mod router;
mod state;
mod workers;

use std::sync::Arc;
use std::time::Duration;

use config::Config;
use quetzal_network::chainsync::ChainSyncDriver;
use quetzal_network::{ChainSyncEvent, NodeClient};
use quetzal_wallet::WalletService;
use state::AppState;
use tracing_subscriber::{fmt, EnvFilter};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("quetzal=info".parse()?))
        .json()
        .init();

    let config = Config::from_args(std::env::args().skip(1))?;
    let wallet_service = WalletService::new(&config.db_path)?;
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
    spawn_chain_sync(state.clone());

    // Spawn background DRep metadata refresh task (every 15 minutes).
    spawn_drep_refresh(state.clone());

    spawn_pool_gc(state.clone());

    let router = router::build_router(state);

    let addr = format!("0.0.0.0:{}", config.port);
    tracing::info!(address = %addr, "Quetzal wallet backend starting");

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, router).await?;
    Ok(())
}

/// Refresh actual Conway registry state, then authenticate off-chain anchors.
fn spawn_drep_refresh(state: AppState) {
    tokio::spawn(async move {
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
    });
}

/// Refresh the configured SMASH delisted set on startup and every five minutes.
fn spawn_pool_gc(state: AppState) {
    tokio::spawn(async move {
        loop {
            if let Err(error) = handlers::pool_catalogue::refresh_gc(&state).await {
                tracing::warn!(%error, "Pool metadata GC refresh failed");
            }
            tokio::time::sleep(Duration::from_secs(300)).await;
        }
    });
}

/// Spawn a background task that connects to cardano-node and drives chain-sync.
///
/// On any error the task logs a warning and retries after 5 seconds, so the
/// HTTP server continues to operate even when the node is temporarily down.
fn spawn_chain_sync(state: AppState) {
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
            let mut client = match NodeClient::connect(&socket_path, network_magic).await {
                Ok(c) => {
                    tracing::info!("Connected to cardano-node");
                    c
                }
                Err(e) => {
                    tracing::warn!(error = %e, "Failed to connect to cardano-node; retrying in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            // ── Find intersect ───────────────────────────────────────────────
            {
                let mut driver = ChainSyncDriver::new(client.chainsync());
                // Start from genesis (empty checkpoints).
                if let Err(e) = driver.find_intersect(&[]).await {
                    tracing::warn!(error = %e, "find_intersect failed; retrying in 5s");
                    client.abort().await;
                    tokio::time::sleep(Duration::from_secs(5)).await;
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
                    match driver.next().await {
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

            tokio::time::sleep(Duration::from_secs(5)).await;
        }
    });
}
