//! Each wallet owns its restoration cursor. A new/restored wallet must scan
//! from its own checkpoint rather than inherit the service's network tip.
use crate::state::AppState;
use quetzal_network::{chainsync::ChainSyncDriver, ChainSyncEvent, NodeClient};
use quetzal_primitives::WalletId;
use quetzal_wallet::WalletError;
use std::time::Duration;

pub(crate) struct WalletWorker {
    stop: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<()>,
    retry_stop: tokio::sync::oneshot::Sender<()>,
    retry_task: tokio::task::JoinHandle<()>,
}

impl AppState {
    pub async fn start_wallet_worker(&self, wallet: WalletId) {
        let mut workers = self.wallet_workers.lock().await;
        if workers
            .get(&wallet)
            .is_some_and(|worker| !worker.task.is_finished())
        {
            return;
        }
        let state = self.clone();
        let id = wallet.clone();
        let retry_state = self.clone();
        let retry_id = wallet.clone();
        let (retry_stop, mut retry_cancelled) = tokio::sync::oneshot::channel();
        let retry_task = tokio::spawn(async move {
            let mut ticks = tokio::time::interval(Duration::from_secs(1));
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = &mut retry_cancelled => break,
                    _ = ticks.tick() => {},
                }
                tokio::select! {
                    biased;
                    _ = &mut retry_cancelled => break,
                    result = retry_state.resubmit_pending(&retry_id) => {
                        if let Err(error) = result {tracing::warn!(wallet=%retry_id,%error,"Pending transaction retry failed");}
                    },
                }
            }
        });
        let (stop, mut cancelled) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let clock_config = state.config.clone();
            let worker = state
                .wallet_service
                .wallet_sync_worker(id.clone())
                .with_slot_reference(move |slot| clock_config.slot_reference(slot));
            loop {
                if state.wallet_service.get_wallet(&id).await.is_err() {
                    break;
                }
                let connection = tokio::select! {
                    biased;
                    _ = &mut cancelled => break,
                    result = NodeClient::connect(&state.config.socket_path, state.config.network_magic) => result,
                };
                let mut node = match connection {
                    Ok(node) => node,
                    Err(error) => {
                        tracing::warn!(wallet=%id,%error,"Wallet sync connection failed");
                        tokio::select! {
                            _ = &mut cancelled => break,
                            _ = tokio::time::sleep(Duration::from_secs(2)) => continue,
                        }
                    }
                };
                let result: Result<(), WalletError> = tokio::select! {
                    biased;
                    _ = &mut cancelled => Ok(()),
                    result = async {
                    let mut driver = ChainSyncDriver::new(node.chainsync());
                    let checkpoints = worker.checkpoints()?;
                    driver
                        .find_intersect(&checkpoints)
                        .await
                        .map_err(|error| WalletError::TxBuild(error.to_string()))?;
                    loop {
                        match driver
                            .next()
                            .await
                            .map_err(|error| WalletError::TxBuild(error.to_string()))?
                        {
                            ChainSyncEvent::Await => worker.process_await()?,
                            ChainSyncEvent::RollForward {
                                block_cbor,
                                tip_slot,
                            } => worker.process_roll_forward(&block_cbor, tip_slot).await?,
                            ChainSyncEvent::RollBackward { to_slot, to_hash } => {
                                worker.process_roll_backward(to_slot, to_hash).await?
                            }
                        }
                    }
                    } => result,
                };
                node.abort().await;
                if result.is_ok() {
                    break;
                }
                if let Err(error) = result {
                    tracing::warn!(wallet=%id,%error,"Wallet sync restarting");
                }
                tokio::select! {
                    _ = &mut cancelled => break,
                    _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                }
            }
        });
        workers.insert(
            wallet,
            WalletWorker {
                stop,
                task,
                retry_stop,
                retry_task,
            },
        );
    }

    pub async fn stop_wallet_worker(&self, wallet: &WalletId) {
        let worker = self.wallet_workers.lock().await.remove(wallet);
        if let Some(worker) = worker {
            let _ = worker.stop.send(());
            let _ = worker.retry_stop.send(());
            let _ = worker.task.await;
            let _ = worker.retry_task.await;
        }
    }

    async fn resubmit_pending(&self, wallet: &WalletId) -> Result<(), WalletError> {
        if !self.chain_ready.load(std::sync::atomic::Ordering::Acquire) {
            return Ok(());
        }
        let slot = self.chain_tip.read().await.0;
        let coefficient = if let Some(genesis) = &self.config.shelley_genesis {
            genesis["activeSlotsCoeff"]
                .as_f64()
                .ok_or_else(|| WalletError::TxBuild("invalid active slot coefficient".into()))?
        } else {
            0.05
        };
        if !coefficient.is_finite() || coefficient <= 0.0 || coefficient > 1.0 {
            return Err(WalletError::TxBuild(
                "invalid active slot coefficient".into(),
            ));
        }
        let interval = (10.0 / coefficient).ceil();
        if interval >= u64::MAX as f64 {
            return Err(WalletError::TxBuild(
                "resubmission slot interval overflow".into(),
            ));
        }
        let interval = interval as u64;
        let _submission = self.submission_lock.lock().await;
        for pending in self.wallet_service.get_pending_txs(wallet).await? {
            if pending.expiry_slot.is_some_and(|expiry| slot >= expiry)
                || pending
                    .latest_submission_slot
                    .unwrap_or(pending.submitted_slot)
                    .checked_add(interval)
                    .is_none_or(|next| slot < next)
            {
                continue;
            }
            let result = tokio::time::timeout(
                Duration::from_secs(10),
                NodeClient::submit_at_tip(
                    &self.config.socket_path,
                    self.config.network_magic,
                    pending.cbor,
                ),
            )
            .await;
            match result {
                Ok(Ok(())) => {
                    tracing::debug!(wallet=%wallet,transaction=%pending.id,"Resubmitted pending transaction")
                }
                Ok(Err(error)) => {
                    tracing::debug!(wallet=%wallet,transaction=%pending.id,%error,"Node rejected pending retry")
                }
                Err(error) => {
                    tracing::debug!(wallet=%wallet,transaction=%pending.id,%error,"Pending retry timed out")
                }
            }
            self.wallet_service
                .mark_pending_submission(wallet, &pending.id, slot)?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn stopping_a_worker_during_handshake_releases_its_socket() {
        let directory = std::env::temp_dir().join(format!("qz-worker-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        let socket = directory.join("node.socket");
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        let service =
            quetzal_wallet::WalletService::new(directory.join("db").to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "worker".into(),
                &[47; 20],
                b"public-test-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let config = crate::config::Config {
            socket_path: socket.to_str().unwrap().into(),
            network_magic: 42,
            ..Default::default()
        };
        let state = AppState::new(service, config).unwrap();
        state.start_wallet_worker(wallet.id.clone()).await;
        state.start_wallet_worker(wallet.id.clone()).await;
        assert_eq!(state.wallet_workers.lock().await.len(), 1);
        let (mut connection, _) = tokio::time::timeout(Duration::from_secs(2), listener.accept())
            .await
            .unwrap()
            .unwrap();
        let mut data = [0; 1024];
        assert!(
            tokio::time::timeout(Duration::from_secs(2), connection.read(&mut data))
                .await
                .unwrap()
                .unwrap()
                > 0
        );
        tokio::time::timeout(Duration::from_secs(2), state.stop_wallet_worker(&wallet.id))
            .await
            .unwrap();
        assert!(state.wallet_workers.lock().await.is_empty());
        let mut remaining = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(2),
            connection.read_to_end(&mut remaining),
        )
        .await
        .unwrap()
        .unwrap();
        // Stopping is idempotent and does not delete wallet state itself.
        state.stop_wallet_worker(&wallet.id).await;
        assert_eq!(
            state
                .wallet_service
                .get_wallet(&wallet.id)
                .await
                .unwrap()
                .id,
            wallet.id
        );
        state
            .wallet_service
            .delete_wallet(&wallet.id)
            .await
            .unwrap();
        drop(connection);
        drop(listener);
        drop(state);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
