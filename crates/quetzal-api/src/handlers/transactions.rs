use crate::{
    error::ApiError,
    json::transaction::{
        AmountResponse, InputResponse, OutputResponse, PostTransactionRequest,
        PostTransactionSubmitRequest, TransactionResponse,
    },
    state::AppState,
};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use quetzal_primitives::Coin;
use quetzal_primitives::{TxHash, WalletId};
use serde::Deserialize;

async fn require_recent_node(
    mut node: quetzal_network::NodeClient,
) -> Result<quetzal_network::NodeClient, ApiError> {
    let era = quetzal_network::StateQuerier::new(node.statequery())
        .current_era()
        .await;
    let result = era
        .map_err(|e| ApiError::internal(e.to_string()))
        .and_then(ApiError::require_recent_era);
    match result {
        Ok(()) => Ok(node),
        Err(error) => {
            node.abort().await;
            Err(error)
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct TxListQuery {
    pub start: Option<String>,
    pub end: Option<String>,
    pub order: Option<String>,
    pub max_count: Option<usize>,
    pub address: Option<String>,
    #[serde(rename = "minWithdrawal")]
    pub min_withdrawal: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ConstructRequest {
    #[serde(skip)]
    pub change_address: Option<String>,
    pub vote: Option<String>,
    pub delegations: Option<Vec<DelegationAction>>,
    pub payments: Option<Vec<PaymentTargetRaw>>,
    pub withdrawal: Option<serde_json::Value>,
    pub metadata: Option<serde_json::Value>,
    pub encrypt_metadata: Option<MetadataPassword>,
    pub validity_interval: Option<serde_json::Value>,
    pub mint_burn: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegationAction {
    Join {
        pool: String,
        stake_key_index: String,
    },
    Quit {
        stake_key_index: String,
    },
}

fn parse_pool_id(text: &str) -> Result<[u8; 28], ApiError> {
    let bytes = if let Ok(bytes) = hex::decode(text) {
        bytes
    } else {
        let (hrp, bytes) = quetzal_address::codec::bech32_decode(text)
            .map_err(|_| ApiError::bad_request("bad_request", "Invalid pool ID"))?;
        if hrp != "pool" {
            return Err(ApiError::bad_request("bad_request", "Invalid pool ID"));
        }
        bytes
    };
    bytes
        .try_into()
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid pool ID"))
}

#[derive(Debug, Deserialize)]
pub struct PaymentTargetRaw {
    pub address: String,
    pub amount: AmountRaw,
    pub assets: Option<Vec<quetzal_primitives::tx::AssetAmount>>,
}

#[derive(Debug, Deserialize)]
pub struct AmountRaw {
    #[serde(deserialize_with = "quetzal_primitives::quantity::deserialize_coin")]
    pub quantity: u64,
    pub unit: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SignRequest {
    pub passphrase: String,
    pub transaction: String, // hex-encoded CBOR
}

#[derive(Debug, Deserialize)]
pub struct DecodeRequest {
    pub transaction: String, // hex-encoded CBOR
    pub decrypt_metadata: Option<MetadataPassword>,
}

#[derive(Debug, Deserialize)]
pub struct MetadataPassword {
    pub passphrase: String,
    pub method: String,
}

pub async fn list_transactions(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(q): Query<TxListQuery>,
) -> Result<Json<Vec<TransactionResponse>>, ApiError> {
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let limit = q.max_count.unwrap_or(usize::MAX);
    if q.order
        .as_deref()
        .is_some_and(|order| !["ascending", "descending"].contains(&order))
    {
        return Err(ApiError::bad_request(
            "bad_request",
            "order must be ascending or descending",
        ));
    }
    let parse_time = |value: &String| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map_err(|e| ApiError::bad_request("bad_request", e.to_string()))
    };
    let start = q.start.as_ref().map(parse_time).transpose()?;
    let end = q.end.as_ref().map(parse_time).transpose()?;
    if start.zip(end).is_some_and(|(start, end)| start > end) {
        return Err(ApiError::bad_request(
            "start_time_later_than_end_time",
            "The start time is later than the end time.",
        ));
    }
    let mut txs = state
        .wallet_service
        .get_transactions(&id, None, usize::MAX)
        .await
        .map_err(ApiError::from)?;
    txs.retain(|tx| {
        let time =
            chrono::DateTime::parse_from_rfc3339(&state.config.slot_reference(tx.slot).2).ok();
        let in_range = time.is_some_and(|time| {
            start.is_none_or(|start| time >= start) && end.is_none_or(|end| time <= end)
        });
        let address = q.address.as_ref().is_none_or(|address| {
            tx.inputs.iter().any(|input| {
                input
                    .address
                    .as_ref()
                    .is_some_and(|owned| owned.as_str() == address)
            }) || tx
                .outputs
                .iter()
                .any(|output| output.address.as_str() == address)
        });
        let withdrawal = q.min_withdrawal.is_none_or(|minimum| {
            tx.withdrawals
                .iter()
                .any(|withdrawal| withdrawal.amount.quantity.0 >= minimum)
        });
        in_range && address && withdrawal
    });
    if q.order.as_deref() == Some("ascending") {
        txs.reverse();
    }
    txs.truncate(limit);
    let height = state.chain_tip.read().await.1;
    Ok(Json(
        txs.iter()
            .map(|tx| TransactionResponse::from_record(tx, &state.config, height))
            .collect::<Result<_, _>>()?,
    ))
}

pub async fn get_transaction(
    State(state): State<AppState>,
    Path((wallet_id, tx_id)): Path<(String, String)>,
) -> Result<Json<TransactionResponse>, ApiError> {
    let wid = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let txid = TxHash::from_hex(&tx_id)
        .map_err(|_| ApiError::not_found(format!("No transaction with id: {tx_id}")))?;
    let _submission = state.submission_lock.lock().await;
    let tx = state
        .wallet_service
        .get_transaction(&wid, &txid)
        .await
        .map_err(ApiError::from)?;
    let height = state.chain_tip.read().await.1;
    Ok(Json(TransactionResponse::from_record(
        &tx,
        &state.config,
        height,
    )?))
}

pub async fn create_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<PostTransactionRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
    let id = WalletId::from_hex(&wallet_id)
        .map_err(|e| ApiError::bad_request("bad_request", e.to_string()))?;
    let change_address = if state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?
        .wallet_type
        == quetzal_primitives::WalletType::Byron
        && !state
            .wallet_service
            .available_utxos(&id)
            .await
            .map_err(ApiError::from)?
            .is_empty()
    {
        let entry = state
            .wallet_service
            .create_random_address(
                &id,
                req.passphrase.as_bytes(),
                None,
                (state.config.network != crate::config::Network::Mainnet).then_some(
                    state
                        .config
                        .network_magic
                        .try_into()
                        .map_err(|_| ApiError::internal("network magic overflow"))?,
                ),
            )
            .await
            .map_err(ApiError::from)?;
        Some(entry.address.0)
    } else {
        None
    };
    let metadata = req.metadata.clone();
    let payments = req
        .payments
        .into_iter()
        .map(|payment| PaymentTargetRaw {
            address: payment.address,
            amount: AmountRaw {
                quantity: payment.amount.quantity,
                unit: Some(payment.amount.unit),
            },
            assets: payment.assets,
        })
        .collect();
    let (_, Json(constructed)) = construct_transaction(
        State(state.clone()),
        Path(wallet_id.clone()),
        Json(ConstructRequest {
            change_address,
            vote: None,
            delegations: None,
            payments: Some(payments),
            withdrawal: req.withdrawal.map(serde_json::Value::String),
            metadata: metadata.clone(),
            encrypt_metadata: None,
            validity_interval: req
                .time_to_live
                .map(|ttl| serde_json::json!({"invalid_hereafter":ttl})),
            mint_burn: None,
        }),
    )
    .await?;
    let unsigned = constructed["transaction"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Missing constructed transaction"))?
        .to_owned();
    let (_, Json(signed)) = sign_transaction(
        State(state.clone()),
        Path(wallet_id.clone()),
        Json(SignRequest {
            passphrase: req.passphrase,
            transaction: unsigned,
        }),
    )
    .await?;
    let signed = signed["transaction"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Missing signed transaction"))?
        .to_owned();
    let (status, Json(mut response)) = submit_transaction(
        State(state),
        Path(wallet_id),
        Json(PostTransactionSubmitRequest {
            transaction: signed,
        }),
    )
    .await?;
    let selection = &constructed["coin_selection"];
    let amount = |value: &serde_json::Value| {
        value["amount"]["quantity"]
            .as_u64()
            .ok_or_else(|| ApiError::internal("Missing selection amount"))
    };
    let mut total = 0u64;
    for output in selection["outputs"]
        .as_array()
        .ok_or_else(|| ApiError::internal("Missing selected outputs"))?
    {
        total = total
            .checked_add(amount(output)?)
            .ok_or_else(|| ApiError::internal("Payment amount overflow"))?;
    }
    let fee = constructed["fee"]["quantity"]
        .as_u64()
        .ok_or_else(|| ApiError::internal("Missing fee"))?;
    response.amount = AmountResponse::lovelace(Coin(
        total
            .checked_add(fee)
            .ok_or_else(|| ApiError::internal("Payment amount overflow"))?,
    ));
    response.fee = AmountResponse::lovelace(Coin(fee));
    response.metadata = metadata;
    response.inputs = selection["inputs"]
        .as_array()
        .ok_or_else(|| ApiError::internal("Missing selected inputs"))?
        .iter()
        .map(|input| {
            Ok(InputResponse {
                id: input["id"]
                    .as_str()
                    .ok_or_else(|| ApiError::internal("Missing input ID"))?
                    .into(),
                index: u32::try_from(
                    input["index"]
                        .as_u64()
                        .ok_or_else(|| ApiError::internal("Missing input index"))?,
                )
                .map_err(|_| ApiError::internal("Input index overflow"))?,
                address: input["address"].as_str().map(str::to_owned),
                amount: Some(AmountResponse::lovelace(Coin(amount(input)?))),
                assets: input["assets"].as_array().cloned().unwrap_or_default(),
            })
        })
        .collect::<Result<_, ApiError>>()?;
    response.outputs = selection["outputs"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(selection["change"].as_array().into_iter().flatten())
        .map(|output| {
            Ok(OutputResponse {
                address: output["address"]
                    .as_str()
                    .ok_or_else(|| ApiError::internal("Missing output address"))?
                    .into(),
                amount: AmountResponse::lovelace(Coin(amount(output)?)),
                assets: output["assets"].as_array().cloned().unwrap_or_default(),
                derivation_path: None,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    Ok((status, Json(response)))
}

pub async fn delete_pending_transaction(
    State(state): State<AppState>,
    Path((wallet_id, tx_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let wid = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let txid = TxHash::from_hex(&tx_id)
        .map_err(|_| ApiError::not_found(format!("No transaction with id: {tx_id}")))?;
    state
        .wallet_service
        .delete_pending(&wid, &txid)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn submit_transaction_id(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<PostTransactionSubmitRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    let (status, Json(transaction)) =
        submit_transaction(State(state), Path(wallet_id), Json(req)).await?;
    Ok((status, Json(serde_json::json!({"id":transaction.id}))))
}

pub async fn submit_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<PostTransactionSubmitRequest>,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
    let wid = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;

    let cbor = hex::decode(&req.transaction)
        .or_else(|_| base64_decode(&req.transaction))
        .map_err(|e| ApiError::bad_request("invalid_transaction", format!("Decode failed: {e}")))?;

    use pallas_primitives::{conway, Fragment};
    state
        .wallet_service
        .get_wallet(&wid)
        .await
        .map_err(ApiError::from)?;
    let tx = conway::Tx::decode_fragment(&cbor)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?;
    if tx
        .transaction_witness_set
        .vkeywitness
        .as_ref()
        .is_none_or(|witnesses| witnesses.is_empty())
        && tx
            .transaction_witness_set
            .bootstrap_witness
            .as_ref()
            .is_none_or(|witnesses| witnesses.is_empty())
    {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "missing_witnesses_in_transaction",
            "The transaction has no verification key witnesses.",
        ));
    }
    let body_hash = quetzal_wallet::txsize::body_hash(&cbor)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e))?;
    // The node is authoritative for signature validity, including the exact
    // ledger rejection reported by this endpoint. Do not turn node failures
    // into a different local 403 response.
    let _submission = state.submission_lock.lock().await;
    let tx_id = TxHash(body_hash);
    if state
        .wallet_service
        .get_pending_txs(&wid)
        .await
        .map_err(ApiError::from)?
        .iter()
        .any(|pending| pending.id == tx_id)
    {
        let record = state
            .wallet_service
            .get_transaction(&wid, &tx_id)
            .await
            .map_err(ApiError::from)?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(TransactionResponse::from_record(
                &record,
                &state.config,
                state.chain_tip.read().await.1,
            )?),
        ));
    }
    let available = state
        .wallet_service
        .available_utxos(&wid)
        .await
        .map_err(ApiError::from)?;
    let owned = tx.transaction_body.inputs.iter().any(|input| {
        available.iter().any(|utxo| {
            utxo.id.tx_hash.0 == *input.transaction_id
                && u64::from(utxo.id.output_index) == input.index
        })
    });
    if !owned {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "foreign_transaction",
            "The transaction has no available inputs belonging to this wallet.",
        ));
    }
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| {
                ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "network_unreachable",
                    e.to_string(),
                )
            })?;
    node = require_recent_node(node).await?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    let parameters = match parameters {
        Ok(parameters) => parameters,
        Err(error) => {
            node.abort().await;
            return Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "network_unreachable",
                error.to_string(),
            ));
        }
    };
    if let Err(error) = ApiError::require_recent_era(parameters.era) {
        node.abort().await;
        return Err(error);
    }
    let saved = state
        .wallet_service
        .submit_pending(&wid, cbor.clone())
        .await;
    if let Err(error) = saved {
        node.abort().await;
        return Err(ApiError::from(error));
    }
    let result = quetzal_network::TxSubmitter::new(node.submission())
        .submit_era(parameters.era, cbor)
        .await;
    node.abort().await;
    if let Err(error) = result {
        if let quetzal_network::NetworkError::TxRejected(raw) = error {
            if !quetzal_network::txsubmit::retryable_rejection(&raw) {
                let _ = state.wallet_service.delete_pending(&wid, &tx_id).await;
            }
            return Err(super::settings::submission_error(
                quetzal_network::NetworkError::TxRejected(raw),
            ));
        }
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "network_unreachable",
            error.to_string(),
        ));
    }
    let record = state
        .wallet_service
        .get_transaction(&wid, &tx_id)
        .await
        .map_err(ApiError::from)?;
    let height = state.chain_tip.read().await.1;
    let response = TransactionResponse::from_record(&record, &state.config, height)?;

    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// POST /v2/wallets/:id/transactions-construct
