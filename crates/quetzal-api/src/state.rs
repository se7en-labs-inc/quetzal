use crate::config::Config;
use quetzal_network::NodeClient;
use quetzal_wallet::{DRepLayer, WalletService};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub wallet_service: Arc<WalletService>,
    byron_service: Arc<WalletService>,
    byron_workers: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<quetzal_primitives::WalletId, crate::workers::WalletWorker>,
        >,
    >,
    pub config: Config,
    /// Live connection to cardano-node; `None` until the background task connects.
    pub node_client: Arc<RwLock<Option<NodeClient>>>,
    /// Latest observed chain tip: `(slot, block_height)`.
    /// Written by the chain-sync background task; read by network handlers.
    pub chain_tip: Arc<RwLock<(u64, u64)>>,
    pub chain_hash: Arc<RwLock<[u8; 32]>>,
    pub chain_ready: Arc<std::sync::atomic::AtomicBool>,
    /// Latest DRep registry view and verified anchor metadata.
    pub drep_layer: Arc<DRepLayer>,
    pub(crate) submission_lock: Arc<tokio::sync::Mutex<()>>,
    pub(crate) wallet_workers: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<quetzal_primitives::WalletId, crate::workers::WalletWorker>,
        >,
    >,
}

impl AppState {
    pub fn new(
        wallet_service: WalletService,
        config: Config,
    ) -> Result<Self, quetzal_wallet::WalletError> {
        let byron_service = Arc::new(wallet_service.byron_service()?);
        Ok(AppState {
            wallet_service: Arc::new(wallet_service),
            byron_service,
            byron_workers: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            config,
            node_client: Arc::new(RwLock::new(None)),
            chain_tip: Arc::new(RwLock::new((0, 0))),
            chain_hash: Arc::new(RwLock::new([0; 32])),
            chain_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            drep_layer: DRepLayer::new(),
            submission_lock: Arc::new(tokio::sync::Mutex::new(())),
            wallet_workers: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        })
    }

    pub fn for_byron(&self) -> Self {
        let mut state = self.clone();
        state.wallet_service = self.byron_service.clone();
        state.wallet_workers = self.byron_workers.clone();
        state
    }
}
