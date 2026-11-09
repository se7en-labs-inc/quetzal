use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use bip39::{Language, Mnemonic};
use quetzal_crypto::Bip32PublicKey;
use quetzal_primitives::{WalletId, WalletMeta};
use std::collections::HashMap;

fn mnemonic_to_entropy(words: &[String]) -> Result<Vec<u8>, ApiError> {
    let joined = words.join(" ");
    let mnemonic = Mnemonic::parse_in_normalized(Language::English, &joined)
        .map_err(|e| ApiError::bad_request("invalid_mnemonic", format!("Invalid mnemonic: {e}")))?;
    Ok(mnemonic.to_entropy())
}

fn parse_shared_wid(s: &str) -> Result<WalletId, ApiError> {
    WalletId::from_hex(s).map_err(|_| ApiError::not_found(format!("No shared wallet with id: {s}")))
}

fn parse_account_index(s: &str) -> Result<u32, ApiError> {
    let num = s.trim_end_matches(['H', '\'']);
    num.parse::<u32>().map_err(|_| {
        ApiError::bad_request(
            "invalid_account_index",
            format!("invalid account index: {s}"),
        )
    })
}

/// Decode a cosigner xpub: accept hex (128 chars = 64 bytes) or bech32.
fn parse_cosigner_xpub(s: &str) -> Result<[u8; 64], ApiError> {
    let bytes = if let Ok(bytes) = hex::decode(s) {
        bytes
    } else {
        let (hrp, bytes) = quetzal_address::codec::bech32_decode(s).map_err(|_| {
            ApiError::bad_request("invalid_xpub", "Invalid cosigner extended public key")
        })?;
        if hrp != "acct_shared_xvk" {
            return Err(ApiError::bad_request(
                "invalid_xpub",
                "Expected acct_shared_xvk",
            ));
        }
        bytes
    };
    bytes
        .try_into()
        .map_err(|_| ApiError::bad_request("invalid_xpub", "xpub must be 64 bytes"))
}
fn encode_xpub(xpub: &[u8; 64]) -> String {
    use bech32::{Bech32, Hrp};
    let hrp = Hrp::parse("acct_shared_xvk").expect("valid hrp");
    bech32::encode::<Bech32>(hrp, xpub).expect("bech32 encode")
}

fn shared_wallet_to_json(
    meta: &WalletMeta,
    payment_template: Option<&serde_json::Value>,
    delegation_template: Option<&serde_json::Value>,
    config: &crate::config::Config,
) -> serde_json::Value {
    let mut body =
        serde_json::to_value(crate::json::wallet::WalletResponse::from_meta(meta, config))
            .expect("wallet response");
    body["account_index"] = serde_json::json!(format!("{}H", meta.account_index));
    body["payment_script_template"] = payment_template.cloned().unwrap_or_default();
    if let Some(template) = delegation_template {
        body["delegation_script_template"] = template.clone();
    }
    if !payment_template.is_some_and(quetzal_wallet::wallet::template_is_complete)
        || delegation_template
            .is_some_and(|template| !quetzal_wallet::wallet::template_is_complete(template))
    {
        for key in ["balance", "assets", "delegation", "tip", "passphrase"] {
            body.as_object_mut().expect("wallet object").remove(key);
        }
        body["state"] = serde_json::json!({"status":"incomplete"});
    }
    body
}

/// POST /v2/shared-wallets
pub async fn create_shared_wallet(
    State(state): State<AppState>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let name = req
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("missing_name", "name is required"))?
        .to_string();

    let words: Vec<String> = req
        .get("mnemonic_sentence")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .ok_or_else(|| {
            ApiError::bad_request("missing_mnemonic", "mnemonic_sentence is required")
        })?;

    let passphrase = req
        .get("passphrase")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("missing_passphrase", "passphrase is required"))?;

    let account_ix = parse_account_index(
        req.get("account_index")
            .and_then(|v| v.as_str())
            .unwrap_or("0H"),
    )?;

    let payment_template = req
        .get("payment_script_template")
        .ok_or_else(|| {
            ApiError::bad_request("missing_template", "payment_script_template is required")
        })?
        .clone();

    let delegation_template = req.get("delegation_script_template").cloned();

    let cosigners_json = payment_template
        .get("cosigners")
        .and_then(|v| v.as_object())
        .ok_or_else(|| {
            ApiError::bad_request(
                "missing_cosigners",
                "payment_script_template.cosigners is required",
            )
        })?;

    let mut cosigners: HashMap<String, Option<[u8; 64]>> = HashMap::new();
    for (cosigner_id, xpub_value) in cosigners_json {
        let xpub_str = xpub_value.as_str().ok_or_else(|| {
            ApiError::bad_request("invalid_cosigner", "cosigner xpub must be a string")
        })?;
        let xpub_opt = if xpub_str == "self" {
            None
        } else {
            Some(parse_cosigner_xpub(xpub_str)?)
        };
        cosigners.insert(cosigner_id.clone(), xpub_opt);
    }

    let entropy = mnemonic_to_entropy(&words)?;
    let meta = state
        .wallet_service
        .create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
            name,
            entropy: &entropy,
            passphrase: passphrase.as_bytes(),
            account_ix,
            cosigners,
            payment_template: &payment_template,
            delegation_template: delegation_template.as_ref(),
            network: state.config.network.to_pallas(),
        })
        .await
        .map_err(ApiError::from)?;

    let Json(body) = get_shared_wallet(State(state.clone()), Path(meta.id.to_hex())).await?;
    if body["state"]["status"] != "incomplete" {
        state.start_wallet_worker(meta.id.clone()).await;
    }
    Ok((StatusCode::CREATED, Json(body)))
}

