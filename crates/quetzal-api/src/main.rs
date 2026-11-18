mod cli;
mod config;
mod error;
mod handlers;
mod json;
mod metadata;
mod router;
mod shared_sync;
mod shutdown;
mod state;
#[cfg(test)]
mod test_pki;
mod transport;
mod ui;
mod workers;

use std::time::Duration;

use config::Config;
use quetzal_network::NodeClient;
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
    if !config.temporary_database {
        let directory = std::path::PathBuf::from(&config.db_path);
        let socket = config.socket_path.clone();
        let network = quetzal_migrate::MigrationNetwork {
            magic: config.network_magic,
            genesis_start: config.genesis_start,
            genesis_hash: config.genesis_hash.clone(),
            mainnet: config.network == config::Network::Mainnet,
        };
        tracing::info!(database=%directory.display(), "Checking persistent wallet database for startup migration");
        let prepared = tokio::task::spawn_blocking(move || {
            quetzal_migrate::prepare_database(&directory, &socket, network)
        })
        .await??;
        config.db_path = prepared
            .path
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("wallet database path is not UTF-8"))?
            .into();
        if let Some(wallets) = prepared.imported_wallets {
            tracing::info!(wallets, database=%prepared.path.display(), "Automatic SQLite wallet migration completed");
        }
    }
    let addr = std::net::SocketAddr::new(config.host, config.port);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let addr = listener.local_addr()?;
    let ui_listener = if let Some(port) = config.ui_port {
        Some(tokio::net::TcpListener::bind((config.host, port)).await?)
    } else {
        None
    };
    // Query the node's own clock, with no neighboring genesis filename required.
    let mut startup_snapshot = None;
    if std::path::Path::new(&config.socket_path).exists() {
        if let Ok(Ok(snapshot)) = tokio::time::timeout(
            Duration::from_secs(3),
            NodeClient::network_snapshot(&config.socket_path, config.network_magic),
        )
        .await
        {
            config.set_native_eras(snapshot.timings.clone());
            config.set_native_genesis(snapshot.genesis);
            startup_snapshot = Some(snapshot);
        }
    }
    let wallet_service = WalletService::new(&config.db_path)?;
    if let Some(source) = &config.pool_metadata_source {
        wallet_service.set_pool_metadata_source(source)?;
    }
    let state = AppState::new(wallet_service, config.clone())?;
    if let Some(snapshot) = startup_snapshot {
        state.observe_network(snapshot).await;
    }
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
        _ = state.shared_sync.failed.cancelled() => {
            Err(anyhow::anyhow!("shared wallet sync failed"))
        },
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
        let shared = state.shared_sync.shutdown().await;
        shelley.and(byron).and(shared)
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

/// Native observation is independent of historical replay. Pool history is a
/// participant in the shared follower and resumes from its recoverable point.
fn spawn_chain_sync(state: AppState, stop: CancellationToken) -> JoinHandle<()> {
    tokio::spawn(async move {
        let registration = state
            .shared_sync
            .register(state.wallet_service.pool_sync_worker())
            .await;
        let registration = match registration {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(%error,"Pool sync registration failed");
                return;
            }
        };
        loop {
            let observed = tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                result = tokio::time::timeout(Duration::from_secs(3), NodeClient::network_snapshot(&state.config.socket_path,state.config.network_magic)) => result,
            };
            match observed {
                Ok(Ok(snapshot)) => {
                    *state.chain_tip.write().await = (snapshot.slot, snapshot.height);
                    let mut hash = [0; 32];
                    if snapshot.hash.len() == 32 {
                        hash.copy_from_slice(&snapshot.hash);
                    }
                    *state.chain_hash.write().await = hash;
                    state
                        .chain_ready
                        .store(true, std::sync::atomic::Ordering::Release);
                    state.observe_network(snapshot).await;
                }
                result => {
                    state
                        .chain_ready
                        .store(false, std::sync::atomic::Ordering::Release);
                    tracing::warn!(error=?result.err(),"Native observation unavailable");
                }
            }
            tokio::select! { _=stop.cancelled()=>break, _=tokio::time::sleep(Duration::from_secs(2))=>{} }
        }
        state.shared_sync.remove(registration).await;
    })
}