///
/// Builds a real CBOR Cardano transaction body using coin selection.
/// Returns hex-encoded CBOR and structured fee/input/output info.
pub async fn construct_transaction(
    state: State<AppState>,
    wallet_id: Path<String>,
    request: Json<ConstructRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    construct_transaction_with_quit(state, wallet_id, request, false).await
}

pub(super) async fn construct_transaction_with_quit(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(mut req): Json<ConstructRequest>,
    mut quit: bool,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    use quetzal_wallet::payment::Payment;
    let actions = req.delegations.as_deref().unwrap_or_default();
    if actions.len() > 1 {
        return Err(ApiError::bad_request(
            "bad_request",
            "Only one delegation action is supported",
        ));
    }
    let mut join = None;
    for action in actions {
        let index = match action {
            DelegationAction::Join {
                pool,
                stake_key_index,
            } => {
                join = Some(parse_pool_id(pool)?);
                stake_key_index
            }
            DelegationAction::Quit { stake_key_index } => {
                quit = true;
                stake_key_index
            }
        };
        if index != "0H" {
            return Err(ApiError::bad_request(
                "bad_request",
                "Only stake key index 0H is supported",
            ));
        }
    }
    if quit && (join.is_some() || req.vote.is_some()) {
        return Err(ApiError::bad_request(
            "bad_request",
            "Retirement cannot be combined with delegation or voting",
        ));
    }
    if let Some(encryption) = req.encrypt_metadata.as_ref() {
        if encryption.method != "basic" {
            return Err(ApiError::bad_request(
                "bad_request",
                "Unsupported metadata encryption method",
            ));
        }
        use rand::RngCore;
        let mut salt = [0; 8];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        req.metadata = Some(
            quetzal_wallet::metadata_encryption::detailed(
                req.metadata.as_ref().ok_or_else(|| {
                    ApiError::bad_request("invalid_metadata_encryption", "Missing metadata")
                })?,
                encryption.passphrase.as_bytes(),
                Some(salt),
            )
            .map_err(|error| {
                ApiError::new(StatusCode::FORBIDDEN, "invalid_metadata_encryption", error)
            })?,
        );
    }
    for (index, payment) in req
        .payments
        .as_deref()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        if payment
            .amount
            .unit
            .as_deref()
            .is_some_and(|unit| unit != "lovelace")
        {
            return Err(ApiError::bad_request(
                "bad_request",
                "Payment amount unit must be lovelace",
            ));
        }
        let address = payment
            .address
            .parse::<pallas_addresses::Address>()
            .map_err(|_| {
                ApiError::bad_request(
                    "bad_request",
                    format!("Error in $.payments[{index}].address: Invalid address"),
                )
            })?;
        let network = if state.config.network == crate::config::Network::Mainnet {
            1
        } else {
            0
        };
        if address
            .network()
            .is_some_and(|value| value.value() != network)
        {
            let expected = if network == 1 { "Mainnet" } else { "Testnet" };
            let actual = if network == 1 { "Testnet" } else { "Mainnet" };
            return Err(ApiError::bad_request("bad_request",format!("Error in $.payments[{index}].address: parsing AddressAmount failed, (String '{}',TextDecodingError {{getTextDecodingError = 'Invalid network discrimination on address. Expecting {expected} but got {actual}.'}})",payment.address)));
        }
    }
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
    let mut payments: Vec<Payment> = req
        .payments
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let value = quetzal_wallet::ledger::from_assets(
                Coin(p.amount.quantity),
                p.assets.as_deref().unwrap_or_default(),
            )
            .map_err(|e| ApiError::bad_request("bad_request", e.to_string()))?;
            Ok(Payment {
                address: p.address,
                amount: value.coin,
                assets: value.assets,
            })
        })
        .collect::<Result<_, ApiError>>()?;
    let withdraw_self = req
        .withdrawal
        .as_ref()
        .is_some_and(|value| value.as_str() == Some("self"));
    if req.withdrawal.is_some() && !withdraw_self {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "External reward-account withdrawal is not implemented.",
        ));
    }
    let mut mint = quetzal_wallet::mint::Minting::default();
    let actions: Vec<quetzal_wallet::mint::Action> = req
        .mint_burn
        .as_ref()
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|e| ApiError::bad_request("bad_request", e.to_string()))?
        .unwrap_or_default();
    if !actions.is_empty() {
        let public = state
            .wallet_service
            .policy_xpub(&id)
            .await
            .map_err(ApiError::from)?;
        let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&public)
            .map_err(|e| ApiError::internal(e.to_string()))?;
        let key = *pallas_crypto::hash::Hasher::<224>::hash(&public.to_vk_bytes());
        for action in actions {
            let script = quetzal_wallet::policy::script(&action.policy_script_template, &key)
                .map_err(ApiError::from)?;
            let policy = script
                .hash()
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let script_bytes = script
                .serialize()
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let script = pallas_codec::minicbor::decode(&script_bytes[1..])
                .map_err(|e| ApiError::internal(e.to_string()))?;
            let name = hex::decode(action.asset_name)
                .map_err(|_| ApiError::bad_request("bad_request", "Invalid asset name"))?;
            let (quantity, receiving_address) = match action.operation {
                quetzal_wallet::mint::Operation::Mint {
                    quantity,
                    receiving_address,
                } => (
                    i64::try_from(quantity).ok().filter(|n| *n > 0),
                    receiving_address,
                ),
                quetzal_wallet::mint::Operation::Burn { quantity } => (
                    i64::try_from(quantity).ok().filter(|n| *n > 0).map(|n| -n),
                    None,
                ),
            };
            let quantity = quantity.ok_or_else(|| {
                ApiError::bad_request(
                    "bad_request",
                    "Asset quantity must be a positive signed 64-bit integer",
                )
            })?;
            mint.add(policy, &name, quantity, script)
                .map_err(ApiError::from)?;
            if let Some(address) = receiving_address {
                let parsed = pallas_addresses::Address::from_bech32(&address).map_err(|_| {
                    ApiError::bad_request("bad_request", "Invalid receiving address")
                })?;
                if parsed.to_vec()[0] & 15 != state.config.network.to_pallas().value() {
                    return Err(ApiError::bad_request(
                        "bad_request",
                        "Invalid network discrimination on receiving address",
                    ));
                }
                let assets = std::collections::BTreeMap::from([(
                    hex::encode(policy),
                    std::collections::BTreeMap::from([(hex::encode(name), quantity as u64)]),
                )]);
                payments.push(Payment {
                    address,
                    amount: Coin::ZERO,
                    assets,
                });
            }
        }
    }
    let mint = (!mint.quantities.is_empty()).then_some(&mint);
    if payments.is_empty()
        && req.metadata.is_none()
        && mint.is_none()
        && req.vote.is_none()
        && join.is_none()
        && !withdraw_self
        && !quit
    {
        return Err(ApiError::bad_request(
            "no_payments",
            "At least one payment target is required",
        ));
    }
    let no_utxos = state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?
        .is_empty();
    if no_utxos && !quit {
        return Err(ApiError::new(StatusCode::FORBIDDEN,"no_utxos_available","Unable to create a transaction because the wallet has no unspent transaction outputs (UTxOs) available. A transaction must spend at least one UTxO in order to be accepted for inclusion on the Cardano blockchain."));
    }
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    node = require_recent_node(node).await?;
    let staking = if req.vote.is_some() || quit || join.is_some() || withdraw_self {
        let result = async {
            let public = state.wallet_service.get_account_xpub_bytes(&id).await.map_err(ApiError::from)?;
            let hash = quetzal_wallet::staking::stake_hash(&public).map_err(ApiError::from)?;
            let mut drep = req.vote.as_ref().map(|vote| quetzal_wallet::staking::drep(vote)).transpose().map_err(ApiError::from)?;
            let confirmed = state.wallet_service.staking_state(&id).await.map_err(ApiError::from)?;
            let requested_vote = drep.as_ref().map(quetzal_wallet::staking::drep_text).transpose().map_err(ApiError::from)?;
            if let Some(vote) = requested_vote.as_ref().filter(|vote| join.is_none() && confirmed.vote.as_ref() == Some(vote)) {
                return Err(ApiError::new(StatusCode::FORBIDDEN, "same_vote", format!("I couldn't cast a vote :  {vote}. I have already voted like that; repeating this action would incur an unnecessary fee!")));
            }
            let accounts = quetzal_network::StateQuerier::new(node.statequery()).stake_accounts(&[(0, hash)]).await.map_err(|e| ApiError::internal(e.to_string()))?;
            let account = accounts.first().ok_or_else(|| ApiError::internal("Missing stake account query result"))?;
            // Conway's legacy join endpoint defaults a wallet that has not
            // voted yet to abstention, preserving an existing vote otherwise.
            if join.is_some() && drep.is_none() && (!account.registered || confirmed.vote.is_none()) {drep = Some(pallas_primitives::conway::DRep::Abstain);}
            let requested_vote = drep.as_ref().map(quetzal_wallet::staking::drep_text).transpose().map_err(ApiError::from)?;
            let pool = if let Some(raw) = join {
                let known = state.wallet_service.registered_pools().map_err(ApiError::from)?;
                let snapshot = quetzal_network::StateQuerier::new(node.statequery()).pools(0).await.map_err(|e|ApiError::internal(e.to_string()))?;
                if !known.contains(raw.as_slice()) || !snapshot.pools.contains_key(raw.as_slice()) {
                    let mut error = ApiError::new(StatusCode::NOT_FOUND, "no_such_pool", format!("I couldn't find any stake pool with the given id: {}", hex::encode(raw)));
                    error.info = Some(serde_json::json!({"pool_id":quetzal_primitives::PoolId(raw).to_bech32()}));
                    return Err(error);
                }
                if confirmed.pool.as_deref() == Some(quetzal_primitives::PoolId(raw).to_bech32().as_str()) && requested_vote.as_ref().is_none_or(|vote| confirmed.vote.as_ref() == Some(vote)) {
                    let voting = drep.is_some();
                    return Err(ApiError::new(StatusCode::FORBIDDEN, if voting {"pool_already_joined_same_vote"} else {"pool_already_joined"}, if voting {
                        format!("I couldn't join a stake pool with the given id: {} and vote. I have already joined this pool, also voted the same last time; joining/voting again would incur an unnecessary fee!", hex::encode(raw))
                    } else {format!("I couldn't join a stake pool with the given id: {}. I have already joined this pool; joining again would incur an unnecessary fee!", hex::encode(raw))}));
                }
                Some(raw)
            } else {None};
            if quit && !account.registered { return Err(ApiError::from(quetzal_wallet::WalletError::NotDelegating)); }
            let refund = if quit { account.deposit.ok_or_else(|| ApiError::internal("Missing ledger stake registration deposit"))? } else {0};
            Ok((hash, drep, pool, account.registered, refund, account.reward_balance))
        }.await;
        match result {
            Ok(result) => Some(result),
            Err(error) => {
                node.abort().await;
                return Err(error);
            }
        }
    } else {
        None
    };
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    node.abort().await;
    let parameters = parameters.map_err(|e| ApiError::internal(e.to_string()))?;
    ApiError::require_recent_era(parameters.era)?;
    if no_utxos {
        return Err(ApiError::from(quetzal_wallet::WalletError::NoUtxos));
    }
    let staking = staking
        .map(
            |(hash, drep, pool, registered, refund, reward)| -> Result<_, ApiError> {
                if withdraw_self {
                    quetzal_wallet::staking::check_withdrawal_reward(reward, parameters.min_fee_a)
                        .map_err(ApiError::from)?;
                }
                let action = match (pool, drep.clone()) {
                    (Some(pool), _) if registered => {
                        quetzal_wallet::writer::StakeAction::Join(pool.into())
                    }
                    (Some(pool), _) => {
                        quetzal_wallet::writer::StakeAction::JoinRegistering(pool.into())
                    }
                    (None, Some(drep)) if registered => {
                        quetzal_wallet::writer::StakeAction::Vote(drep)
                    }
                    (None, Some(drep)) => {
                        quetzal_wallet::writer::StakeAction::VoteRegistering(drep)
                    }
                    (None, None) => quetzal_wallet::writer::StakeAction::Quit,
                };
                let credential = pallas_primitives::StakeCredential::AddrKeyhash(hash.into());
                let mut certificates = if quit || pool.is_some() || drep.is_some() {
                    quetzal_wallet::writer::certificates(
                        credential.clone(),
                        Some(if quit { refund } else { parameters.key_deposit }),
                        action,
                    )
                    .map_err(ApiError::internal)?
                } else {
                    vec![]
                };
                if pool.is_some() {
                    if let Some(drep) = drep {
                        certificates.extend(
                            quetzal_wallet::writer::certificates(
                                credential,
                                None,
                                quetzal_wallet::writer::StakeAction::Vote(drep),
                            )
                            .map_err(ApiError::internal)?,
                        );
                    }
                }
                Ok(quetzal_wallet::staking::Operation {
                    certificates,
                    deposit_taken: if registered || (pool.is_none() && req.vote.is_none()) {
                        0
                    } else {
                        parameters.key_deposit
                    },
                    deposit_returned: refund,
                    withdrawals: if (quit || withdraw_self) && reward > 0 {
                        let mut account = vec![0xe0 | state.config.network.to_pallas().value()];
                        account.extend(hash);
                        std::collections::BTreeMap::from([(account, reward)])
                    } else {
                        Default::default()
                    },
                })
            },
        )
        .transpose()?;
    let now = std::time::SystemTime::now();
    let invalid_bounds = || {
        ApiError::new(StatusCode::FORBIDDEN, "invalid_validity_bounds",
        "Attempted to create a transaction with invalid validity bounds. Please make sure that the 'invalid_before' bound precedes the 'invalid_hereafter' bound, and that you have not used negative time values.")
    };
    let clock_slot = |seconds, ceiling| {
        state
            .config
            .time_to_slot(now, seconds, ceiling)
            .ok_or_else(|| ApiError::internal("Validity clock overflow"))
    };
    let bound = |name: &str, default: u64, ceiling: bool| -> Result<u64, ApiError> {
        let Some(value) = req
            .validity_interval
            .as_ref()
            .and_then(|interval| interval.get(name))
        else {
            return Ok(default);
        };
        let quantity = value
            .get("quantity")
            .and_then(|value| value.as_number())
            .and_then(|value| quetzal_primitives::quantity::parse_u64(&value.to_string()))
            .ok_or_else(invalid_bounds)?;
        match value.get("unit").and_then(|unit| unit.as_str()) {
            Some("slot") => Ok(quantity),
            Some("second") => clock_slot(quantity, ceiling),
            _ => Err(ApiError::bad_request(
                "bad_request",
                "Validity bound unit must be slot or second",
            )),
        }
    };
    let start = bound("invalid_before", 0, true)?;
    let expiry = bound("invalid_hereafter", clock_slot(7200, true)?, true)?;
    if start > expiry {
        return Err(invalid_bounds());
    }
    let network = if state.config.network == crate::config::Network::Mainnet {
        1
    } else {
        0
    };
    let built = if payments.is_empty() && mint.is_none() && staking.is_none() {
        let inputs = state
            .wallet_service
            .available_utxos(&id)
            .await
            .map_err(ApiError::from)?;
        let mut addresses = state
            .wallet_service
            .get_addresses(&id)
            .await
            .map_err(ApiError::from)?;
        addresses.sort_by_key(|entry| entry.path.address_ix);
        let change = addresses
            .into_iter()
            .find(|entry| entry.path.role == quetzal_primitives::AddressRole::Internal)
            .ok_or_else(|| ApiError::internal("Missing change address"))?;
        let mut built = state
            .wallet_service
            .construct_sweep(
                &inputs,
                change.address.as_str(),
                &parameters,
                expiry,
                network,
                req.metadata.as_ref(),
            )
            .map_err(ApiError::from)?;
        let tx = pallas_traverse::MultiEraTx::decode(&built.cbor)
            .map_err(|error| ApiError::internal(error.to_string()))?;
        let value = quetzal_wallet::ledger::value(&tx.outputs()[0].value());
        built.change = Some((change, value));
        built
    } else {
        state
            .wallet_service
            .construct_payment_with_change(
                &id,
                &payments,
                &parameters,
                quetzal_wallet::payment::TransactionOptions {
                    start,
                    expiry,
                    network,
                    metadata: req.metadata.as_ref(),
                    mint,
                    staking: staking.as_ref(),
                },
                req.change_address.as_deref(),
            )
            .await
            .map_err(ApiError::from)?
    };
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
    let path = |entry: &quetzal_primitives::AddressEntry| {
        crate::json::transaction::derivation_path(&kind, &entry.path)
    };
    let inputs: Vec<_> = built.inputs.iter().map(|input| {
        let derivation = addresses.iter().find(|entry| entry.address == input.address).map(&path);
        serde_json::json!({"id":input.id.tx_hash.to_hex(),"index":input.id.output_index,"address":input.address.as_str(),"amount":{"quantity":input.value.coin.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&input.value),"derivation_path":derivation})
    }).collect();
    let outputs: Vec<_> = built.outputs.iter().map(|payment| serde_json::json!({"address":payment.address,"amount":{"quantity":payment.amount.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&quetzal_primitives::Value{coin:payment.amount,assets:payment.assets.clone()})})).collect();
    let change: Vec<_> = built.change.iter().map(|(entry,value)| serde_json::json!({"address":entry.address.as_str(),"amount":{"quantity":value.coin.0,"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(value),"derivation_path":path(entry)})).collect();
    let serialized_metadata = req
        .metadata
        .as_ref()
        .map(|metadata| {
            use base64::Engine;
            quetzal_wallet::metadata::encode(metadata)
                .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes))
        })
        .transpose()
        .map_err(ApiError::from)?;
    let multi = pallas_traverse::MultiEraTx::decode(&built.cbor)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let mut withdrawals = crate::json::transaction::withdrawal_responses(
        &quetzal_wallet::ledger::withdrawals(&multi).map_err(ApiError::from)?,
    );
    if !withdrawals.is_empty() {
        let metadata = state
            .wallet_service
            .get_wallet(&id)
            .await
            .map_err(ApiError::from)?;
        for withdrawal in &mut withdrawals {
            withdrawal["derivation_path"] = serde_json::json!([
                "1852H",
                "1815H",
                format!("{}H", metadata.account_index),
                "2",
                "0"
            ]);
        }
    }
    let mut response = serde_json::json!({
        "transaction":hex::encode(built.cbor),
        "fee":{"quantity":built.fee.0,"unit":"lovelace"},
        "coin_selection":{"inputs":inputs,"outputs":outputs,"change":change,"collateral":[],"withdrawals":withdrawals,"deposits_taken":[],"deposits_returned":[],"metadata":serialized_metadata},
        "script_cost":{"quantity":0,"unit":"lovelace"}
    });
    if let Some(staking) = &staking {
        let metadata = state
            .wallet_service
            .get_wallet(&id)
            .await
            .map_err(ApiError::from)?;
        let path = quetzal_primitives::DerivationPath {
            account_ix: metadata.account_index,
            role: quetzal_primitives::AddressRole::Stake,
            address_ix: 0,
        };
        response["coin_selection"]["certificates"] =
            serde_json::json!(quetzal_wallet::staking::api_certificate_list(
                &staking.encoded().map_err(ApiError::from)?,
                &path
            )
            .map_err(ApiError::from)?);
        for (name, quantity) in [
            ("deposits_taken", staking.deposit_taken),
            ("deposits_returned", staking.deposit_returned),
        ] {
            if quantity != 0 {
                response["coin_selection"][name] =
                    serde_json::json!([{"quantity":quantity,"unit":"lovelace"}]);
            }
        }
    }
    if serialized_metadata.is_none() {
        response["coin_selection"]
            .as_object_mut()
            .expect("selection object")
            .remove("metadata");
    }
    Ok((StatusCode::ACCEPTED, Json(response)))
}

/// POST /v2/wallets/:id/transactions-sign
///
/// Signs an unsigned transaction CBOR using the wallet's private keys.
/// The passphrase is used to decrypt the root key.
pub async fn sign_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<SignRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    use pallas_primitives::{conway, Fragment};

    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let meta = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;

    let cbor_bytes = hex::decode(&req.transaction)
        .or_else(|_| base64_decode(&req.transaction))
        .map_err(|_| {
            ApiError::bad_request(
                "invalid_transaction",
                "Transaction must be hex-encoded CBOR",
            )
        })?;

    // Decode the transaction to extract body hash
    let tx: conway::Tx = conway::Tx::decode_fragment(&cbor_bytes).map_err(|_| {
        ApiError::bad_request("invalid_transaction", "Failed to decode CBOR transaction")
    })?;

    let body_hash = quetzal_wallet::txsize::body_hash(&cbor_bytes)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e))?;
    if matches!(
        meta.wallet_type,
        quetzal_primitives::WalletType::Byron | quetzal_primitives::WalletType::Icarus
    ) {
        let signed = state
            .wallet_service
            .sign_bootstrap_transaction(&id, req.passphrase.as_bytes(), &cbor_bytes)
            .await
            .map_err(ApiError::from)?;
        return Ok((
            StatusCode::ACCEPTED,
            Json(
                serde_json::json!({"transaction":hex::encode(signed),"status":"pending","script_validity":"valid"}),
            ),
        ));
    }

    // Find which inputs belong to this wallet
    let utxos = state
        .wallet_service
        .get_utxos(&id)
        .await
        .map_err(ApiError::from)?;
    let addresses = state
        .wallet_service
        .get_addresses(&id)
        .await
        .map_err(ApiError::from)?;

    let mut signing_paths = Vec::new();
    for txin in tx.transaction_body.inputs.iter() {
        let hash_bytes: [u8; 32] = *txin.transaction_id;
        let txin_hash = TxHash(hash_bytes);
        if let Some(utxo) = utxos
            .iter()
            .find(|u| u.id.tx_hash == txin_hash && u.id.output_index == txin.index as u32)
        {
            if let Some(addr_entry) = addresses
                .iter()
                .find(|a| a.address.as_str() == utxo.address.as_str())
            {
                if !signing_paths.contains(&addr_entry.path) {
                    signing_paths.push(addr_entry.path.clone());
                }
            }
        }
    }

    let public = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let stake_hash = quetzal_wallet::staking::stake_hash(&public).map_err(ApiError::from)?;
    let multi = pallas_traverse::MultiEraTx::decode(&cbor_bytes)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?;
    let own_withdrawal = tx
        .transaction_body
        .withdrawals
        .as_ref()
        .is_some_and(|withdrawals| {
            withdrawals.iter().any(|(account, _)| {
                account.len() == 29 && account[0] >> 4 == 14 && account[1..] == stake_hash
            })
        });
    if own_withdrawal
        || !quetzal_wallet::staking::owned_certificates(&multi, &stake_hash)
            .map_err(ApiError::from)?
            .certificates
            .is_empty()
    {
        let meta = state
            .wallet_service
            .get_wallet(&id)
            .await
            .map_err(ApiError::from)?;
        signing_paths.push(quetzal_primitives::DerivationPath {
            account_ix: meta.account_index,
            role: quetzal_primitives::AddressRole::Stake,
            address_ix: 0,
        });
    }
    if signing_paths.is_empty() {
        return Err(ApiError::bad_request(
            "foreign_transaction",
            "No wallet inputs found in this transaction",
        ));
    }

    let mut sigs = state
        .wallet_service
        .sign_transaction_inputs(&id, req.passphrase.as_bytes(), &signing_paths, &body_hash)
        .await
        .map_err(ApiError::from)?;

    if let Some(signature) = state
        .wallet_service
        .sign_minting_policy(&id, req.passphrase.as_bytes(), &tx, &body_hash)
        .await
        .map_err(ApiError::from)?
    {
        sigs.push(signature);
    }

    // Merge signatures by key so repeated signing is idempotent.
    let mut tx_mut = tx;
    for (vk, sig) in &sigs {
        quetzal_wallet::writer::merge_vkey(
            &mut tx_mut.transaction_witness_set,
            conway::VKeyWitness {
                vkey: vk.to_vec().into(),
                signature: sig.to_vec().into(),
            },
        );
    }

    let signed_cbor =
        quetzal_wallet::txsize::replace_witnesses(&cbor_bytes, &tx_mut.transaction_witness_set)
            .map_err(|_| ApiError::internal("Failed to re-encode signed transaction"))?;

    let signed_hex = hex::encode(&signed_cbor);

    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "transaction": signed_hex,
            "status": "pending",
            "script_validity": "valid"
        })),
    ))
}

