//! Node-backed pool catalogue and authenticated metadata retrieval.
use crate::{error::ApiError, state::AppState};
use pallas_codec::utils::Nullable;
use quetzal_primitives::{
    pool::{PoolGcState, PoolMargin, PoolRetirement},
    Coin, PoolId, PoolMeta, PoolStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Deserialize, Serialize)]
struct Metadata {
    name: String,
    ticker: String,
    homepage: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

pub(crate) async fn bounded_get(url: reqwest::Url, limit: usize) -> Result<Vec<u8>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| e.to_string())? {
        if result.len() + chunk.len() > limit {
            return Err("response exceeds size limit".into());
        }
        result.extend(chunk);
    }
    Ok(result)
}

async fn fetch_metadata(url: reqwest::Url, hash: &[u8; 32]) -> Option<Metadata> {
    let raw = bounded_get(url, 512).await.ok()?;
    if pallas_crypto::hash::Hasher::<256>::hash(&raw).as_ref() != hash {
        return None;
    }
    let value: Metadata = serde_json::from_slice(&raw).ok()?;
    let valid = (1..=50).contains(&value.name.chars().count())
        && (3..=5).contains(&value.ticker.chars().count())
        && value
            .description
            .as_ref()
            .is_none_or(|description| description.chars().count() <= 255)
        && reqwest::Url::parse(&value.homepage).is_ok();
    valid.then_some(value)
}

fn endpoint(source: &str, path: &str) -> Result<reqwest::Url, String> {
    let mut url = reqwest::Url::parse(source).map_err(|e| e.to_string())?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("metadata requires HTTP or HTTPS".into());
    }
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Ok(url)
}

