use axum::{
    extract::{Path, Query, State},
    Json,
};
use rand::seq::SliceRandom;
use serde::Deserialize;

use crate::{error::ApiError, state::AppState};

async fn current(state: &AppState) -> Result<Vec<quetzal_wallet::drep::DRepRegistration>, String> {
    if let Some(rows) = state.drep_layer.cached_dreps().await {
        return Ok(rows);
    }
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        quetzal_network::NodeClient::dreps_at_tip(
            &state.config.socket_path,
            state.config.network_magic,
        ),
    )
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let previous = state.drep_layer.list_dreps().await;
    let mut registrations = Vec::with_capacity(snapshot.registrations.len());
    for entry in snapshot.registrations {
        let mut payload = vec![0x22 + entry.kind];
        payload.extend(entry.hash);
        let id =
            quetzal_address::codec::bech32_encode("drep", &payload).map_err(|e| e.to_string())?;
        let (anchor_url, anchor_data_hash) = entry
            .anchor
            .map(|(url, hash)| (Some(url), Some(hex::encode(hash))))
            .unwrap_or_default();
        let metadata = anchor_data_hash
            .as_ref()
            .and_then(|hash| {
                previous
                    .iter()
                    .find(|entry| entry.anchor_data_hash.as_ref() == Some(hash))
            })
            .and_then(|entry| entry.metadata.clone());
        registrations.push(quetzal_wallet::drep::DRepRegistration {
            id,
            kind: entry.kind,
            hash: entry.hash,
            deposit: entry.deposit,
            expiry_epoch: entry.expiry_epoch,
            is_active: entry.expiry_epoch >= snapshot.epoch,
            voting_power: entry.voting_power,
            anchor_url,
            anchor_data_hash,
            metadata,
        });
    }
    state.drep_layer.update(registrations.clone()).await;
    Ok(registrations)
}

pub(crate) async fn refresh(state: &AppState) -> Result<(), String> {
    let mut registrations = current(state).await?;
    for entry in &mut registrations {
        if let Some((url, hash)) = entry
            .anchor_url
            .as_ref()
            .zip(entry.anchor_data_hash.as_ref())
        {
            if entry.metadata.is_none() {
                entry.metadata = fetch_metadata(url, hash).await;
            }
        }
    }
    state.drep_layer.update_metadata(&registrations).await;
    Ok(())
}

async fn fetch_metadata(url: &str, hash: &str) -> Option<quetzal_wallet::drep::DRepMetadata> {
    let url = if let Some(path) = url.strip_prefix("ipfs://") {
        format!(
            "https://ipfs.blockfrost.dev/ipfs/{}",
            path.strip_prefix("ipfs/").unwrap_or(path)
        )
    } else {
        url.to_string()
    };
    let url = reqwest::Url::parse(&url).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let raw = super::pool_catalogue::bounded_get(url, 1024 * 1024)
        .await
        .ok()?;
    if hex::encode(pallas_crypto::hash::Hasher::<256>::hash(&raw)) != hash {
        return None;
    }
    parse_metadata(serde_json::from_slice(&raw).ok()?)
}

fn parse_metadata(top: serde_json::Value) -> Option<quetzal_wallet::drep::DRepMetadata> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Fields {
        given_name: String,
        objectives: Option<String>,
        motivations: Option<String>,
        qualifications: Option<String>,
        payment_address: Option<String>,
    }
    let fields: Fields = serde_json::from_value(
        top.get("body")
            .filter(|body| !body.is_null())
            .unwrap_or(&top)
            .clone(),
    )
    .ok()?;
    let references = top
        .get("references")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let do_not_list = top
        .get("doNotList")
        .filter(|v| !v.is_null())
        .cloned()
        .unwrap_or(serde_json::Value::Bool(false));
    Some(quetzal_wallet::drep::DRepMetadata {
        name: Some(fields.given_name),
        objectives: fields.objectives,
        motivations: fields.motivations,
        qualifications: fields.qualifications,
        payment_address: fields.payment_address,
        references: serde_json::from_value(references).ok()?,
        do_not_list: serde_json::from_value(do_not_list).ok()?,
    })
}

#[derive(Debug, Deserialize)]
pub struct DRepQuery {
    pub count: Option<usize>,
}

/// Build the JSON representation of a single DRep registration.
fn drep_to_json(reg: &quetzal_wallet::drep::DRepRegistration, detail: bool) -> serde_json::Value {
    let status = if reg.is_active { "active" } else { "inactive" };

    serde_json::json!({
        "id": reg.id,
        "credential":{"type":if reg.kind == 0 {"key_hash"} else {"script_hash"},"hash":hex::encode(reg.hash)},
        "expiry_epoch":reg.expiry_epoch,
        "deposit":{"quantity":reg.deposit,"unit":"lovelace"},
        "anchor": reg.anchor_url.as_ref().zip(reg.anchor_data_hash.as_ref()).map(|(url,hash)|serde_json::json!({"url":url,"data_hash":hash})),
        "name":reg.metadata.as_ref().and_then(|metadata|metadata.name.as_ref()),
        "voting_power": {
            "quantity": reg.voting_power.to_string(),
            "unit": "lovelace"
        },
        "status": status,
        "metadata": if detail {reg.metadata.as_ref()} else {None},
    })
}

