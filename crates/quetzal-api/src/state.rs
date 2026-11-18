use crate::config::Config;
use quetzal_wallet::{DRepLayer, WalletService};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct AppState {
    pub(crate) shared_sync: Arc<crate::shared_sync::SharedSync>,
    pub wallet_service: Arc<WalletService>,
    byron_service: Arc<WalletService>,
    byron_workers: Arc<
        tokio::sync::Mutex<
            std::collections::HashMap<quetzal_primitives::WalletId, crate::workers::WalletWorker>,
        >,
    >,
    pub config: Config,
    /// Last node query, independent of historical replay and retained on disconnect.
    pub(crate) network_snapshot: Arc<RwLock<Option<quetzal_network::client::NetworkSnapshot>>>,
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
            shared_sync: Arc::new(crate::shared_sync::SharedSync::new(config.clone())?),
            wallet_service: Arc::new(wallet_service),
            byron_service,
            byron_workers: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
            config,
            network_snapshot: Arc::new(RwLock::new(None)),
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

    pub(crate) async fn observe_network(&self, snapshot: quetzal_network::client::NetworkSnapshot) {
        self.config.set_native_eras(snapshot.timings.clone());
        self.config.set_native_genesis(snapshot.genesis);
        *self.network_snapshot.write().await = Some(snapshot);
    }
}
