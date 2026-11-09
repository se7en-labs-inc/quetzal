use crate::{
    error::ApiError,
    json::wallet::{PostWalletRequest, PutPassphraseRequest, PutWalletRequest, WalletResponse},
    state::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use bip39::{Language, Mnemonic};
use quetzal_primitives::WalletId;

fn mnemonic_to_entropy(words: &[String]) -> Result<Vec<u8>, ApiError> {
    let joined = words.join(" ");
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &joined)
        .map_err(|e| ApiError::bad_request("invalid_mnemonic", format!("Invalid mnemonic: {e}")))?;
    Ok(mnemonic.to_entropy())
}

pub async fn create_wallet(
    State(state): State<AppState>,
    Json(req): Json<PostWalletRequest>,
) -> Result<(StatusCode, Json<WalletResponse>), ApiError> {
    if let Some(encoded) = req.account_public_key {
        let account = hex::decode(&encoded)
            .map_err(|_| ApiError::bad_request("bad_request", "Invalid account public key"))?;
        let wallet = state
            .wallet_service
            .create_watch_wallet(
                req.name,
                &account,
                req.address_pool_gap,
                state.config.network.to_pallas(),
            )
            .await
            .map_err(ApiError::from)?;
        state.start_wallet_worker(wallet.id.clone()).await;
        return Ok((
            StatusCode::CREATED,
            Json(WalletResponse::from_meta(&wallet, &state.config)),
        ));
    }
    let entropy = mnemonic_to_entropy(&req.mnemonic_sentence)?;
    let second_factor = req
        .mnemonic_second_factor
        .as_ref()
        .map(|words| mnemonic_to_entropy(words))
        .transpose()?
        .unwrap_or_default();
    let meta = state
        .wallet_service
        .create_wallet_with_second_factor(
            req.name,
            &entropy,
            req.passphrase.as_bytes(),
            req.address_pool_gap,
            state.config.network.to_pallas(),
            &second_factor,
        )
        .await
        .map_err(ApiError::from)?;
    state.start_wallet_worker(meta.id.clone()).await;
    Ok((
        StatusCode::CREATED,
        Json(WalletResponse::from_meta(&meta, &state.config)),
    ))
}

pub async fn list_wallets(
    State(state): State<AppState>,
) -> Result<Json<Vec<WalletResponse>>, ApiError> {
    let wallets = state
        .wallet_service
        .list_wallets()
        .await
        .map_err(ApiError::from)?;
    let mut responses = Vec::new();
    for metadata in wallets
        .iter()
        .filter(|meta| meta.wallet_type == quetzal_primitives::WalletType::Shelley)
    {
        refresh_reward(&state, &metadata.id).await?;
        let meta = state
            .wallet_service
            .get_wallet(&metadata.id)
            .await
            .map_err(ApiError::from)?;
        let mut response = WalletResponse::from_meta(&meta, &state.config);
        response.apply_staking(
            &state
                .wallet_service
                .staking_state(&meta.id)
                .await
                .map_err(ApiError::from)?,
            &state.config,
            state.config.slot_reference(meta.tip_slot).0,
        )?;
        responses.push(response);
    }
    Ok(Json(responses))
}

pub async fn get_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<WalletResponse>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    refresh_reward(&state, &id).await?;
    let meta = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let mut response = WalletResponse::from_meta(&meta, &state.config);
    response.apply_staking(
        &state
            .wallet_service
            .staking_state(&id)
            .await
            .map_err(ApiError::from)?,
        &state.config,
        state.config.slot_reference(meta.tip_slot).0,
    )?;
    Ok(Json(response))
}

async fn refresh_reward(state: &AppState, id: &WalletId) -> Result<(), ApiError> {
    let public = state
        .wallet_service
        .get_account_xpub_bytes(id)
        .await
        .map_err(ApiError::from)?;
    let hash = quetzal_wallet::staking::stake_hash(&public).map_err(ApiError::from)?;
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        quetzal_network::NodeClient::stake_accounts_at_tip(
            &state.config.socket_path,
            state.config.network_magic,
            &[(0, hash)],
        ),
    )
    .await;
    if let Ok(Ok(accounts)) = result {
        let account = accounts
            .first()
            .ok_or_else(|| ApiError::internal("Missing stake account query result"))?;
        state
            .wallet_service
            .update_reward(id, account.reward_balance.into())
            .map_err(ApiError::from)?;
    }
    Ok(())
}

pub async fn delete_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    state.stop_wallet_worker(&id).await;
    state
        .wallet_service
        .delete_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn update_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<PutWalletRequest>,
) -> Result<Json<WalletResponse>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let meta = if let Some(name) = req.name {
        state
            .wallet_service
            .update_wallet_name(&id, name)
            .await
            .map_err(ApiError::from)?
    } else {
        state
            .wallet_service
            .get_wallet(&id)
            .await
            .map_err(ApiError::from)?
    };
    Ok(Json(WalletResponse::from_meta(&meta, &state.config)))
}

pub async fn update_passphrase(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(value): Json<serde_json::Value>,
) -> Result<StatusCode, ApiError> {
    let req = PutPassphraseRequest::parse(value, false)?;
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    if req.old_passphrase.is_none() {
        let entropy = mnemonic_to_entropy(
            req.mnemonic_sentence
                .as_deref()
                .ok_or_else(|| ApiError::bad_request("bad_request", "Missing mnemonic"))?,
        )?;
        let second_factor = req
            .mnemonic_second_factor
            .as_deref()
            .map(mnemonic_to_entropy)
            .transpose()?
            .unwrap_or_default();
        state
            .wallet_service
            .recover_passphrase(&id, &entropy, &second_factor, req.new_passphrase.as_bytes())
            .await
            .map_err(ApiError::from)?;
        return Ok(StatusCode::NO_CONTENT);
    }
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

/// Select inputs and change with the same ledger constraints as construction.
pub async fn coin_selection(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<super::transactions::ConstructRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (_, Json(constructed)) =
        super::transactions::construct_transaction(State(state), Path(wallet_id), Json(request))
            .await?;
    Ok(Json(constructed["coin_selection"].clone()))
}