/// `GET /v2/dreps`
///
/// Return the full list of known DRep registrations.
pub async fn list_dreps(
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let dreps = current(&state).await.map_err(ApiError::internal)?;
    let body: Vec<serde_json::Value> = dreps.iter().map(|r| drep_to_json(r, false)).collect();

    Ok(Json(body))
}

/// `GET /v2/dreps/summary`
///
/// Return aggregate statistics for all known DReps.
pub async fn get_drep_summary(
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let dreps = current(&state).await.map_err(ApiError::internal)?;

    let total = dreps.len();
    let active = dreps.iter().filter(|r| r.is_active).count();
    let inactive = total - active;
    let total_stake = dreps
        .iter()
        .try_fold(0u64, |total, r| total.checked_add(r.voting_power))
        .ok_or_else(|| ApiError::internal("DRep stake overflow"))?;

    Ok(Json(serde_json::json!({
        "total_drep_count": total,
        "active_drep_count": active,
        "inactive_drep_count": inactive,
        "total_drep_stake": {
            "quantity": total_stake.to_string(),
            "unit": "lovelace"
        }
    })))
}

/// `GET /v2/dreps/suggested`
///
/// Return a random sample of recommended DReps.
///
/// Algorithm:
///   1. Sort by `voting_power` descending and drop the top 35.
///   2. Keep active DReps with metadata and `do_not_list == false`.
///   3. Return a uniform random sample of up to `count` (default 20, max 200).
pub async fn get_suggested_dreps(
    State(state): State<AppState>,
    Query(q): Query<DRepQuery>,
) -> Result<Json<Vec<serde_json::Value>>, ApiError> {
    let count = q.count.unwrap_or(20).min(200);

    let mut candidates = current(&state).await.map_err(ApiError::internal)?;
    candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.voting_power));
    candidates.drain(..35.min(candidates.len()));
    candidates.retain(|r| r.is_active && r.metadata.as_ref().is_some_and(|m| !m.do_not_list));

    // Uniform random sample of up to `count`.
    let mut rng = rand::thread_rng();
    let sample_size = count.min(candidates.len());
    let (sampled, _) = candidates.partial_shuffle(&mut rng, sample_size);

    let body: Vec<serde_json::Value> = sampled.iter().map(|r| drep_to_json(r, false)).collect();

    Ok(Json(body))
}

/// `GET /v2/dreps/:drep_id`
///
/// Return the full detail for a single DRep.  Returns 404 if not found.
pub async fn get_drep(
    State(state): State<AppState>,
    Path(drep_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let credential = quetzal_wallet::staking::drep(&drep_id).map_err(ApiError::from)?;
    let dreps = current(&state).await.map_err(ApiError::internal)?;
    match dreps.iter().find(|r| match &credential {
        pallas_primitives::conway::DRep::Key(hash) => r.kind == 0 && hash.as_ref() == r.hash,
        pallas_primitives::conway::DRep::Script(hash) => r.kind == 1 && hash.as_ref() == r.hash,
        _ => false,
    }) {
        Some(reg) => Ok(Json(drep_to_json(reg, true))),
        None => Err(ApiError::new(
            axum::http::StatusCode::NOT_FOUND,
            "not_found",
            "No DRep found for the given identifier.",
        )),
    }
}

/// `PUT /v2/dreps/:drep_id/wallets/:wallet_id`
///
/// Construct, sign and submit an actual Conway voting delegation.
pub async fn vote_for_drep(
    State(state): State<AppState>,
    Path((drep_id, wallet_id)): Path<(String, String)>,
    Json(req): Json<super::stake_pools::DelegationRequest>,
) -> Result<
    (
        axum::http::StatusCode,
        Json<crate::json::transaction::TransactionResponse>,
    ),
    ApiError,
> {
    super::transactions::construct_sign_submit(
        state,
        wallet_id,
        super::transactions::ConstructRequest {
            vote: Some(drep_id),
            ..Default::default()
        },
        req.passphrase,
        false,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn metadata_requires_authentic_bytes_and_well_typed_cip119_fields() {
        use axum::{routing::get, Router};
        for (raw, bad_hash, accepted) in [
            (r#"{"givenName":"Alice","references":[{"label":"Site","uri":"https://example.com"}]}"#.to_string(), false, true),
            (r#"{"body":{"givenName":"Bob","objectives":"Participation"},"doNotList":true}"#.to_string(), false, true),
            (r#"{"givenName":"Alice"}"#.to_string(), true, false),
            (r#"{"givenName":42}"#.to_string(), false, false),
            (" ".repeat(1024 * 1024 + 1), false, false),
        ] {
            let hash = if bad_hash {hex::encode([0;32])} else {hex::encode(pallas_crypto::hash::Hasher::<256>::hash(raw.as_bytes()))};
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let app = Router::new().route("/",get(move ||{let raw=raw.clone(); async move {raw}}));
            let task = tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
            let result = fetch_metadata(&format!("http://{address}/"),&hash).await;
            assert_eq!(result.is_some(),accepted);
            task.abort();
            let _ = task.await;
        }
    }
}
