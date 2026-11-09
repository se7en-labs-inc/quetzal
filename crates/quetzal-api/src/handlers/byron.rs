use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use bip39::{Language, Mnemonic};
use quetzal_primitives::WalletId;
use serde::Deserialize;

use crate::{
    error::ApiError,
    json::wallet::{PutPassphraseRequest, WalletResponse},
    state::AppState,
};

// ── Request body ──────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PostByronWalletRequest {
    name: String,
    mnemonic_sentence: Vec<String>,
    passphrase: String,
    style: Option<String>,
}

fn mnemonic_to_entropy(words: &[String]) -> Result<Vec<u8>, ApiError> {
    let joined = words.join(" ");
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &joined)
        .map_err(|e| ApiError::bad_request("invalid_mnemonic", format!("Invalid mnemonic: {e}")))?;
    Ok(mnemonic.to_entropy())
}

fn parse_wallet_id(s: &str) -> Result<WalletId, ApiError> {
    WalletId::from_hex(s).map_err(|_| ApiError::not_found(format!("No Byron wallet with id: {s}")))
}

// ── Handlers ──────────────────────────────────────────────────────────────────
fn byron_response(
    meta: &quetzal_primitives::WalletMeta,
    config: &crate::config::Config,
) -> serde_json::Value {
    let mut value =
        serde_json::to_value(WalletResponse::from_meta(meta, config)).unwrap_or_default();
    if let Some(object) = value.as_object_mut() {
        object.remove("address_pool_gap");
        object.remove("delegation");
    }
    if let Some(balance) = value["balance"].as_object_mut() {
        balance.remove("reward");
    }
    value["discovery"] = serde_json::json!(if meta.wallet_type
        == quetzal_primitives::WalletType::Byron
    {
        "random"
    } else {
        "sequential"
    });
    value
}
pub async fn list_byron_wallets(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let wallets = state
        .wallet_service
        .get_byron_wallets()
        .await
        .map_err(ApiError::from)?;
    Ok(Json(
        wallets
            .iter()
            .map(|wallet| byron_response(wallet, &state.config))
            .collect(),
    ))
}
pub async fn update_byron_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<crate::json::wallet::PutWalletRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    let mut wallet = state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    if let Some(name) = request.name {
        wallet = state
            .wallet_service
            .update_wallet_name(&id, name)
            .await
            .map_err(ApiError::from)?;
    }
    Ok(Json(byron_response(&wallet, &state.config)))
}

pub async fn create_byron_wallet(
    State(state): State<AppState>,
    Json(req): Json<PostByronWalletRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let entropy = mnemonic_to_entropy(&req.mnemonic_sentence)?;
    let meta = state
        .wallet_service
        .create_byron_wallet_style(
            req.name,
            &entropy,
            req.passphrase.as_bytes(),
            if state.config.network == crate::config::Network::Mainnet {
                None
            } else {
                Some(
                    u32::try_from(state.config.network_magic)
                        .map_err(|_| ApiError::internal("Invalid Byron network magic"))?,
                )
            },
            req.style.as_deref().unwrap_or("random"),
        )
        .await
        .map_err(ApiError::from)?;
    state.start_wallet_worker(meta.id.clone()).await;
    Ok((
        StatusCode::CREATED,
        Json(byron_response(&meta, &state.config)),
    ))
}

pub async fn get_byron_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    let meta = state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(byron_response(&meta, &state.config)))
}

