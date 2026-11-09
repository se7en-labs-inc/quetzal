use crate::{error::ApiError, state::AppState};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    Json,
};
use serde::Deserialize;

pub async fn get_settings(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({
        "pool_metadata_source": state.wallet_service.pool_metadata_source().map_err(ApiError::from)?
    })))
}

#[derive(Deserialize)]
pub struct SettingsPut {
    settings: Settings,
}
#[derive(Deserialize)]
pub struct Settings {
    pool_metadata_source: String,
}

fn absolute_uri(value: &str) -> Result<reqwest::Url, ApiError> {
    reqwest::Url::parse(value)
        .ok()
        .filter(|url| url.host_str().is_some())
        .ok_or_else(|| {
            ApiError::bad_request(
                "bad_request",
                "Error parsing query parameter url failed: Not a valid absolute URI.",
            )
        })
}

pub async fn put_settings(
    State(state): State<AppState>,
    Json(req): Json<SettingsPut>,
) -> Result<StatusCode, ApiError> {
    let source = &req.settings.pool_metadata_source;
    if source != "direct" && source != "none" {
        absolute_uri(source)?;
    }
    state
        .wallet_service
        .set_pool_metadata_source(source)
        .map_err(ApiError::from)?;
    super::pool_catalogue::schedule_gc(state);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Default, Deserialize)]
pub struct SmashQuery {
    url: Option<String>,
}

async fn smash_health(mut url: reqwest::Url) -> Result<&'static str, reqwest::Error> {
    url.set_path("/api/v1/status");
    url.set_query(None);
    url.set_fragment(None);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > 65536 {
            return Ok("unreachable");
        }
        bytes.extend_from_slice(&chunk);
    }
    #[derive(Deserialize)]
    struct Health {
        status: String,
        version: String,
    }
    let Ok(health) = serde_json::from_slice::<Health>(&bytes) else {
        return Ok("unreachable");
    };
    tracing::debug!(version = %health.version, "Received SMASH health response");
    Ok(if health.status.eq_ignore_ascii_case("ok") {
        "available"
    } else {
        "unavailable"
    })
}

pub async fn get_smash_health(
    State(state): State<AppState>,
    Query(query): Query<SmashQuery>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let source = query.url.unwrap_or(
        state
            .wallet_service
            .pool_metadata_source()
            .map_err(ApiError::from)?,
    );
    let health = if source == "direct" || source == "none" {
        "no_smash_configured"
    } else {
        smash_health(absolute_uri(&source)?)
            .await
            .unwrap_or("unreachable")
    };
    Ok(Json(serde_json::json!({ "health": health })))
}

pub async fn proxy_transactions(
    State(state): State<AppState>,
    body: axum::body::Bytes,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    let transaction = pallas_traverse::MultiEraTx::decode(&body).map_err(|error| {
        let mut decoder = pallas_codec::minicbor::Decoder::new(&body);
        let reason = match decoder.skip() {
            Err(error) if error.is_end_of_input() => "end of input".to_owned(),
            _ => error.to_string(),
        };
        ApiError::bad_request(
            "bad_request",
            format!("DecoderErrorDeserialiseFailure 'ConwayTx' (DeserialiseFailure 0 '{reason}')"),
        )
    })?;
    let id = transaction.hash().to_string();
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|error| {
                ApiError::new(
                    axum::http::StatusCode::SERVICE_UNAVAILABLE,
                    "network_unreachable",
                    error.to_string(),
                )
            })?;
    let parameters = quetzal_network::StateQuerier::new(node.statequery())
        .protocol_params()
        .await;
    let parameters = match parameters {
        Ok(parameters) => parameters,
        Err(error) => {
            node.abort().await;
            return Err(ApiError::internal(error.to_string()));
        }
    };
    let result = quetzal_network::TxSubmitter::new(node.submission())
        .submit_era(parameters.era, body.to_vec())
        .await;
    node.abort().await;
    result.map_err(submission_error)?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({"id":id})),
    ))
}

// Conway ledger tags are pinned to the node/reference source. Decode only the
// recognized constructor chain; all other failures retain their original CBOR.
fn validity_rejection(raw: &[u8]) -> Option<String> {
    use pallas_codec::minicbor::Decoder;
    let mut d = Decoder::new(raw);
    if d.array().ok()? != Some(2)
        || d.u8().ok()? != 2
        || d.array().ok()? != Some(1)
        || d.array().ok()? != Some(2)
        || d.u16().ok()? != 6
        || d.array().ok()? != Some(1)
        || d.array().ok()? != Some(2)
        || d.u8().ok()? != 1
        || d.array().ok()? != Some(2)
        || d.u8().ok()? != 0
        || d.array().ok()? != Some(3)
        || d.u8().ok()? != 2
        || d.array().ok()? != Some(2)
    {
        return None;
    }
    let bound = |d: &mut Decoder<'_>| -> Option<Option<u64>> {
        match d.array().ok()? {
            Some(0) => Some(None),
            Some(1) => Some(Some(d.u64().ok()?)),
            _ => None,
        }
    };
    let lower = bound(&mut d)?;
    let upper = bound(&mut d)?;
    let slot = d.u64().ok()?;
    if d.position() != raw.len() {
        return None;
    }
    let show_bound = |slot: Option<u64>| {
        slot.map_or_else(
            || "SNothing".into(),
            |slot| format!("SJust (SlotNo {slot})"),
        )
    };
    let interval = format!("OutsideValidityIntervalUTxO (ValidityInterval {{invalidBefore = {}, invalidHereafter = {}}}) (SlotNo {slot})", show_bound(lower), show_bound(upper));
    Some(format!("HardForkApplyTxErrFromEra S (S (S (S (S (S (Z (WrapApplyTxErr {{unwrapApplyTxErr = ConwayApplyTxError (ConwayUtxowFailure (UtxoFailure ({interval})) :| [])}})))))))"))
}

