use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use quetzal_wallet::WalletError;
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
    pub info: Option<serde_json::Value>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code: code.to_string(),
            message: message.into(),
            info: None,
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::NOT_FOUND, "no_such_wallet", message)
    }

    pub fn bad_request(code: &str, message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, code, message)
    }

    pub fn conflict(code: &str, message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::CONFLICT, code, message)
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        ApiError::new(
            StatusCode::FORBIDDEN,
            "wrong_encryption_passphrase",
            message,
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    pub fn require_recent_era(era: u16) -> Result<(), Self> {
        let Some(name) =
            ["Byron", "Shelley", "Allegra", "Mary", "Alonzo", "Babbage"].get(usize::from(era))
        else {
            return Ok(());
        };
        let mut error = Self::new(
            StatusCode::FORBIDDEN,
            "node_not_yet_in_recent_era",
            format!("This operation requires the node to be synchronised to a recent era, but the node is currently only synchronised to the {name} era. Please wait until the node is fully synchronised and try again."),
        );
        error.info =
            Some(json!({"node_era":name.to_ascii_lowercase(),"supported_recent_eras":["conway"]}));
        Err(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "code": self.code, "message": self.message });
        if let Some(info) = self.info {
            body["info"] = info;
        }
        (self.status, Json(body)).into_response()
    }
}

impl From<WalletError> for ApiError {
    fn from(e: WalletError) -> Self {
        match e {
            WalletError::WithdrawalNotBeneficial => ApiError::new(StatusCode::FORBIDDEN, "withdrawal_not_beneficial", "I've noticed that you're requesting a withdrawal from an account that is either empty or doesn't have a balance big enough to deserve being withdrawn. I won't proceed with that request."),
            WalletError::InvalidRandomWalletType => ApiError::new(StatusCode::FORBIDDEN, "invalid_wallet_type", "I cannot derive new address for this wallet type. Make sure to use Byron random wallet id."),
            WalletError::AddressNotOurs => ApiError::new(StatusCode::FORBIDDEN, "key_not_found_for_address", "I couldn't identify this address as one of mine. It likely belongs to another wallet and I will therefore not import it."),
            WalletError::AddressAlreadyExists(index) => ApiError::conflict("address_already_exists", format!("I cannot derive a new unused address #{index} because I already know of such address.")),
            WalletError::TransactionInLedger(id)=>{
                let mut error=ApiError::new(StatusCode::FORBIDDEN,"transaction_already_in_ledger",format!("The transaction with id: {id} cannot be forgotten as it is already in the ledger."));
                error.info=Some(json!({"transaction_id":id}));error
            },
            WalletError::TransactionNotFound(id) => {
                let mut error = ApiError::new(
                    StatusCode::NOT_FOUND,
                    "no_such_transaction",
                    format!("I couldn't find a transaction with the given id: {id}"),
                );
                error.info = Some(json!({"transaction_id":id}));
                error
            }
            WalletError::NotFound(id) => {
                let mut error = ApiError::not_found(format!(
                    "I couldn't find a wallet with the given id: {id}"
                ));
                error.info = Some(json!({"wallet_id": id}));
                error
            }
            WalletError::AlreadyExists(id) => ApiError::conflict(
                "wallet_already_exists",
                format!("This operation would yield a wallet with the following id: {id} However, I already know of a wallet with this id."),
            ),
            WalletError::WrongPassphrase => ApiError::forbidden("Wrong passphrase"),
            WalletError::WrongMnemonic(id) => ApiError::new(StatusCode::FORBIDDEN,"wrong_mnemonic",format!("The given mnemonic doesn't match the one this wallet was created with : {id}")),
            WalletError::Balancing { code, message } | WalletError::SharedWallet { code, message } => ApiError::new(StatusCode::FORBIDDEN, code, message),
            WalletError::UnsupportedTransaction(message) => ApiError::new(StatusCode::NOT_IMPLEMENTED, "not_implemented", message),
            WalletError::MissingPolicyPublicKey => ApiError::new(
                StatusCode::FORBIDDEN, "missing_policy_public_key",
                "It seems the wallet lacks a policy public key. Therefore it's not possible to create a minting/burning transaction or get a policy id. Please first POST to endpoint /wallets/{walletId}/policy-key to set a policy key."),
            WalletError::InvalidPolicyTemplate => ApiError::new(
                StatusCode::FORBIDDEN, "created_wrong_policy_script_template",
                "It looks like policy id is requested for a policy script that either does not pass validation, contains more than one cosigner, or has a cosigner that is different from cosigner#0."),
            WalletError::NoRootKey(id) => {
                ApiError::new(
                    StatusCode::FORBIDDEN,
                    "no_root_key",
                    format!("I couldn't find a root private key for the given wallet: {id}. However, this operation requires that I do have such a key. Either there's no such wallet, or I don't fully own it."),
                )
            }
            WalletError::InsufficientFunds => {
                ApiError::bad_request("not_enough_money", "Insufficient funds")
            }
            WalletError::NoUtxos => ApiError::new(StatusCode::FORBIDDEN, "no_utxos_available", "Unable to create a transaction because the wallet has no unspent transaction outputs (UTxOs) available. A transaction must spend at least one UTxO in order to be accepted for inclusion on the Cardano blockchain."),
            WalletError::NotDelegating => ApiError::new(StatusCode::FORBIDDEN, "not_delegating_to", "It seems that you're trying to retire from delegation although you're not even delegating, nor won't be in an immediate future."),
            WalletError::CoinSelection => {
                ApiError::bad_request("cannot_cover_fee", "Coin selection failed")
            }
            WalletError::TxTooLarge => ApiError::new(
                StatusCode::FORBIDDEN,
                "transaction_is_too_big",
                "The transaction exceeds the maximum size allowed by the ledger.",
            ),
            WalletError::OutputTooSmall { minimum } => ApiError::new(
                StatusCode::FORBIDDEN,
                "utxo_too_small",
                format!("The output requires at least {minimum} lovelace."),
            ),
            WalletError::OutputCoinTooSmall { index, specified, minimum } => {
                let mut error = ApiError::new(StatusCode::FORBIDDEN, "utxo_too_small", "One of the outputs you've specified has an ada quantity that is below the minimum required. Either increase the ada quantity to at least the minimum, or specify an ada quantity of zero, in which case the wallet will automatically assign the correct minimum ada quantity to the output.");
                error.info = Some(json!({"tx_output_index":index,"tx_output_lovelace_required_minimum":{"quantity":minimum,"unit":"lovelace"},"tx_output_lovelace_specified":{"quantity":specified,"unit":"lovelace"}}));
                error
            },
            other => ApiError::internal(other.to_string()),
        }
    }
}

