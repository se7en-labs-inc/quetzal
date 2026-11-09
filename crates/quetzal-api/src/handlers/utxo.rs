use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, State},
    Json,
};
use quetzal_primitives::WalletId;

pub async fn get_utxos(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let utxos = state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?;
    let entries: Vec<_> =
        utxos
            .iter()
            .map(|u| serde_json::json!({"ada":{"quantity":u.value.coin.0,"unit":"lovelace"},"assets":u.value.assets.iter().flat_map(|(policy,assets)|assets.iter().map(move |(name,quantity)|serde_json::json!({"policy_id":policy,"asset_name":name,"quantity":quantity}))).collect::<Vec<_>>()}))
            .collect();
    Ok(Json(serde_json::json!({"entries":entries})))
}

pub async fn get_utxo_statistics(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let utxos = state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?;
    let total: u64 = utxos.iter().map(|u| u.value.coin.as_lovelace()).sum();
    let mut distribution = std::collections::BTreeMap::new();
    let bounds: Vec<u64> = (1..=16)
        .map(|power| 10u64.pow(power))
        .chain(std::iter::once(45_000_000_000_000_000))
        .collect();
    for bound in &bounds {
        distribution.insert(bound.to_string(), 0u64);
    }
    for utxo in &utxos {
        let bound = bounds
            .iter()
            .find(|bound| **bound >= utxo.value.coin.0)
            .ok_or_else(|| ApiError::internal("UTxO exceeds maximum supply"))?;
        *distribution
            .get_mut(&bound.to_string())
            .expect("initialized bucket") += 1;
    }
    Ok(Json(serde_json::json!({
        "total": { "quantity": total, "unit": "lovelace" },
        "scale": "log10",
        "distribution": distribution
    })))
}
