use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use quetzal_primitives::{Coin, Value, WalletId};
use quetzal_wallet::payment::ConstructedPayment;
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct MigrationPlanRequest {
    pub passphrase: Option<String>,
    pub addresses: Vec<String>,
}

async fn prepare(
    state: &AppState,
    wallet_id: &str,
    request: &MigrationPlanRequest,
) -> Result<Vec<ConstructedPayment>, ApiError> {
    let id = WalletId::from_hex(wallet_id).map_err(|_| {
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
    if request.addresses.is_empty() {
        return Err(ApiError::bad_request(
            "bad_request",
            "At least one target address is required",
        ));
    }
    let network = if state.config.network == crate::config::Network::Mainnet {
        1
    } else {
        0
    };
    for address in &request.addresses {
        let parsed = pallas_addresses::Address::from_bech32(address)
            .map_err(|_| ApiError::bad_request("bad_request", "Invalid migration destination"))?;
        if parsed.to_vec()[0] & 15 != network {
            return Err(ApiError::bad_request(
                "bad_request",
                "Invalid network discrimination on address",
            ));
        }
    }
    let utxos = state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?;
    if utxos.is_empty() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "nothing_to_migrate",
            "I wasn't able to construct a migration plan. This could be because your wallet is empty, or it could be because the amount of ada in your wallet is insufficient to pay for any of the funds to be migrated. Try adding some ada to your wallet before trying again.",
        ));
    }
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    node.abort().await;
    let parameters = parameters.map_err(|error| ApiError::internal(error.to_string()))?;
    let expiry = state
        .chain_tip
        .read()
        .await
        .0
        .saturating_add(7_200_000 / state.config.current_slot_millis());
    let mut groups = std::collections::VecDeque::from([utxos]);
    let mut built = Vec::new();
    while let Some(inputs) = groups.pop_front() {
        let destination = &request.addresses[built.len() % request.addresses.len()];
        match state.wallet_service.construct_sweep(
            &inputs,
            destination,
            &parameters,
            expiry,
            network,
            None,
        ) {
            Ok(transaction) => built.push(transaction),
            Err(quetzal_wallet::WalletError::TxTooLarge) if inputs.len() > 1 => {
                let middle = inputs.len() / 2;
                groups.push_front(inputs[middle..].to_vec());
                groups.push_front(inputs[..middle].to_vec());
            }
            Err(error) => return Err(ApiError::from(error)),
        }
    }
    Ok(built)
}

pub async fn plan_migration(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<MigrationPlanRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let built = prepare(&state, &wallet_id, &request).await?;
    let mut total = Value::default();
    let mut fee = Coin::ZERO;
    let mut selections = Vec::new();
    let id =
        WalletId::from_hex(&wallet_id).map_err(|error| ApiError::internal(error.to_string()))?;
    let addresses = state
        .wallet_service
        .get_addresses(&id)
        .await
        .map_err(ApiError::from)?;
    let kind = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?
        .wallet_type;
    for transaction in built {
        fee = fee
            .checked_add(transaction.fee)
            .ok_or_else(|| ApiError::internal("Migration fee overflow"))?;
        let inputs: Vec<_> = transaction.inputs.iter().map(|input| {
            let path = addresses.iter().find(|entry|entry.address == input.address).map(|entry|crate::json::transaction::derivation_path(&kind,&entry.path));
            json!({"id":input.id.tx_hash.to_hex(),"index":input.id.output_index,"address":input.address.as_str(),"amount":{"quantity":input.value.coin.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&input.value),"derivation_path":path})
        }).collect();
        for input in &transaction.inputs {
            total = total
                .checked_add(&input.value)
                .ok_or_else(|| ApiError::internal("Migration balance overflow"))?;
        }
        let decoded = pallas_traverse::MultiEraTx::decode(&transaction.cbor)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        let outputs: Vec<_> = decoded.outputs().iter().map(|output| {
            let value = quetzal_wallet::ledger::value(&output.value());
            json!({"address":output.address().map(|address|address.to_string()).unwrap_or_default(),"amount":{"quantity":value.coin.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&value)})
        }).collect();
        selections.push(json!({"inputs":inputs,"outputs":outputs,"change":[],"collateral":[],"withdrawals":[],"deposits_taken":[],"deposits_returned":[]}));
    }
    Ok((
        StatusCode::ACCEPTED,
        Json(
            json!({"selections":selections,"total_fee":{"quantity":fee.0,"unit":"lovelace"},"balance_selected":{"ada":{"quantity":total.coin.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&total)},"balance_leftover":{"ada":{"quantity":0,"unit":"lovelace"},"assets":[]}}),
        ),
    ))
}

pub async fn migrate_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<MigrationPlanRequest>,
) -> Result<
    (
        StatusCode,
        Json<Vec<crate::json::transaction::TransactionResponse>>,
    ),
    ApiError,
> {
    let passphrase = request
        .passphrase
        .clone()
        .ok_or_else(|| ApiError::bad_request("bad_request", "A passphrase is required"))?;
    let built = prepare(&state, &wallet_id, &request).await?;
    let mut responses = Vec::new();
    // Authenticate every transaction before broadcasting the first one.
    let mut signed_transactions = Vec::new();
    for transaction in built {
        let (_, Json(signed)) = super::transactions::sign_transaction(
            State(state.clone()),
            Path(wallet_id.clone()),
            Json(super::transactions::SignRequest {
                passphrase: passphrase.clone(),
                transaction: hex::encode(transaction.cbor),
            }),
        )
        .await?;
        signed_transactions.push(
            signed["transaction"]
                .as_str()
                .ok_or_else(|| ApiError::internal("Missing signed migration"))?
                .to_owned(),
        );
    }
    for transaction in signed_transactions {
        let (_, Json(response)) = super::transactions::submit_transaction(
            State(state.clone()),
            Path(wallet_id.clone()),
            Json(crate::json::transaction::PostTransactionSubmitRequest { transaction }),
        )
        .await?;
        responses.push(response);
    }
    Ok((StatusCode::ACCEPTED, Json(responses)))
}

pub async fn get_migration_plan(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let wallet = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        json!({"balance_available":{"quantity":wallet.balance_available.0,"unit":"lovelace"},"utxo_count":state.wallet_service.available_utxos(&id).await.map_err(ApiError::from)?.len()}),
    ))
}