/// Axum extractor rejections must use the same JSON envelope as handler errors.
pub async fn normalize_rejection(response: Response) -> Response {
    let status = response.status();
    if matches!(
        status,
        StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::UNSUPPORTED_MEDIA_TYPE
    ) && !response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .is_some_and(|value| value == "application/json")
    {
        return transport_error(status, "application/json").into_response();
    }
    if matches!(
        status,
        StatusCode::BAD_REQUEST
            | StatusCode::UNPROCESSABLE_ENTITY
            | StatusCode::UNSUPPORTED_MEDIA_TYPE
    ) && !response
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .is_some_and(|value| value == "application/json")
    {
        let mut message = match axum::body::to_bytes(response.into_body(), 65536).await {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => "Invalid request".to_owned(),
        };
        if message.contains("EOF while parsing an object") {
            message = "Unexpected end-of-input, expecting record key literal or }".to_owned();
        }
        if let Some((path, reason)) = message
            .strip_prefix("Failed to deserialize the JSON body into the target type: ")
            .and_then(|message| message.split_once(": "))
        {
            if reason.starts_with("parsing ") {
                let reason = reason
                    .rsplit_once(" at line ")
                    .map_or(reason, |(reason, _)| reason);
                let path = if reason.contains("invalid coin value:") {
                    path.strip_suffix(".amount.quantity").unwrap_or(path)
                } else {
                    path
                };
                let mut pointer = String::from("$");
                for segment in path.split('.') {
                    let (key, indices) = segment.find('[').map_or((segment, ""), |position| {
                        (&segment[..position], &segment[position..])
                    });
                    if key.contains('_') {
                        pointer.push_str(&format!("['{key}']"));
                    } else if !key.is_empty() {
                        pointer.push('.');
                        pointer.push_str(key);
                    }
                    pointer.push_str(indices);
                }
                message = format!("Error in {pointer}: {reason}");
            }
        }
        let status = if status == StatusCode::UNPROCESSABLE_ENTITY {
            StatusCode::BAD_REQUEST
        } else {
            status
        };
        return ApiError::new(status, "bad_request", message).into_response();
    }
    response
}

pub fn transport_error(status: StatusCode, media: &str) -> ApiError {
    let (code, message) = match status {
        StatusCode::NOT_FOUND => ("not_found", "I couldn't find the requested endpoint. If the endpoint contains path parameters, please ensure they are well-formed, otherwise I won't be able to route them correctly.".to_owned()),
        StatusCode::METHOD_NOT_ALLOWED => ("method_not_allowed", "You've reached a known endpoint but I don't know how to handle the HTTP method specified. Please double-check both the endpoint and the method: one of them is likely to be incorrect (for example: POST instead of PUT, or GET instead of POST...).".to_owned()),
        StatusCode::NOT_ACCEPTABLE => ("not_acceptable", format!("It seems as though you don't accept '{media}', but unfortunately I only speak '{media}'! Please double-check your 'Accept' request header and make sure it's set to '{media}'.")),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ("unsupported_media_type", format!("I'm really sorry but I only understand '{media}'. I need you to tell me what language you're speaking in order for me to understand your message. Please double-check your 'Content-Type' request header and make sure it's set to '{media}'.")),
        _ => ("bad_request", "Invalid request".to_owned()),
    };
    ApiError::new(status, code, message)
}

