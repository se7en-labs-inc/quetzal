use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use quetzal_primitives::WalletId;
use serde_json::{json, Value};

fn wallet_id(text: &str) -> Result<WalletId, ApiError> {
    WalletId::from_hex(text).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })
}
fn policy_id(text: &str) -> Result<Vec<u8>, ApiError> {
    let bytes = hex::decode(text).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "policy id should be a hex-encoded string of 56 characters",
        )
    })?;
    if bytes.len() != 28 {
        return Err(ApiError::bad_request(
            "bad_request",
            "policy id should be a hex-encoded string of 56 characters",
        ));
    }
    Ok(bytes)
}
fn render(policy: &str, name: &str) -> Result<Value, ApiError> {
    let policy_bytes = policy_id(policy)?;
    let name_bytes = hex::decode(name)
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid asset name"))?;
    let fingerprint = quetzal_address::codec::fingerprint(&policy_bytes, &name_bytes)
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid asset name"))?;
    Ok(json!({"policy_id":policy,"asset_name":name,"fingerprint":fingerprint}))
}
pub async fn list_assets(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<Value>>, ApiError> {
    let id = wallet_id(&id)?;
    let wallet = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        wallet
            .assets_total
            .iter()
            .map(|asset| render(&asset.policy_id, &asset.asset_name))
            .collect::<Result<_, _>>()?,
    ))
}
pub async fn get_asset_by_policy(
    State(state): State<AppState>,
    Path((id, policy)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    get_asset(State(state), Path((id, policy, String::new()))).await
}
pub async fn get_asset(
    State(state): State<AppState>,
    Path((id, policy, name)): Path<(String, String, String)>,
) -> Result<Json<Value>, ApiError> {
    let id = wallet_id(&id)?;
    let normalized_policy = hex::encode(policy_id(&policy)?);
    let name_bytes = hex::decode(&name)
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid asset name"))?;
    if name_bytes.len() > 32 {
        return Err(ApiError::bad_request(
            "bad_request",
            "Asset name exceeds 32 bytes",
        ));
    }
    let normalized_name = hex::encode(name_bytes);
    let wallet = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    if !wallet
        .assets_total
        .iter()
        .any(|asset| asset.policy_id == normalized_policy && asset.asset_name == normalized_name)
    {
        return Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "asset_not_present",
            "The requested asset is not present in the wallet.",
        ));
    }
    Ok(Json(render(&normalized_policy, &normalized_name)?))
}