/// POST /v2/wallets/:id/transactions-decode
///
/// Decodes a CBOR-encoded transaction into a structured JSON representation.
pub async fn decode_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<DecodeRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    use pallas_primitives::{conway, Fragment};

    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let _ = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;

    let cbor_bytes = hex::decode(&req.transaction)
        .or_else(|_| base64_decode(&req.transaction))
        .map_err(|_| {
            ApiError::bad_request(
                "invalid_transaction",
                "Transaction must be hex-encoded CBOR",
            )
        })?;

    let tx: conway::Tx = conway::Tx::decode_fragment(&cbor_bytes).map_err(|_| {
        ApiError::bad_request("invalid_transaction", "Failed to decode CBOR transaction")
    })?;

    let tx_hash_hex = hex::encode(
        quetzal_wallet::txsize::body_hash(&cbor_bytes)
            .map_err(|e| ApiError::bad_request("invalid_transaction", e))?,
    );

    let inputs: Vec<serde_json::Value> = tx
        .transaction_body
        .inputs
        .iter()
        .map(|i| {
            let hash: [u8; 32] = *i.transaction_id;
            serde_json::json!({ "id": hex::encode(hash), "index": i.index })
        })
        .collect();

    let fee = tx.transaction_body.fee;
    let mut metadata = pallas_traverse::MultiEraTx::decode(&cbor_bytes)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?
        .metadata()
        .as_alonzo()
        .map(quetzal_wallet::metadata::decode_ledger)
        .transpose()
        .map_err(ApiError::from)?;
    if let Some(decryption) = req.decrypt_metadata {
        if decryption.method != "basic" {
            return Err(ApiError::bad_request(
                "bad_request",
                "Unsupported metadata encryption method",
            ));
        }
        metadata = Some(
            quetzal_wallet::metadata_encryption::detailed(
                metadata.as_ref().ok_or_else(|| {
                    ApiError::new(
                        StatusCode::FORBIDDEN,
                        "invalid_metadata_decryption",
                        "Missing metadata",
                    )
                })?,
                decryption.passphrase.as_bytes(),
                None,
            )
            .map_err(|error| {
                ApiError::new(StatusCode::FORBIDDEN, "invalid_metadata_decryption", match error {
                    "ErrMissingEncryptionMethod" => "It looks like the encrypted metadata has wrong structure. It is expected to have encryption method under 'enc' key - see CIP83.",
                    error => error,
                })
            })?,
        );
    }

    let wit_count = tx
        .transaction_witness_set
        .vkeywitness
        .as_ref()
        .map(|w| w.len())
        .unwrap_or(0);

    let outputs: Vec<_> = pallas_traverse::MultiEraTx::decode(&cbor_bytes).map_err(|e| ApiError::bad_request("invalid_transaction",e.to_string()))?.outputs().iter().map(|output| {
        serde_json::json!({"address":output.address().map(|address|address.to_string()).unwrap_or_default(),"amount":{"quantity":output.value().coin(),"unit":"lovelace"},"assets":quetzal_wallet::ledger::asset_amounts(&quetzal_wallet::ledger::value(&output.value()))})
    }).collect();
    let multi = pallas_traverse::MultiEraTx::decode(&cbor_bytes)
        .map_err(|e| ApiError::bad_request("invalid_transaction", e.to_string()))?;
    let withdrawals = crate::json::transaction::withdrawal_responses(
        &quetzal_wallet::ledger::withdrawals(&multi).map_err(ApiError::from)?,
    );
    let meta = state
        .wallet_service
        .get_wallet(&id)
        .await
        .map_err(ApiError::from)?;
    let public = state
        .wallet_service
        .get_account_xpub_bytes(&id)
        .await
        .map_err(ApiError::from)?;
    let staking = quetzal_wallet::staking::owned_certificates(
        &multi,
        &quetzal_wallet::staking::stake_hash(&public).map_err(ApiError::from)?,
    )
    .map_err(ApiError::from)?;
    let path = quetzal_primitives::DerivationPath {
        account_ix: meta.account_index,
        role: quetzal_primitives::AddressRole::Stake,
        address_ix: 0,
    };
    let certificates = quetzal_wallet::staking::api_certificate_list(
        &staking.encoded().map_err(ApiError::from)?,
        &path,
    )
    .map_err(ApiError::from)?;
    let deposit = |quantity: u64| {
        if quantity == 0 {
            serde_json::json!([])
        } else {
            serde_json::json!([{"quantity":quantity,"unit":"lovelace"}])
        }
    };
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({
            "id": tx_hash_hex,
            "fee": { "quantity": fee, "unit": "lovelace" },
            "inputs": inputs,
            "outputs": outputs,
            "script_validity": if tx.success { "valid" } else { "invalid" },
            "withdrawals": withdrawals,
            "mint": {"tokens":[]},
            "burn": {"tokens":[]},
            "certificates": certificates,
            "deposits_taken": deposit(staking.deposit_taken),
            "deposits_returned": deposit(staking.deposit_returned),
            "metadata": metadata,
            "validity_interval": {"invalid_before":{"quantity":tx.transaction_body.validity_interval_start.unwrap_or(0),"unit":"slot"},"invalid_hereafter":{"quantity":tx.transaction_body.ttl.unwrap_or(u64::MAX),"unit":"slot"}},
            "witness_count": { "verification_key": wit_count, "bootstrap": tx.transaction_witness_set.bootstrap_witness.as_ref().map_or(0,|w|w.len()), "scripts": [] }
        })),
    ))
}