pub async fn delete_byron_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    state.stop_wallet_worker(&id).await;
    state
        .wallet_service
        .delete_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_byron_passphrase(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> Result<StatusCode, ApiError> {
    let req = PutPassphraseRequest::parse(value, true)?;
    let id = parse_wallet_id(&wallet_id)?;
    // Verify the wallet is a Byron wallet first.
    let _ = state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    state
        .wallet_service
        .update_passphrase(
            &id,
            req.old_passphrase.as_deref().unwrap_or_default().as_bytes(),
            req.new_passphrase.as_bytes(),
        )
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_byron_addresses(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    let meta = state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let mut addresses = state
        .wallet_service
        .get_known_addresses(&id)
        .await
        .map_err(ApiError::from)?;
    addresses.sort_by_key(|entry| (entry.path.address_ix, entry.path.role.index()));
    let result = addresses
        .iter()
        .map(|entry| {
            byron_address(
                entry,
                meta.wallet_type == quetzal_primitives::WalletType::Byron,
            )
        })
        .collect();
    Ok(Json(result))
}

pub async fn import_byron_address(
    State(state): State<AppState>,
    Path((wallet_id, address_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    state
        .wallet_service
        .import_random_addresses(&id, &[address_id], byron_magic(&state)?)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

fn byron_address(entry: &quetzal_primitives::AddressEntry, random: bool) -> serde_json::Value {
    let path = if random {
        [entry.path.account_ix, entry.path.address_ix]
            .into_iter()
            .map(|index| {
                if index >= 0x80000000 {
                    format!("{}H", index - 0x80000000)
                } else {
                    index.to_string()
                }
            })
            .collect::<Vec<_>>()
    } else {
        vec![
            "44H".into(),
            "1815H".into(),
            format!("{}H", entry.path.account_ix),
            entry.path.role.index().to_string(),
            entry.path.address_ix.to_string(),
        ]
    };
    serde_json::json!({"id": entry.address.as_str(), "state": if entry.state == quetzal_primitives::AddressState::Used {"used"} else {"unused"}, "derivation_path": path})
}

fn byron_magic(state: &AppState) -> Result<Option<u32>, ApiError> {
    if state.config.network == crate::config::Network::Mainnet {
        Ok(None)
    } else {
        u32::try_from(state.config.network_magic)
            .map(Some)
            .map_err(|_| ApiError::internal("Invalid Byron network magic"))
    }
}

#[derive(Deserialize)]
pub struct RandomAddressRequest {
    passphrase: String,
    address_index: Option<u32>,
}

pub async fn create_random_address(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<RandomAddressRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    if req.address_index.is_some_and(|index| index < 0x80000000) {
        return Err(ApiError::bad_request(
            "bad_request",
            "A random address index must be between 2147483648 and 4294967295",
        ));
    }
    let entry = state
        .wallet_service
        .create_random_address(
            &id,
            req.passphrase.as_bytes(),
            req.address_index,
            byron_magic(&state)?,
        )
        .await
        .map_err(ApiError::from)?;
    Ok((StatusCode::CREATED, Json(byron_address(&entry, true))))
}

#[derive(Deserialize)]
pub struct ImportAddressesRequest {
    addresses: Vec<String>,
}

pub async fn import_byron_addresses(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<ImportAddressesRequest>,
) -> Result<StatusCode, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    state
        .wallet_service
        .import_random_addresses(&id, &req.addresses, byron_magic(&state)?)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_byron_assets(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    super::assets::list_assets(State(state), Path(wallet_id)).await
}

pub async fn get_byron_asset_by_policy(
    State(state): State<AppState>,
    Path((wallet_id, policy_id)): Path<(String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::assets::get_asset_by_policy(State(state), Path((wallet_id, policy_id))).await
}

pub async fn get_byron_asset(
    State(state): State<AppState>,
    Path((wallet_id, policy_id, asset_name)): Path<(String, String, String)>,
) -> Result<Json<serde_json::Value>, ApiError> {
    super::assets::get_asset(State(state), Path((wallet_id, policy_id, asset_name))).await
}

pub async fn get_byron_utxos(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    super::utxo::get_utxos(State(state), Path(wallet_id)).await
}

pub async fn get_byron_utxo_statistics(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wallet_id(&wallet_id)?;
    state
        .wallet_service
        .get_byron_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    super::utxo::get_utxo_statistics(State(state), Path(wallet_id)).await
}

pub async fn list_byron_transactions(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(query): Query<super::transactions::TxListQuery>,
) -> Result<Json<Vec<crate::json::transaction::TransactionResponse>>, ApiError> {
    super::transactions::list_transactions(State(state), Path(wallet_id), Query(query)).await
}

pub async fn get_byron_transaction(
    State(state): State<AppState>,
    Path((wallet_id, tx_id)): Path<(String, String)>,
) -> Result<Json<crate::json::transaction::TransactionResponse>, ApiError> {
    super::transactions::get_transaction(State(state), Path((wallet_id, tx_id))).await
}