fn witness_rejection(raw: &[u8]) -> Option<String> {
    use pallas_codec::minicbor::{data::Type, Decoder};
    let mut d = Decoder::new(raw);
    if d.array().ok()? != Some(2)
        || d.u8().ok()? != 2
        || d.array().ok()? != Some(1)
        || d.array().ok()? != Some(2)
        || d.u16().ok()? != 6
        || d.array().ok()? != Some(1)
        || d.array().ok()? != Some(2)
        || d.u8().ok()? != 1
        || d.array().ok()? != Some(2)
    {
        return None;
    }
    let (constructor, length) = match d.u8().ok()? {
        1 => ("InvalidWitnessesUTXOW", 32),
        2 => ("MissingVKeyWitnessesUTXOW", 28),
        3 => ("MissingScriptWitnessesUTXOW", 28),
        4 => ("ScriptWitnessNotValidatingUTXOW", 28),
        _ => return None,
    };
    if d.datatype().ok()? == Type::Tag && d.tag().ok()?.as_u64() != 258 {
        return None;
    }
    let count = d.array().ok()??;
    let mut hashes = Vec::new();
    for _ in 0..count {
        let hash = d.bytes().ok()?;
        if hash.len() != length {
            return None;
        }
        hashes.push(hex::encode(hash));
    }
    if d.position() != raw.len() {
        return None;
    }
    Some(format!(
        "ConwayUtxowFailure ({constructor} [{}])",
        hashes.join(",")
    ))
}

pub(super) fn submission_error(error: quetzal_network::NetworkError) -> ApiError {
    use axum::http::StatusCode;
    match error {
        quetzal_network::NetworkError::TxRejected(raw) => {
            // Pallas 0.32 returns the complete RejectTx message, including its tag.
            let mut decoder = pallas_codec::minicbor::Decoder::new(&raw);
            let _ = decoder.array();
            let _ = decoder.u8();
            let offset = decoder.position();
            let direct = decoder.str().ok().map(str::to_owned);
            decoder.set_position(offset);
            let mempool = quetzal_network::txsubmit::mempool_failure_message(&raw)
                .map(|message| format!("ConwayMempoolFailure {message:?}"));
            let reason = direct
                .or(mempool)
                .or_else(|| validity_rejection(&raw))
                .or_else(|| witness_rejection(&raw))
                .unwrap_or_else(|| {
                    tracing::warn!(reason_cbor=%hex::encode(&raw),"Structured ledger rejection");
                    format!("Ledger rejection (CBOR): {}", hex::encode(&raw))
                });
            ApiError::new(StatusCode::INTERNAL_SERVER_ERROR,"created_invalid_transaction",format!("The submitted transaction was rejected by the local node. Here's an error message that may help with debugging: {reason}"))
        }
        error => ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "network_unreachable",
            error.to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn smash_health_uses_real_status_endpoint_and_bounds_responses() {
        use axum::{routing::get, Router};
        for (body, expected) in [
            (
                r#"{"status":"OK","version":"public-test"}"#.to_owned(),
                "available",
            ),
            (
                r#"{"status":"maintenance","version":"public-test"}"#.to_owned(),
                "unavailable",
            ),
            (r#"{"status":"ok"}"#.to_owned(), "unreachable"),
            (" ".repeat(65537), "unreachable"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route(
                "/api/v1/status",
                get(move || {
                    let body = body.clone();
                    async move { body }
                }),
            );
            let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let url = reqwest::Url::parse(&format!(
                "http://{address}/ignored/path?discard=true#fragment"
            ))
            .unwrap();
            assert_eq!(smash_health(url).await.unwrap(), expected);
            task.abort();
            let _ = task.await;
        }
        assert!(absolute_uri("invalid").is_err());
    }

    #[test]
    fn captured_node_validity_rejection_preserves_ledger_bounds_and_slot() {
        let raw = hex::decode("82028182068182018200830282810081011901a4").unwrap();
        let reason = validity_rejection(&raw).unwrap();
        assert_eq!(reason, "HardForkApplyTxErrFromEra S (S (S (S (S (S (Z (WrapApplyTxErr {unwrapApplyTxErr = ConwayApplyTxError (ConwayUtxowFailure (UtxoFailure (OutsideValidityIntervalUTxO (ValidityInterval {invalidBefore = SJust (SlotNo 0), invalidHereafter = SJust (SlotNo 1)}) (SlotNo 420))) :| [])})))))))");
        assert!(reason.contains("OutsideValidityIntervalUTxO (ValidityInterval {invalidBefore = SJust (SlotNo 0), invalidHereafter = SJust (SlotNo 1)}) (SlotNo 420)"));
        assert!(reason.ends_with("))) :| [])})))))))"));
        let error = submission_error(quetzal_network::NetworkError::TxRejected(raw.clone()));
        assert_eq!(error.status, axum::http::StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code, "created_invalid_transaction");
        assert!(error.message.ends_with(&reason));
        for length in 0..raw.len() {
            assert!(validity_rejection(&raw[..length]).is_none());
        }
        let mut trailing = raw;
        trailing.push(0);
        assert!(validity_rejection(&trailing).is_none());
    }
}