/// GET /v2/shared-wallets/:id
pub async fn list_shared_wallets(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let wallets = state
        .wallet_service
        .get_shared_wallets()
        .await
        .map_err(ApiError::from)?;
    let mut result = Vec::new();
    for wallet in wallets {
        let Json(body) = get_shared_wallet(State(state.clone()), Path(wallet.id.to_hex())).await?;
        result.push(body);
    }
    Ok(Json(result))
}

pub async fn rename_shared_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(request): Json<super::super::json::wallet::PutWalletRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    if let Some(name) = request.name {
        state
            .wallet_service
            .update_wallet_name(&id, name)
            .await
            .map_err(ApiError::from)?;
    }
    get_shared_wallet(State(state), Path(wallet_id)).await
}

pub async fn get_shared_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    let meta = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let payment = state
        .wallet_service
        .get_script_template(&id, b"payment_script")
        .await
        .map_err(ApiError::from)?;
    let delegation = state
        .wallet_service
        .get_script_template(&id, b"delegation_script")
        .await
        .map_err(ApiError::from)?;
    Ok(Json(shared_wallet_to_json(
        &meta,
        payment.as_ref(),
        delegation.as_ref(),
        &state.config,
    )))
}

/// DELETE /v2/shared-wallets/:id
pub async fn delete_shared_wallet(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state.stop_wallet_worker(&id).await;
    state
        .wallet_service
        .delete_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /v2/shared-wallets/:id/addresses
pub async fn list_shared_addresses(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(query): Query<super::addresses::AddressListQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    if query
        .state
        .as_deref()
        .is_some_and(|s| s != "used" && s != "unused")
    {
        return Err(ApiError::bad_request("bad_request", "Error parsing query parameter state failed: Unable to decode the given text value. Please specify one of the following values: used, unused."));
    }
    let id = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let mut addrs = state
        .wallet_service
        .get_addresses(&id)
        .await
        .map_err(ApiError::from)?;
    addrs.sort_by_key(|entry| (entry.path.address_ix, entry.path.role.index()));
    let resp: Vec<serde_json::Value> = addrs
        .iter()
        .filter(|entry| {
            entry.path.role == quetzal_primitives::AddressRole::External
                || entry.state == quetzal_primitives::AddressState::Used
        })
        .filter(|entry| {
            query.state.as_deref().is_none_or(|state| {
                (state == "used") == (entry.state == quetzal_primitives::AddressState::Used)
            })
        })
        .map(|a| {
            serde_json::json!({
                "id": a.address.as_str(),
                "state": a.state,
                "derivation_path": [
                    format!("{}H", 1854),
                    format!("{}H", 1815),
                    format!("{}H", a.path.account_ix),
                    format!("{}", a.path.role.index()),
                    format!("{}", a.path.address_ix),
                ]
            })
        })
        .collect();
    Ok(Json(resp))
}

/// GET /v2/shared-wallets/:id/utxo
pub async fn get_shared_utxos(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    super::utxo::get_utxos(State(state), Path(wallet_id)).await
}

/// GET /v2/shared-wallets/:id/utxo/statistics
pub async fn get_shared_utxo_statistics(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    super::utxo::get_utxo_statistics(State(state), Path(wallet_id)).await
}

/// GET /v2/shared-wallets/:id/transactions
pub async fn list_shared_transactions(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(query): Query<super::transactions::TxListQuery>,
) -> Result<Json<Vec<crate::json::transaction::TransactionResponse>>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    super::transactions::list_transactions(State(state), Path(wallet_id), Query(query)).await
}

