use crate::{
    error::ApiError,
    json::network::{
        BlockHeight, NetworkClockResponse, NetworkInfoResponse, NetworkParamsResponse, NodeTipInfo,
        SyncProgress,
    },
    state::AppState,
};
use axum::{extract::State, Json};

pub async fn get_network_info(
    State(state): State<AppState>,
) -> Result<Json<NetworkInfoResponse>, ApiError> {
    let (tip_slot, tip_height, observed, era) = match state.network_snapshot.read().await.as_ref() {
        Some(snapshot) => (snapshot.slot, snapshot.height, true, snapshot.era),
        None => {
            let (slot, height) = *state.chain_tip.read().await;
            (slot, height, false, 6)
        }
    };
    let (epoch, slot, time) = state.config.slot_reference(tip_slot);
    let now = std::time::SystemTime::now();
    let wall_slot = state.config.time_to_slot(now, 0, false).unwrap_or(0);
    let (wall_epoch, wall_local_slot, wall_time) = state.config.slot_reference(wall_slot);
    let next_slot = state
        .config
        .epoch_first_slot(wall_epoch + 1)
        .unwrap_or(wall_slot);
    let progress = state.config.wallet_sync_progress_at(tip_slot, now);

    Ok(Json(NetworkInfoResponse {
        node_tip: NodeTipInfo {
            absolute_slot_number: Some(tip_slot),
            slot_number: Some(slot),
            epoch_number: Some(epoch),
            time: Some(time),
            height: Some(BlockHeight {
                quantity: tip_height,
                unit: "block".to_string(),
            }),
        },
        network_tip: Some(NodeTipInfo {
            absolute_slot_number: Some(wall_slot),
            slot_number: Some(wall_local_slot),
            epoch_number: Some(wall_epoch),
            time: Some(wall_time),
            height: None,
        }),
        next_epoch: Some(crate::json::network::EpochInfo {
            epoch_number: wall_epoch + 1,
            epoch_start_time: state.config.slot_reference(next_slot).2,
        }),
        node_era: [
            "byron", "shelley", "allegra", "mary", "alonzo", "babbage", "conway",
        ]
        .get(usize::from(era))
        .unwrap_or(&"conway")
        .to_string(),
        sync_progress: SyncProgress {
            status: if !observed {
                "not_responding"
            } else if progress.is_some() {
                "syncing"
            } else {
                "ready"
            }
            .into(),
            progress: if observed {
                progress.map(|quantity| crate::json::network::ProgressValue {
                    quantity,
                    unit: "percent".into(),
                })
            } else {
                None
            },
        },
        network_info: crate::json::network::NetworkIdentity {
            protocol_magic: state.config.network_magic,
            network_id: if state.config.network == crate::config::Network::Mainnet {
                "mainnet"
            } else {
                "testnet"
            },
        },
        wallet_mode: "node",
    }))
}

pub async fn get_network_parameters(
    State(state): State<AppState>,
) -> Result<Json<NetworkParamsResponse>, ApiError> {
    use crate::json::network::{ByteSize, EpochLength, ExecutionPrices, ProgressValue, SlotLength};
    let mut client =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let result = async {
        let mut query = quetzal_network::StateQuerier::new(client.statequery());
        let parameters = query.protocol_params().await?;
        let native = query.era_timings().await?;
        let eras = native
            .iter()
            .map(|era| (era.start_slot, era.start_epoch))
            .collect::<Vec<_>>();
        state.config.set_native_eras(native);
        state
            .config
            .set_native_genesis(query.genesis_config().await?);
        Ok::<_, quetzal_network::NetworkError>((parameters, eras))
    }
    .await;
    client.abort().await;
    let (parameters, history) = result.map_err(|e| ApiError::internal(e.to_string()))?;
    let mut eras = serde_json::Map::new();
    // The pinned wallet's parameter projection exposes Byron through Babbage;
    // the current node era is reported separately by network information.
    for (index, name) in ["byron", "shelley", "allegra", "mary", "alonzo", "babbage"]
        .iter()
        .enumerate()
    {
        let value = history.get(index).map(|(slot, epoch)| serde_json::json!({"epoch_number":epoch,"epoch_start_time":state.config.slot_reference(*slot).2})).unwrap_or(serde_json::Value::Null);
        eras.insert((*name).into(), value);
    }
    Ok(Json(NetworkParamsResponse {
        genesis_block_hash: state.config.genesis_hash.clone(),
        blockchain_start_time: state.config.slot_reference(0).2,
        slot_length: SlotLength {
            quantity: state.config.current_slot_millis() as f64 / 1000.0,
            unit: "second".to_string(),
        },
        epoch_length: EpochLength {
            quantity: state.config.current_epoch_length(),
            unit: "slot".to_string(),
        },
        security_parameter: BlockHeight {
            quantity: state
                .config
                .security_parameter()
                .ok_or_else(|| ApiError::internal("native genesis parameters unavailable"))?,
            unit: "block".into(),
        },
        active_slot_coefficient: ProgressValue {
            quantity: 100.0
                * state
                    .config
                    .active_slot_coefficient()
                    .ok_or_else(|| ApiError::internal("native genesis parameters unavailable"))?,
            unit: "percent".into(),
        },
        decentralization_level: ProgressValue {
            quantity: 100.0,
            unit: "percent".into(),
        },
        desired_pool_number: parameters.desired_pool_number,
        maximum_collateral_input_count: parameters.max_collateral_inputs,
        minimum_collateral_percentage: parameters.collateral_percentage,
        maximum_token_bundle_size: ByteSize {
            quantity: parameters.max_value_size,
            unit: "byte",
        },
        execution_unit_prices: ExecutionPrices {
            memory_unit_price: parameters.memory_price,
            step_price: parameters.step_price,
        },
        eras: serde_json::Value::Object(eras),
    }))
}

