use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use pallas_crypto::hash::Hasher;
use quetzal_primitives::WalletId;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct KeyFormatQuery {
    pub format: Option<String>,
    pub hash: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct PostAccountKeyRequest {
    pub passphrase: String,
    pub format: Option<String>,
    pub purpose: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SignatureRequest {
    pub passphrase: String,
    pub metadata: Option<serde_json::Value>,
    pub payload: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PolicyKeyRequest {
    pub passphrase: String,
    pub format: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PolicyIdRequest {
    pub policy_script_template: serde_json::Value,
}

fn parse_wid(s: &str) -> Result<WalletId, ApiError> {
    WalletId::from_hex(s)
        .map_err(|_| ApiError::bad_request("bad_request", format!("No wallet with id: {s}")))
}

fn parse_role(role: &str) -> Result<u32, ApiError> {
    match role {
        "utxo_external" => Ok(0),
        "utxo_internal" => Ok(1),
        "mutable_account" => Ok(2),
        "multisig_script" => Ok(3),
        _ => Err(ApiError::bad_request(
            "invalid_role",
            format!("Unknown role: {role}"),
        )),
    }
}

fn role_hrp_prefix(role: u32) -> &'static str {
    match role {
        2 => "stake",
        _ => "addr",
    }
}

fn encode_bech32(hrp_str: &str, data: &[u8]) -> String {
    use bech32::{Bech32, Hrp};
    let hrp = Hrp::parse(hrp_str).expect("valid hrp");
    bech32::encode::<Bech32>(hrp, data).expect("bech32 encode")
}

fn encode_xpub(xpub: &[u8; 64], hrp_prefix: &str, extended: bool) -> String {
    if extended {
        encode_bech32(&format!("{hrp_prefix}_xvk"), xpub)
    } else {
        encode_bech32(&format!("{hrp_prefix}_vk"), &xpub[..32])
    }
}

/// GET /v2/wallets/:id/keys
pub async fn get_wallet_keys(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(q): Query<KeyFormatQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let xpub = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let extended = q.format.as_deref() == Some("extended");
    Ok(Json(serde_json::Value::String(encode_xpub(
        &xpub, "acct", extended,
    ))))
}

/// POST /v2/wallets/:id/keys/:index
pub async fn get_account_key(
    State(state): State<AppState>,
    Path((wallet_id, index)): Path<(String, String)>,
    Json(req): Json<PostAccountKeyRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_wid(&wallet_id)?;
    let acct_ix: u32 = index
        .strip_suffix('H')
        .ok_or_else(|| ApiError::bad_request("bad_request", "Account index must be hardened"))?
        .parse()
        .map_err(|_| {
            ApiError::bad_request(
                "invalid_index",
                "Account index must be a non-negative integer",
            )
        })?;
    if acct_ix >= quetzal_crypto::HARDENED {
        return Err(ApiError::bad_request(
            "bad_request",
            "Account index exceeds the hardened index range",
        ));
    }
    if req.purpose.as_deref().is_some_and(|p| p != "1852H") {
        return Err(ApiError::bad_request(
            "bad_request",
            "Unsupported account purpose",
        ));
    }
    let xpub = state
        .wallet_service
        .derive_account_xpub_at(&id, req.passphrase.as_bytes(), acct_ix)
        .await
        .map_err(ApiError::from)?;
    let extended = req.format.as_deref() == Some("extended");
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::Value::String(encode_xpub(
            &xpub, "acct", extended,
        ))),
    ))
}