/// GET /v2/shared-wallets/:id/transactions/:tx_id
pub async fn get_shared_transaction(
    State(state): State<AppState>,
    Path((wallet_id, tx_id)): Path<(String, String)>,
) -> Result<Json<crate::json::transaction::TransactionResponse>, ApiError> {
    let wid = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&wid)
        .await
        .map_err(ApiError::from)?;
    super::transactions::get_transaction(State(state), Path((wallet_id, tx_id))).await
}

/// POST /v2/shared-wallets/:id/transactions/construct
pub async fn construct_shared_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<super::transactions::ConstructRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .require_shared_active(&id)
        .await
        .map_err(ApiError::from)?;
    let (status, Json(mut response)) =
        super::transactions::construct_transaction(State(state), Path(wallet_id), Json(req))
            .await?;
    if let Some(selection) = response["coin_selection"].as_object_mut() {
        for field in ["inputs", "change"] {
            for entry in selection
                .get_mut(field)
                .and_then(|v| v.as_array_mut())
                .into_iter()
                .flatten()
            {
                if let Some(path) = entry["derivation_path"].as_array_mut() {
                    path[0] = serde_json::json!("1854H");
                }
            }
        }
    }
    Ok((status, Json(response)))
}

/// POST /v2/shared-wallets/:id/transactions/sign
pub async fn sign_shared_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<super::transactions::SignRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .require_shared_active(&id)
        .await
        .map_err(ApiError::from)?;
    super::transactions::sign_transaction(State(state), Path(wallet_id), Json(req)).await
}

/// POST /v2/shared-wallets/:id/transactions/decode
pub async fn decode_shared_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<super::transactions::DecodeRequest>,
) -> Result<axum::response::Response, ApiError> {
    use axum::response::IntoResponse;
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    // Match the pinned API's unsupported shared decryption response. Ordinary
    // shared decoding still inspects the encrypted metadata without a password.
    if req.decrypt_metadata.is_some() {
        return Ok((
            StatusCode::INTERNAL_SERVER_ERROR,
            [(axum::http::header::CONTENT_TYPE, "text/plain")],
            "Something went wrong",
        )
            .into_response());
    }
    super::transactions::decode_transaction(State(state), Path(wallet_id), Json(req))
        .await
        .map(IntoResponse::into_response)
}

/// POST /v2/shared-wallets/:id/transactions/submit
pub async fn submit_shared_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<crate::json::transaction::PostTransactionSubmitRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .require_shared_active(&id)
        .await
        .map_err(ApiError::from)?;
    use pallas_primitives::{conway, Fragment};
    let raw = hex::decode(&req.transaction)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?;
    let tx = conway::Tx::decode_fragment(&raw)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?;
    fn missing(
        script: &conway::NativeScript,
        keys: &std::collections::HashSet<pallas_crypto::hash::Hash<28>>,
    ) -> usize {
        match script {
            conway::NativeScript::ScriptPubkey(hash) => usize::from(!keys.contains(hash)),
            conway::NativeScript::ScriptAll(nodes) => nodes.iter().map(|n| missing(n, keys)).sum(),
            conway::NativeScript::ScriptAny(nodes) => {
                nodes.iter().map(|n| missing(n, keys)).min().unwrap_or(0)
            }
            conway::NativeScript::ScriptNOfK(required, nodes) => {
                let mut costs: Vec<_> = nodes.iter().map(|n| missing(n, keys)).collect();
                costs.sort_unstable();
                costs.into_iter().take(*required as usize).sum()
            }
            _ => 0,
        }
    }
    let witnesses = tx.transaction_witness_set.vkeywitness.as_ref();
    let keys = witnesses
        .into_iter()
        .flat_map(|s| s.iter())
        .map(|w| pallas_crypto::hash::Hasher::<224>::hash(&w.vkey))
        .collect();
    let absent: usize = tx
        .transaction_witness_set
        .native_script
        .iter()
        .flat_map(|s| s.iter())
        .map(|s| missing(s, &keys))
        .sum();
    if absent > 0 {
        let detected = witnesses.map_or(0, |s| s.len());
        let mut error = ApiError::new(
            StatusCode::FORBIDDEN,
            "missing_witnesses_in_transaction",
            "The transaction is missing verification key witnesses required by its native scripts.",
        );
        error.info = Some(
            serde_json::json!({"detected_number_of_key_wits":detected,"expected_number_of_key_wits":detected + absent}),
        );
        return Err(error);
    }
    super::transactions::submit_transaction_id(State(state), Path(wallet_id), Json(req)).await
}

