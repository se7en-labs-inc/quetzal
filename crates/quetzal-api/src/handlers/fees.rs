use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use quetzal_primitives::WalletId;

pub async fn estimate_fees(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<super::transactions::ConstructRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    use pallas_primitives::{conway, Fragment};
    let (_, Json(constructed)) = super::transactions::construct_transaction(
        State(state.clone()),
        Path(wallet_id),
        Json(request),
    )
    .await?;
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    node.abort().await;
    let parameters = parameters.map_err(|e| ApiError::internal(e.to_string()))?;
    let encoded = hex::decode(
        constructed["transaction"]
            .as_str()
            .ok_or_else(|| ApiError::internal("Missing transaction"))?,
    )
    .map_err(|e| ApiError::internal(e.to_string()))?;
    let tx =
        conway::Tx::decode_fragment(&encoded).map_err(|e| ApiError::internal(e.to_string()))?;
    let count = constructed["coin_selection"]["outputs"]
        .as_array()
        .map_or(0, Vec::len);
    let mut minimum_coins = Vec::new();
    for output in tx.transaction_body.outputs.iter().take(count) {
        let encoded = pallas_codec::minicbor::to_vec(output)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let minimum =
            quetzal_wallet::minimum_coin::minimum_coin(&encoded, parameters.ada_per_utxo_byte)
                .map_err(ApiError::internal)?;
        let minimum =
            serde_json::from_str::<serde_json::Value>(&minimum.wallet_minimum_coin.to_string())
                .map_err(|e| ApiError::internal(e.to_string()))?;
        minimum_coins.push(serde_json::json!({"quantity":minimum,"unit":"lovelace"}));
    }
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(
            serde_json::json!({"estimated_min":constructed["fee"],"estimated_max":constructed["fee"],"minimum_coins":minimum_coins,"deposit":{"quantity":0,"unit":"lovelace"}}),
        ),
    ))
}

pub async fn get_delegation_fees(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    if state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?
        .is_empty()
    {
        return Err(ApiError::new(axum::http::StatusCode::FORBIDDEN,"no_utxos_available","Unable to create a transaction because the wallet has no unspent transaction outputs (UTxOs) available. A transaction must spend at least one UTxO in order to be accepted for inclusion on the Cardano blockchain."));
    }
    use pallas_primitives::{conway, Fragment};
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    node.abort().await;
    let parameters = parameters.map_err(|error| ApiError::internal(error.to_string()))?;
    let prototype = quetzal_wallet::balancing::fee_prototype().map_err(ApiError::from)?;
    let balanced = state
        .wallet_service
        .balance_partial(&id, &prototype, &parameters)
        .await
        .map_err(ApiError::from)?;
    let transaction = conway::Tx::decode_fragment(&balanced)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let fee = transaction.transaction_body.fee;
    Ok(Json(serde_json::json!({
        "estimated_min": { "quantity": fee, "unit": "lovelace" },
        "estimated_max": { "quantity": fee, "unit": "lovelace" },
        "minimum_coins": [],
        "deposit": { "quantity": parameters.key_deposit, "unit": "lovelace" }
    })))
}