/// GET /v2/wallets/:id/keys/:role/:index
pub async fn get_derived_key(
    State(state): State<AppState>,
    Path((wallet_id, role, index)): Path<(String, String, String)>,
    Query(q): Query<KeyFormatQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wid(&wallet_id)?;
    let role_ix = parse_role(&role)?;
    let ix: u32 = index.parse().map_err(|_| {
        ApiError::bad_request("invalid_index", "Index must be a non-negative integer")
    })?;

    let acct_xpub = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;

    // Soft-derive role/ix from account xpub (no passphrase needed)
    let acct_pub = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&acct_xpub)
        .map_err(|_| ApiError::internal("Failed to decode account xpub"))?;
    let role_pub = acct_pub
        .derive(role_ix)
        .map_err(|_| ApiError::internal("Role derivation failed"))?;
    let addr_pub = role_pub
        .derive(ix)
        .map_err(|_| ApiError::internal("Index derivation failed"))?;
    let key_bytes = addr_pub.to_raw_bytes();

    let hrp_prefix = role_hrp_prefix(role_ix);
    let extended = q.format.as_deref() == Some("extended");
    let result = if q.hash.unwrap_or(false) {
        // Return bech32-encoded Blake2b-224 hash of the verification key
        let vk = addr_pub.to_vk_bytes();
        let hash: pallas_crypto::hash::Hash<28> = Hasher::<224>::hash(&vk);
        let hrp_suffix = if role_ix == 2 {
            "stake_vkh"
        } else {
            "addr_vkh"
        };
        encode_bech32(hrp_suffix, hash.as_ref())
    } else {
        encode_xpub(&key_bytes, hrp_prefix, extended)
    };
    Ok(Json(serde_json::Value::String(result)))
}

/// POST /v2/wallets/:id/signatures/:role/:index
pub async fn sign_with_key(
    State(state): State<AppState>,
    Path((wallet_id, role, index)): Path<(String, String, String)>,
    Json(req): Json<SignatureRequest>,
) -> Result<impl axum::response::IntoResponse, ApiError> {
    let id = parse_wid(&wallet_id)?;
    let role_ix = parse_role(&role)?;
    let ix: u32 = index.parse().map_err(|_| {
        ApiError::bad_request("invalid_index", "Index must be a non-negative integer")
    })?;

    // Get the payload to sign
    let payload_bytes = match &req.payload {
        Some(hex_str) => hex::decode(hex_str)
            .map_err(|_| ApiError::bad_request("invalid_payload", "Payload must be hex-encoded"))?,
        None => {
            let metadata =
                quetzal_wallet::metadata::encode(req.metadata.as_ref().ok_or_else(|| {
                    ApiError::bad_request("bad_request", "metadata field is required")
                })?)
                .map_err(|_| {
                    ApiError::bad_request("bad_request", "Invalid transaction metadata")
                })?;
            Hasher::<256>::hash(&metadata).to_vec()
        }
    };

    let (_, sig) = state
        .wallet_service
        .sign_with_key(&id, req.passphrase.as_bytes(), role_ix, ix, &payload_bytes)
        .await
        .map_err(ApiError::from)?;

    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
        sig.to_vec(),
    ))
}