pub async fn get_network_clock(
    State(_state): State<AppState>,
) -> Result<Json<NetworkClockResponse>, ApiError> {
    let offset = quetzal_network::clock::measure().await.ok();
    Ok(Json(NetworkClockResponse {
        status: if offset.is_some() {
            "available"
        } else {
            "unavailable"
        }
        .into(),
        offset: offset.map(|quantity| crate::json::network::OffsetValue {
            quantity,
            unit: "microsecond".into(),
        }),
    }))
}

pub async fn get_blocks_latest_header(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (tip_slot, tip_hash, tip_height) = quetzal_network::NodeClient::chain_header_at_tip(
        &state.config.socket_path,
        state.config.network_magic,
    )
    .await
    .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(serde_json::json!({
        "slot_no": { "quantity": tip_slot, "unit": "slot" },
        "header_hash": hex::encode(tip_hash),
        "block_height": { "quantity": tip_height, "unit": "block" }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn disconnected_network_keeps_native_observation_across_namespaces_and_replaces_it() {
        let directory = tempfile::tempdir().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let config = crate::config::Config {
            socket_path: directory
                .path()
                .join("absent.socket")
                .to_string_lossy()
                .into_owned(),
            network: crate::config::Network::Testnet,
            genesis_start: now - 100,
            slot_millis: 1000,
            epoch_length: 20,
            ..Default::default()
        };
        let mut state = AppState::new(
            quetzal_wallet::WalletService::new(directory.path().join("wallet").to_str().unwrap())
                .unwrap(),
            config,
        )
        .unwrap();
        let byron = state.for_byron();
        *state.chain_tip.write().await = (200_000_000, 1_000_000);
        let snapshot = |slot, height, era| quetzal_network::client::NetworkSnapshot {
            slot,
            height,
            era,
            hash: vec![7; 32],
            timings: vec![quetzal_network::statequery::EraTiming {
                start_slot: 0,
                start_epoch: 0,
                relative_millis: 0,
                slot_millis: 1000,
                epoch_length: 20,
            }],
            genesis: None,
        };
        state.observe_network(snapshot(100, 20, 6)).await;
        let Json(response) = get_network_info(State(byron.clone())).await.unwrap();
        assert_eq!(response.node_tip.absolute_slot_number, Some(100));
        assert_eq!(response.node_tip.height.unwrap().quantity, 20);
        assert_eq!(response.node_era, "conway");
        assert_eq!(response.sync_progress.status, "ready");
        assert!(response.sync_progress.progress.is_none());

        // A subsequent real observation may move backwards during rollback.
        state.observe_network(snapshot(80, 15, 5)).await;
        let Json(response) = get_network_info(State(byron)).await.unwrap();
        assert_eq!(response.node_tip.absolute_slot_number, Some(80));
        assert_eq!(response.node_tip.height.unwrap().quantity, 15);
        assert_eq!(response.node_era, "babbage");

        // Retaining a point does not retain a ready status past its tolerance.
        state.config.sync_tolerance_seconds = 0;
        let Json(response) = get_network_info(State(state)).await.unwrap();
        assert_eq!(response.sync_progress.status, "syncing");
        let progress = response.sync_progress.progress.unwrap();
        assert!(progress.quantity > 0.0 && progress.quantity < 100.0);
        assert_eq!(progress.unit, "percent");
        assert_eq!(response.node_tip.absolute_slot_number, Some(80));
    }

    #[tokio::test]
    async fn disconnected_node_never_inherits_replay_ready_or_a_percentage() {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            socket_path: directory
                .path()
                .join("absent.socket")
                .to_string_lossy()
                .into_owned(),
            ..Default::default()
        };
        let state = AppState::new(
            quetzal_wallet::WalletService::new(directory.path().join("wallet").to_str().unwrap())
                .unwrap(),
            config,
        )
        .unwrap();
        state
            .chain_ready
            .store(true, std::sync::atomic::Ordering::Release);
        *state.chain_tip.write().await = (200000000, 1000000);
        let Json(response) = get_network_info(State(state)).await.unwrap();
        assert_eq!(response.sync_progress.status, "not_responding");
        assert!(response.sync_progress.progress.is_none());
    }
}