pub async fn negotiate(request: axum::extract::Request, next: axum::middleware::Next) -> Response {
    use axum::extract::FromRequestParts;
    let (mut parts, body) = request.into_parts();
    let parameters = axum::extract::RawPathParams::from_request_parts(&mut parts, &()).await;
    let request = axum::extract::Request::from_parts(parts, body);
    if let Ok(parameters) = parameters {
        for (name, value) in parameters.iter() {
            let invalid = match name {
                "id" | "wallet_id" if quetzal_primitives::WalletId::from_hex(value).is_err() => Some("wallet id should be a hex-encoded string of 40 characters"),
                "pool_id" if !(value == "*" && request.method() == axum::http::Method::DELETE) && quetzal_primitives::PoolId::from_hex(value).is_err() && !quetzal_address::codec::bech32_decode(value).is_ok_and(|(hrp, bytes)| hrp == "pool" && bytes.len() == 28) => Some("Invalid stake pool id: expecting a Bech32 encoded value with human readable part of 'pool'."),
                "drep_id" if !matches!(value, "abstain" | "no_confidence") && !valid_drep(value) => Some("Invalid DRep key/script hash: expecting a Bech32 encoded value with human readable part of 'drep', 'drep_vkh' or 'drep_script'."),
                _ => None,
            };
            if let Some(message) = invalid {
                return ApiError::bad_request("bad_request", message).into_response();
            }
        }
    }
    // Servant's NoContent endpoints have no representation to negotiate.
    let segments: Vec<_> = request
        .uri()
        .path()
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let wallet_family = segments
        .get(1)
        .is_some_and(|s| matches!(*s, "wallets" | "byron-wallets" | "shared-wallets"));
    let no_content = match request.method().as_str() {
        "DELETE" => {
            wallet_family
                && (segments.len() == 3 || (segments.len() == 5 && segments[3] == "transactions"))
        }
        "PUT" => {
            request.uri().path() == "/v2/settings"
                || (wallet_family && segments.len() == 4 && segments[3] == "passphrase")
        }
        "POST" => request.uri().path() == "/v2/stake-pools/maintenance-actions",
        _ => false,
    };
    let wallet = request
        .uri()
        .path()
        .split('/')
        .nth(3)
        .filter(|value| value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(str::to_owned);
    let media = if request.uri().path().contains("/signatures/") {
        "application/octet-stream"
    } else {
        "application/json"
    };
    let accepted = request
        .headers()
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_none_or(|accept| {
            accept.split(',').any(|item| {
                let mut parts = item.trim().split(';');
                let mime = parts.next().unwrap_or_default().trim();
                let quality = parts
                    .filter_map(|part| part.trim().strip_prefix("q="))
                    .next()
                    .and_then(|q| q.parse::<f64>().ok())
                    .unwrap_or(1.0);
                quality > 0.0
                    && (mime.eq_ignore_ascii_case(media)
                        || mime == "*/*"
                        || mime == "application/*")
            })
        });
    if !no_content && !accepted {
        // The signature endpoint produces octets on success, but Servant's
        // JSON error representation determines its negotiation rejection.
        return transport_error(StatusCode::NOT_ACCEPTABLE, "application/json").into_response();
    }
    if request.uri().path() == "/v2/proxy/transactions"
        && !request
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.split(';').next().is_some_and(|mime| {
                    mime.trim().eq_ignore_ascii_case("application/octet-stream")
                })
            })
    {
        return transport_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "application/octet-stream",
        )
        .into_response();
    }
    let response = next.run(request).await;
    if response.status() != StatusCode::FORBIDDEN || wallet.is_none() {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 65536).await {
        Ok(bytes) => bytes,
        Err(error) => return ApiError::internal(error.to_string()).into_response(),
    };
    if let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
        if value["code"] == "wrong_encryption_passphrase" {
            if let Some(id) = wallet {
                value["info"] = json!({"wallet_id":id});
                value["message"]=json!(format!("The given encryption passphrase doesn't match the one I use to encrypt the root private key of the given wallet: {id}"));
                parts.headers.remove(axum::http::header::CONTENT_LENGTH);
                return Response::from_parts(parts, axum::body::Body::from(value.to_string()));
            }
        }
    }
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

fn valid_drep(value: &str) -> bool {
    quetzal_address::codec::bech32_decode(value).is_ok_and(|(hrp, _)| {
        matches!(hrp.as_str(), "drep" | "drep_vkh" | "drep_script")
            && (quetzal_address::codec::governance_decode(value, false).is_ok()
                || quetzal_address::codec::governance_decode(value, true).is_ok())
    })
}
