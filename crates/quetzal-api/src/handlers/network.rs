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
    let (tip_slot, tip_height) = *state.chain_tip.read().await;
    let (epoch, slot, time) = state.config.slot_reference(tip_slot);
    let now_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| ApiError::internal(e.to_string()))?
        .as_millis() as u64;
    let wall_slot = if state.config.network == crate::config::Network::Mainnet {
        4492800 + now_millis.saturating_sub(1596059091000) / 1000
    } else {
        now_millis.saturating_sub(state.config.genesis_start * 1000) / state.config.slot_millis
    };
    let (wall_epoch, wall_local_slot, wall_time) = state.config.slot_reference(wall_slot);
    let next_slot = wall_slot + state.config.epoch_length - wall_local_slot;

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
        node_era: "conway".to_string(),
        sync_progress: SyncProgress {
            status: if state.chain_ready.load(std::sync::atomic::Ordering::Acquire) {
                "ready".to_string()
            } else if tip_slot > 0 {
                "syncing".to_string()
            } else {
                "not_responding".to_string()
            },
            progress: None,
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
        let eras = query.era_history().await?;
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
    let genesis = state.config.shelley_genesis.as_ref();
    Ok(Json(NetworkParamsResponse {
        genesis_block_hash: state.config.genesis_hash.clone(),
        blockchain_start_time: state.config.slot_reference(0).2,
        slot_length: SlotLength {
            quantity: state.config.slot_millis as f64 / 1000.0,
            unit: "second".to_string(),
        },
        epoch_length: EpochLength {
            quantity: state.config.epoch_length,
            unit: "slot".to_string(),
        },
        security_parameter: BlockHeight {
            quantity: genesis
                .and_then(|v| v["securityParam"].as_u64())
                .unwrap_or(2160),
            unit: "block".into(),
        },
        active_slot_coefficient: ProgressValue {
            quantity: 100.0
                * genesis
                    .and_then(|v| v["activeSlotsCoeff"].as_f64())
                    .unwrap_or(0.05),
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
