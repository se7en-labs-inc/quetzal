use crate::{error::ApiError, json::transaction::TransactionResponse, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct PoolListQuery {
    pub stake: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DelegationRequest {
    pub passphrase: String,
}

pub async fn list_stake_pools(
    State(state): State<AppState>,
    Query(q): Query<PoolListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let stake = q
        .stake
        .as_deref()
        .map(|value| {
            quetzal_primitives::quantity::parse_u64(value).ok_or_else(|| {
                ApiError::bad_request(
                    "bad_request",
                    "Error parsing query parameter stake failed: Expecting natural number",
                )
            })
        })
        .transpose()?
        .unwrap_or(0);
    Ok(Json(super::pool_catalogue::list(&state, stake).await?))
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    GcStakePools,
}

#[derive(Deserialize)]
pub struct MaintenanceRequest {
    maintenance_action: MaintenanceAction,
}

pub async fn trigger_pool_maintenance(
    State(state): State<AppState>,
    Json(req): Json<MaintenanceRequest>,
) -> Result<StatusCode, ApiError> {
    match req.maintenance_action {
        MaintenanceAction::GcStakePools => super::pool_catalogue::schedule_gc(state),
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_pool_maintenance(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let source = state
        .wallet_service
        .pool_metadata_source()
        .map_err(ApiError::from)?;
    let status = if source == "none" || source == "direct" {
        json!({"status":"not_applicable"})
    } else if let Some(gc) = state.wallet_service.pool_gc().map_err(ApiError::from)? {
        json!({"status":"has_run","last_run":gc.last_run})
    } else {
        json!({"status":"not_started"})
    };
    Ok(Json(json!({"gc_stake_pools":status})))
}

pub async fn join_stake_pool(
    State(state): State<AppState>,
    Path((pool_id, wallet_id)): Path<(String, String)>,
    Json(req): Json<DelegationRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
    super::transactions::construct_sign_submit(
        state,
        wallet_id,
        super::transactions::ConstructRequest {
            delegations: Some(vec![super::transactions::DelegationAction::Join {
                pool: pool_id,
                stake_key_index: "0H".into(),
            }]),
            ..Default::default()
        },
        req.passphrase,
        false,
    )
    .await
}

pub async fn quit_stake_pool(
    State(state): State<AppState>,
    Path((pool_id, wallet_id)): Path<(String, String)>,
    Json(req): Json<DelegationRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
    if pool_id != "*" {
        return Err(ApiError::bad_request(
            "bad_request",
            "Expected * for pool ID",
        ));
    }
    super::transactions::construct_sign_submit(
        state,
        wallet_id,
        super::transactions::ConstructRequest::default(),
        req.passphrase,
        true,
    )
    .await
}