async fn patch_template(
    state: AppState,
    wallet_id: String,
    req: serde_json::Value,
    delegation: bool,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    state
        .wallet_service
        .patch_shared_cosigners(&id, delegation, &req, state.config.network.to_pallas())
        .await
        .map_err(ApiError::from)?;
    let response = get_shared_wallet(State(state.clone()), Path(wallet_id.clone())).await?;
    if response.0["state"]["status"] != "incomplete" {
        state.start_wallet_worker(id.clone()).await;
        return get_shared_wallet(State(state), Path(wallet_id)).await;
    }
    Ok(response)
}

pub async fn patch_payment_template(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    patch_template(state, wallet_id, req, false).await
}
pub async fn patch_delegation_template(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, ApiError> {
    patch_template(state, wallet_id, req, true).await
}

/// GET /v2/shared-wallets/:id/keys
pub async fn get_shared_keys(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(query): Query<super::keys::KeyFormatQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let xpub = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let encoded = if query.format.as_deref() == Some("extended") {
        encode_xpub(&xpub)
    } else {
        quetzal_address::codec::bech32_encode("acct_shared_vk", &xpub[..32])
            .map_err(|e| ApiError::internal(e.to_string()))?
    };
    Ok(Json(serde_json::Value::String(encoded)))
}

/// POST /v2/shared-wallets/:id/keys/:index
pub async fn get_shared_account_key(
    State(state): State<AppState>,
    Path((wallet_id, index)): Path<(String, String)>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let acct_ix = parse_account_index(&index)?;
    let passphrase = req
        .get("passphrase")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ApiError::bad_request("missing_passphrase", "passphrase is required"))?;
    let xpub = state
        .wallet_service
        .derive_shared_account_xpub_at(&id, passphrase.as_bytes(), acct_ix)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::Value::String(encode_xpub(&xpub))),
    ))
}

/// GET /v2/shared-wallets/:id/keys/:role/:index
pub async fn get_shared_derived_key(
    State(state): State<AppState>,
    Path((wallet_id, role_str, index_str)): Path<(String, String, String)>,
    Query(query): Query<super::keys::KeyFormatQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    let _ = state
        .wallet_service
        .get_shared_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let role: u32 = match role_str.as_str() {
        "utxo_external" => 0,
        "utxo_internal" => 1,
        "mutable_account" => 2,
        "multisig_script" => 3,
        _ => {
            return Err(ApiError::bad_request(
                "invalid_role",
                format!("Unknown role: {role_str}"),
            ))
        }
    };
    let ix: u32 = index_str
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_index", "index must be a number"))?;
    let acct_xpub_bytes = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let acct_xpub = Bip32PublicKey::from_raw_bytes(&acct_xpub_bytes)
        .map_err(|e| ApiError::internal(format!("xpub decode: {e}")))?;
    let role_key = acct_xpub
        .derive(role)
        .map_err(|e| ApiError::internal(format!("derive: {e}")))?;
    let addr_key = role_key
        .derive(ix)
        .map_err(|e| ApiError::internal(format!("derive: {e}")))?;
    let xpub_bytes: [u8; 64] = addr_key.to_raw_bytes();
    let prefix = match (role == 2, query.hash == Some(true)) {
        (true, true) => "stake_shared_vkh",
        (true, false) => "stake_shared_vk",
        (false, true) => "addr_shared_vkh",
        (false, false) => "addr_shared_vk",
    };
    let hash = pallas_crypto::hash::Hasher::<224>::hash(&xpub_bytes[..32]);
    let bytes = if query.hash == Some(true) {
        hash.as_ref()
    } else {
        &xpub_bytes[..32]
    };
    let encoded = quetzal_address::codec::bech32_encode(prefix, bytes)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Json(serde_json::Value::String(encoded)))
}

/// GET /v2/shared-wallets/:id/payment-script-template
pub async fn get_payment_script_template(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    match state
        .wallet_service
        .get_script_template(&id, b"payment_script")
        .await
        .map_err(ApiError::from)?
    {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found("No payment script template found")),
    }
}

/// GET /v2/shared-wallets/:id/delegation-script-template
pub async fn get_delegation_script_template(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = parse_shared_wid(&wallet_id)?;
    match state
        .wallet_service
        .get_script_template(&id, b"delegation_script")
        .await
        .map_err(ApiError::from)?
    {
        Some(v) => Ok(Json(v)),
        None => Err(ApiError::not_found("No delegation script template found")),
    }
}