pub(crate) async fn refresh_gc(state: &AppState) -> Result<(), String> {
    let source = state
        .wallet_service
        .pool_metadata_source()
        .map_err(|e| e.to_string())?;
    if source == "direct" || source == "none" {
        return Ok(());
    }
    #[derive(Deserialize)]
    struct Delisted {
        #[serde(rename = "poolId")]
        pool_id: String,
    }
    let raw = bounded_get(endpoint(&source, "/api/v1/delisted")?, 1024 * 1024).await?;
    let rows: Vec<Delisted> = serde_json::from_slice(&raw).map_err(|e| e.to_string())?;
    let delisted = rows
        .into_iter()
        .map(|row| PoolId::from_hex(&row.pool_id).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?;
    let now = chrono::DateTime::from_timestamp(
        now.as_secs().try_into().map_err(|_| "GC time overflow")?,
        now.subsec_nanos(),
    )
    .ok_or("GC time overflow")?;
    state
        .wallet_service
        .store_pool_gc(&PoolGcState {
            source,
            last_run: now.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
            delisted,
        })
        .map_err(|e| e.to_string())
}

pub(crate) fn schedule_gc(state: AppState) {
    tokio::spawn(async move {
        if let Err(error) = refresh_gc(&state).await {
            tracing::warn!(%error, "Pool metadata GC failed");
        }
    });
}

fn float(value: f64) -> Result<Value, ApiError> {
    #[derive(Serialize)]
    struct Number(#[serde(serialize_with = "quetzal_primitives::quantity::serialize_float")] f64);
    serde_json::to_value(Number(value)).map_err(|e| ApiError::internal(e.to_string()))
}

fn retirement(epoch: u64, state: &AppState) -> Result<PoolRetirement, ApiError> {
    let slot = state
        .config
        .epoch_first_slot(epoch)
        .ok_or_else(|| ApiError::internal("retirement slot overflow"))?;
    Ok(PoolRetirement {
        epoch_number: epoch
            .try_into()
            .map_err(|_| ApiError::internal("retirement epoch overflow"))?,
        epoch_start_time: state.config.slot_reference(slot).2,
    })
}

pub(crate) async fn list(state: &AppState, stake: u64) -> Result<Vec<Value>, ApiError> {
    let mut node =
        quetzal_network::NodeClient::connect(&state.config.socket_path, state.config.network_magic)
            .await
            .map_err(|e| ApiError::internal(e.to_string()))?;
    let snapshot = quetzal_network::StateQuerier::new(node.statequery())
        .pools(stake)
        .await;
    node.abort().await;
    let snapshot = snapshot.map_err(|e| ApiError::internal(e.to_string()))?;
    let counts = state
        .wallet_service
        .pool_block_counts()
        .map_err(ApiError::from)?;
    let registered = state
        .wallet_service
        .registered_pools()
        .map_err(ApiError::from)?;
    let source = state
        .wallet_service
        .pool_metadata_source()
        .map_err(ApiError::from)?;
    let gc = state.wallet_service.pool_gc().map_err(ApiError::from)?;
    let seed = state
        .wallet_service
        .pool_order_seed()
        .map_err(ApiError::from)?;
    let mut cache = Vec::new();
    let mut response: Vec<(u64, [u8; 32], Value)> = Vec::new();
    for (raw_id, pool) in snapshot.pools {
        if !registered.contains(&raw_id) {
            continue;
        }
        let raw_id: [u8; 28] = raw_id
            .try_into()
            .map_err(|_| ApiError::internal("Invalid node pool ID"))?;
        let id = PoolId(raw_id);
        let delisted = gc.as_ref().is_some_and(|gc| gc.delisted.contains(&id));
        let ratio = pool.relative_stake;
        let relative =
            quetzal_wallet::pool_metrics::percentage(ratio.0, ratio.1).map_err(ApiError::from)?;
        let saturation =
            quetzal_wallet::pool_metrics::saturation(ratio.0, ratio.1, snapshot.optimum_pool_count)
                .map_err(ApiError::from)?;
        let margin = &pool.parameters.margin;
        let margin_percentage =
            quetzal_wallet::pool_metrics::percentage(margin.numerator, margin.denominator)
                .map_err(ApiError::from)?;
        let cost = u64::from(&pool.parameters.cost);
        let pledge = u64::from(&pool.parameters.pledge);
        let produced_blocks = counts.get(raw_id.as_slice()).copied().unwrap_or(0);
        let retirement = pool
            .retirement
            .map(|epoch| retirement(epoch, state))
            .transpose()?;
        let flags = if delisted {
            vec!["delisted".to_owned()]
        } else {
            vec![]
        };
        let mut metadata = None;
        if source != "none" && !delisted {
            if let Nullable::Some(reference) = &pool.parameters.pool_metadata {
                let url = if source == "direct" {
                    reqwest::Url::parse(&reference.url).map_err(|e| e.to_string())
                } else {
                    endpoint(
                        &source,
                        &format!(
                            "/api/v1/metadata/{}/{}",
                            id.to_hex(),
                            hex::encode(reference.hash)
                        ),
                    )
                };
                if let Ok(url) = url {
                    metadata = fetch_metadata(url, &reference.hash).await;
                }
            }
        }
        let mut row = json!({"id":id.to_bech32(), "cost":{"quantity":cost,"unit":"lovelace"}, "pledge":{"quantity":pledge,"unit":"lovelace"}, "margin":{"quantity":float(margin_percentage)?,"unit":"percent"}, "flags":flags,
            "metrics":{"relative_stake":{"quantity":float(relative)?,"unit":"percent"}, "saturation":float(saturation)?, "produced_blocks":{"quantity":produced_blocks,"unit":"block"}, "non_myopic_member_rewards":{"quantity":pool.member_rewards,"unit":"lovelace"}}});
        if let Some(metadata) = &metadata {
            row["metadata"] =
                serde_json::to_value(metadata).map_err(|e| ApiError::internal(e.to_string()))?;
        }
        if let Some(retirement) = &retirement {
            row["retirement"] = json!({"epoch_number":retirement.epoch_number,"epoch_start_time":retirement.epoch_start_time});
        }
        cache.push(PoolMeta {
            id,
            name: metadata.as_ref().map(|m| m.name.clone()),
            ticker: metadata.as_ref().map(|m| m.ticker.clone()),
            description: metadata.as_ref().and_then(|m| m.description.clone()),
            homepage: metadata.as_ref().map(|m| m.homepage.clone()),
            margin: Some(PoolMargin {
                numerator: margin.numerator,
                denominator: margin.denominator,
            }),
            cost: Some(Coin(cost)),
            pledge: Some(Coin(pledge)),
            relative_stake: Some(relative),
            saturation: Some(saturation),
            produced_blocks,
            status: if retirement.is_some() {
                PoolStatus::Retiring
            } else {
                PoolStatus::Registered
            },
            retirement,
            flags,
        });
        let mut random_weight = seed.to_be_bytes().to_vec();
        random_weight.extend(raw_id);
        response.push((
            pool.member_rewards,
            *pallas_crypto::hash::Hasher::<256>::hash(&random_weight),
            row,
        ));
    }
    response.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    state
        .wallet_service
        .update_pool_cache(cache)
        .await
        .map_err(ApiError::from)?;
    Ok(response.into_iter().map(|(_, _, row)| row).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn fetched_metadata_requires_the_registered_hash_and_bounded_valid_fields() {
        use axum::{routing::get, Router};
        for (raw, wrong_hash, expected) in [
            (
                r#"{"name":"Test Pool","ticker":"TEST","homepage":"https://example.com"}"#
                    .to_owned(),
                false,
                true,
            ),
            (
                r#"{"name":"Test Pool","ticker":"TEST","homepage":"https://example.com"}"#
                    .to_owned(),
                true,
                false,
            ),
            (
                r#"{"name":"Test Pool","ticker":"TOO-LONG","homepage":"https://example.com"}"#
                    .to_owned(),
                false,
                false,
            ),
            (" ".repeat(513), false, false),
            ("not-json".into(), false, false),
        ] {
            let hash = if wrong_hash {
                [0; 32]
            } else {
                *pallas_crypto::hash::Hasher::<256>::hash(raw.as_bytes())
            };
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route(
                "/metadata",
                get(move || {
                    let raw = raw.clone();
                    async move { raw }
                }),
            );
            let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            let url = reqwest::Url::parse(&format!("http://{address}/metadata")).unwrap();
            assert_eq!(fetch_metadata(url, &hash).await.is_some(), expected);
            task.abort();
            let _ = task.await;
        }
    }

    #[tokio::test]
    async fn smash_gc_fetches_actual_delisted_ids_and_persists_the_result() {
        use axum::{routing::get, Router};
        let path = std::env::temp_dir().join(format!("quetzal-pool-gc-{}", rand::random::<u64>()));
        let service = quetzal_wallet::WalletService::new(path.to_str().unwrap()).unwrap();
        let state = AppState::new(service, crate::config::Config::default()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let router = Router::new().route(
            "/api/v1/delisted",
            get(|| async { axum::Json(json!([{"poolId":hex::encode([1;28])}])) }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        state
            .wallet_service
            .set_pool_metadata_source(&format!("http://{address}/ignored"))
            .unwrap();
        refresh_gc(&state).await.unwrap();
        let gc = state.wallet_service.pool_gc().unwrap().unwrap();
        assert_eq!(gc.delisted, vec![PoolId([1; 28])]);
        assert!(chrono::DateTime::parse_from_rfc3339(&gc.last_run).is_ok());
        state
            .wallet_service
            .set_pool_metadata_source("none")
            .unwrap();
        assert!(state.wallet_service.pool_gc().unwrap().is_none());
        task.abort();
        let _ = task.await;
        drop(state);
        std::fs::remove_dir_all(path).unwrap();
    }
}