/// GET /v2/wallets/:id/stake-keys
pub async fn get_stake_keys(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wid(&wallet_id)?;
    let meta = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let xpub = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let account = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&xpub)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let stake = account
        .derive(2)
        .and_then(|role| role.derive(0))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let own = (0, *Hasher::<224>::hash(&stake.to_vk_bytes()));
    let utxos = state
        .wallet_service
        .get_utxos(&id)
        .await
        .map_err(ApiError::from)?;
    let mut distribution = quetzal_wallet::stake::distribution(&utxos).map_err(ApiError::from)?;
    distribution.credentials.entry(own).or_default();
    let credentials: Vec<_> = distribution.credentials.keys().copied().collect();
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let accounts = quetzal_network::StateQuerier::new(node.statequery())
        .stake_accounts(&credentials)
        .await;
    node.abort().await;
    let accounts = accounts.map_err(|e| ApiError::internal(e.to_string()))?;
    let staking = state
        .wallet_service
        .staking_state(&id)
        .await
        .map_err(ApiError::from)?;
    let mut wallet = crate::json::wallet::WalletResponse::from_meta(&meta, &state.config);
    wallet.apply_staking(
        &staking,
        &state.config,
        state.config.slot_reference(meta.tip_slot).0,
    )?;
    let delegation =
        serde_json::to_value(wallet.delegation).map_err(|e| ApiError::internal(e.to_string()))?;
    let amount = |coin: u64| serde_json::json!({"quantity": coin, "unit": "lovelace"});
    let mut ours = Vec::new();
    let mut foreign = Vec::new();
    for (credential, account) in credentials.into_iter().zip(accounts) {
        let mut address = vec![
            (if credential.0 == 0 { 0xe0 } else { 0xf0 })
                | if state.config.network == crate::config::Network::Mainnet {
                    1
                } else {
                    0
                },
        ];
        address.extend(credential.1);
        let key = quetzal_address::codec::bech32_encode(
            if state.config.network == crate::config::Network::Mainnet {
                "stake"
            } else {
                "stake_test"
            },
            &address,
        )
        .map_err(|e| ApiError::internal(e.to_string()))?;
        let mut entry = serde_json::json!({
            "key": key,
            "stake": amount(distribution.credentials[&credential].checked_add(account.reward_balance)
                .ok_or_else(|| ApiError::internal("stake coin overflow"))?),
            "reward_balance": amount(account.reward_balance),
        });
        if credential == own {
            entry["index"] = serde_json::json!(0);
            entry["delegation"] = delegation.clone();
            ours.push(entry);
        } else {
            foreign.push(entry);
        }
    }
    Ok(Json(serde_json::json!({
        "ours": ours,
        "foreign": foreign,
        "none": {"stake": amount(distribution.unassigned)},
    })))
}

/// POST /v2/wallets/:id/policy-id
pub async fn get_policy_id(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<PolicyIdRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    quetzal_wallet::policy::script(&req.policy_script_template, &[0; 28])
        .map_err(ApiError::from)?;
    let id = parse_wid(&wallet_id)?;
    let xpub = state
        .wallet_service
        .policy_xpub(&id)
        .await
        .map_err(ApiError::from)?;
    let policy_pub = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&xpub)
        .map_err(|_| ApiError::internal("Failed to decode policy xpub"))?;
    let key_hash = Hasher::<224>::hash(&policy_pub.to_vk_bytes());
    let script = quetzal_wallet::policy::script(&req.policy_script_template, &key_hash)
        .map_err(ApiError::from)?;
    let policy = script
        .hash()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({ "policy_id": hex::encode(policy) })),
    ))
}

/// GET /v2/wallets/:id/policy-key
pub async fn get_policy_key(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(q): Query<KeyFormatQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_wid(&wallet_id)?;
    let xpub = state
        .wallet_service
        .policy_xpub(&id)
        .await
        .map_err(ApiError::from)?;
    let result = if q.hash.unwrap_or(false) {
        let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&xpub)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        encode_bech32(
            "policy_vkh",
            Hasher::<224>::hash(&public.to_vk_bytes()).as_ref(),
        )
    } else {
        encode_xpub(&xpub, "policy", false)
    };
    Ok(Json(serde_json::Value::String(result)))
}

/// POST /v2/wallets/:id/policy-key
pub async fn post_policy_key(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(q): Query<KeyFormatQuery>,
    Json(req): Json<PolicyKeyRequest>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_wid(&wallet_id)?;
    let xpub = state
        .wallet_service
        .write_policy_xpub(&id, req.passphrase.as_bytes())
        .await
        .map_err(ApiError::from)?;
    let result = if q.hash.unwrap_or(false) {
        let policy_pub = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&xpub)
            .map_err(|_| ApiError::internal("Failed to decode policy xpub"))?;
        let vk = policy_pub.to_vk_bytes();
        let hash: pallas_crypto::hash::Hash<28> = Hasher::<224>::hash(&vk);
        encode_bech32("policy_vkh", hash.as_ref())
    } else if req.format.as_deref() == Some("extended") {
        encode_xpub(&xpub, "policy", true)
    } else {
        encode_xpub(&xpub, "policy", false)
    };
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::Value::String(result)),
    ))
}