/// POST /v2/wallets/:id/transactions-balance
pub(super) async fn construct_sign_submit(
    state: AppState,
    wallet_id: String,
    request: ConstructRequest,
    passphrase: String,
    quit: bool,
) -> Result<(StatusCode, Json<TransactionResponse>), ApiError> {
    let (_, Json(constructed)) = construct_transaction_with_quit(
        State(state.clone()),
        Path(wallet_id.clone()),
        Json(request),
        quit,
    )
    .await?;
    let transaction = constructed["transaction"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Missing constructed transaction"))?
        .to_owned();
    let (_, Json(signed)) = sign_transaction(
        State(state.clone()),
        Path(wallet_id.clone()),
        Json(SignRequest {
            transaction,
            passphrase,
        }),
    )
    .await?;
    let transaction = signed["transaction"]
        .as_str()
        .ok_or_else(|| ApiError::internal("Missing signed transaction"))?
        .to_owned();
    submit_transaction(
        State(state),
        Path(wallet_id),
        Json(PostTransactionSubmitRequest { transaction }),
    )
    .await
}

pub async fn balance_transaction(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Json(req): Json<serde_json::Value>,
) -> Result<(StatusCode, Json<serde_json::Value>), ApiError> {
    use pallas_primitives::{conway, Fragment};
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
    let encoded = req["transaction"]
        .as_str()
        .ok_or_else(|| ApiError::bad_request("bad_request", "Missing transaction"))?;
    let raw = hex::decode(encoded)
        .map_err(|error| error.to_string())
        .or_else(|_| base64_decode(encoded))
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid transaction encoding"))?;
    let transaction = conway::Tx::decode_fragment(&raw)
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid CBOR transaction"))?;
    quetzal_wallet::balancing::validate(&transaction).map_err(ApiError::from)?;
    if ["inputs", "redeemers"].iter().any(|field| {
        req.get(field)
            .is_some_and(|value| !value.as_array().is_some_and(Vec::is_empty))
    }) {
        return Err(ApiError::new(
            StatusCode::NOT_IMPLEMENTED,
            "not_implemented",
            "Balancing external inputs and script redeemers is not implemented.",
        ));
    }
    if state
        .wallet_service
        .available_utxos(&id)
        .await
        .map_err(ApiError::from)?
        .is_empty()
    {
        return Err(ApiError::new(StatusCode::FORBIDDEN, "no_utxos_available", "Unable to create a transaction because the wallet has no unspent transaction outputs (UTxOs) available. A transaction must spend at least one UTxO in order to be accepted for inclusion on the Cardano blockchain."));
    }
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|error| ApiError::internal(error.to_string()))?;
    node = require_recent_node(node).await?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    node.abort().await;
    let parameters = parameters.map_err(|error| ApiError::internal(error.to_string()))?;
    ApiError::require_recent_era(parameters.era)?;
    let balanced = state
        .wallet_service
        .balance_partial(&id, &raw, &parameters)
        .await
        .map_err(ApiError::from)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"transaction":hex::encode(balanced),"encoding":"base16"})),
    ))
}

fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    let alphabet = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (i, &c) in alphabet.iter().enumerate() {
        lookup[c as usize] = i as u8;
    }

    let input = input.trim().as_bytes();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buf = [0u8; 4];
    let mut pos = 0;

    for &byte in input {
        if byte == b'=' {
            break;
        }
        let val = lookup[byte as usize];
        if val == 255 {
            return Err(format!("invalid base64 character: {}", byte as char));
        }
        buf[pos % 4] = val;
        pos += 1;
        if pos % 4 == 0 {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
            out.push((buf[2] << 6) | buf[3]);
        }
    }

    match pos % 4 {
        2 => out.push((buf[0] << 2) | (buf[1] >> 4)),
        3 => {
            out.push((buf[0] << 2) | (buf[1] >> 4));
            out.push((buf[1] << 4) | (buf[2] >> 2));
        }
        _ => {}
    }

    Ok(out)
}

#[cfg(test)]
mod transaction_tests {
    use super::*;
    use pallas_primitives::{conway, Fragment, NonEmptySet};
    use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
    use quetzal_primitives::{Coin, UtxoEntry, UtxoId, Value};
    use quetzal_wallet::{db::WalletDb, WalletService};

    // Sled's background flusher can briefly retain the lock after the last
    // foreground handle is dropped. Retry only that transient fixture error.
    async fn reopen<T, E: std::fmt::Display>(mut open: impl FnMut() -> Result<T, E>) -> T {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            match open() {
                Ok(database) => return database,
                Err(error)
                    if error.to_string().contains("could not acquire lock")
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("failed to reopen fixture database: {error}"),
            }
        }
    }

    #[tokio::test]
    async fn repeated_signing_preserves_other_witnesses_and_body() {
        let path = std::env::temp_dir().join(format!("quetzal-signing-{}", uuid::Uuid::new_v4()));
        let path_str = path.to_str().unwrap();
        let passphrase = "public-signing-test-passphrase";
        let service = WalletService::new(path_str).unwrap();
        let wallet = service
            .create_wallet(
                "signing".into(),
                &[43; 20],
                passphrase.as_bytes(),
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let address = service.get_addresses(&wallet.id).await.unwrap().remove(0);
        let utxo = UtxoEntry {
            id: UtxoId::new(TxHash([7; 32]), 0),
            address: address.address.clone(),
            value: Value::lovelace(Coin(10_000_000)),
        };
        drop(service);
        let db = reopen(|| WalletDb::open(path_str)).await;
        db.insert_utxo(&wallet.id, &utxo).unwrap();
        db.flush().unwrap();
        drop(db);

        let built = StagingTransaction::new()
            .network_id(0)
            .fee(200_000)
            .input(Input::new(utxo.id.tx_hash.0.into(), 0))
            .output(Output::new(
                pallas_addresses::Address::from_bech32(address.address.as_str()).unwrap(),
                9_800_000,
            ))
            .build_conway_raw()
            .unwrap();
        let mut tx = conway::Tx::decode_fragment(&built.tx_bytes.0).unwrap();
        // A legal indefinite body map must survive signing byte-for-byte.
        let body = tx.transaction_body.encode_fragment().unwrap();
        assert!(body[0] >= 0xa0 && body[0] <= 0xb7);
        let mut raw_body = vec![0xbf];
        raw_body.extend_from_slice(&body[1..]);
        raw_body.push(0xff);
        let body_hash = pallas_crypto::hash::Hasher::<256>::hash(&raw_body);
        let other_key = quetzal_crypto::derive_icarus_master_key(&[44; 20], b"").unwrap();
        let other_witness = quetzal_wallet::writer::vkey_witness(&body_hash, &other_key);
        tx.transaction_witness_set.vkeywitness = NonEmptySet::from_vec(vec![other_witness.clone()]);
        let script = conway::NativeScript::ScriptPubkey([9; 28].into());
        tx.transaction_witness_set.native_script = NonEmptySet::from_vec(vec![script.clone()]);
        let state = AppState::new(
            reopen(|| WalletService::new(path_str)).await,
            Default::default(),
        )
        .unwrap();
        let mut raw = vec![0x84];
        raw.extend_from_slice(&raw_body);
        raw.extend(pallas_codec::minicbor::to_vec(&tx.transaction_witness_set).unwrap());
        raw.extend_from_slice(&[0xf5, 0xf6]);
        let mut encoded = hex::encode(raw);
        for signing_round in 0..2 {
            let (status, Json(response)) = sign_transaction(
                State(state.clone()),
                Path(wallet.id.to_hex()),
                Json(SignRequest {
                    passphrase: passphrase.into(),
                    transaction: encoded.clone(),
                }),
            )
            .await
            .unwrap();
            assert_eq!(status, StatusCode::ACCEPTED);
            let signed_hex = response["transaction"].as_str().unwrap();
            let signed = conway::Tx::decode_fragment(&hex::decode(signed_hex).unwrap()).unwrap();
            assert_eq!(
                quetzal_wallet::txsize::body_hash(&hex::decode(signed_hex).unwrap()).unwrap(),
                *body_hash
            );
            assert_eq!(
                *pallas_traverse::MultiEraTx::decode(&hex::decode(signed_hex).unwrap())
                    .unwrap()
                    .hash(),
                *body_hash
            );
            let witnesses = signed.transaction_witness_set.vkeywitness.unwrap();
            assert_eq!(witnesses.len(), 2);
            assert!(witnesses.iter().any(|w| w == &other_witness));
            for witness in witnesses.iter() {
                assert!(quetzal_crypto::verify(
                    &witness.vkey.as_slice().try_into().unwrap(),
                    body_hash.as_ref(),
                    &witness.signature.as_slice().try_into().unwrap(),
                ));
            }
            assert_eq!(
                signed.transaction_witness_set.native_script.unwrap()[0],
                script
            );
            if signing_round > 0 {
                assert_eq!(signed_hex, encoded);
            }
            encoded = signed_hex.into();
        }
        let raw = hex::decode(&encoded).unwrap();
        let pending_id = state
            .wallet_service
            .submit_pending(&wallet.id, raw)
            .await
            .unwrap();
        assert_eq!(pending_id.0, *body_hash);
        let (_, Json(decoded)) = decode_transaction(
            State(state.clone()),
            Path(wallet.id.to_hex()),
            Json(DecodeRequest {
                transaction: encoded,
                decrypt_metadata: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(decoded["id"], hex::encode(body_hash));
        drop(state);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn history_accepts_zero_limit_and_filters_individual_withdrawals() {
        use quetzal_primitives::tx::{BlockHeight, CoinBundle, TxBlock, TxTime, Withdrawal};
        use quetzal_primitives::{TxDirection, TxRecord, TxStatus};
        let path = std::env::temp_dir().join(format!("quetzal-history-{}", uuid::Uuid::new_v4()));
        let path_str = path.to_str().unwrap();
        let service = WalletService::new(path_str).unwrap();
        let wallet = service
            .create_wallet(
                "history".into(),
                &[45; 20],
                b"test-passphrase",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let address = service
            .get_addresses(&wallet.id)
            .await
            .unwrap()
            .remove(0)
            .address;
        drop(service);
        let db = reopen(|| WalletDb::open(path_str)).await;
        for (index, amounts) in [vec![], vec![3, 3], vec![6]].into_iter().enumerate() {
            db.insert_tx(
                &wallet.id,
                &TxRecord {
                    id: TxHash([index as u8; 32]),
                    slot: index as u64,
                    block_hash: [0; 32],
                    block_height: 0,
                    direction: TxDirection::Incoming,
                    amount: Coin(10),
                    fee: None,
                    inputs: vec![],
                    outputs: vec![],
                    status: TxStatus::InLedger,
                    inserted_at: TxTime {
                        time: String::new(),
                        block: TxBlock {
                            slot_no: index as u64,
                            epoch_no: 0,
                            height: BlockHeight::new(0),
                        },
                    },
                    metadata: None,
                    withdrawals: amounts
                        .into_iter()
                        .map(|amount| Withdrawal {
                            stake_address: address.clone(),
                            amount: CoinBundle::lovelace(Coin(amount)),
                        })
                        .collect(),
                    expires_at_slot: None,
                    certificates: vec![],
                    deposit_taken: Coin::ZERO,
                    deposit_returned: Coin::ZERO,
                    stake_derivation_path: None,
                },
            )
            .unwrap();
        }
        db.flush().unwrap();
        drop(db);
        let state = AppState::new(
            reopen(|| WalletService::new(path_str)).await,
            Default::default(),
        )
        .unwrap();
        for (maximum, minimum, expected) in [
            (None, None, vec![2, 1, 0]),
            (Some(0), None, vec![]),
            (None, Some(1), vec![2, 1]),
            (None, Some(4), vec![2]),
            (None, Some(7), vec![]),
            (Some(1), Some(1), vec![2]),
        ] {
            let Json(rows) = list_transactions(
                State(state.clone()),
                Path(wallet.id.to_hex()),
                Query(TxListQuery {
                    max_count: maximum,
                    min_withdrawal: minimum,
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
            assert_eq!(
                rows.iter().map(|row| row.id.clone()).collect::<Vec<_>>(),
                expected
                    .into_iter()
                    .map(|index| TxHash([index; 32]).to_hex())
                    .collect::<Vec<_>>()
            );
        }
        let error = list_transactions(
            State(state.clone()),
            Path(WalletId([0; 20]).to_hex()),
            Query(TxListQuery {
                max_count: Some(0),
                ..Default::default()
            }),
        )
        .await
        .unwrap_err();
        assert_eq!(error.status, StatusCode::NOT_FOUND);
        drop(state);
        std::fs::remove_dir_all(path).unwrap();
    }
}
