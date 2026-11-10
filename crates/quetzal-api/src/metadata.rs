//! Bounded cardano-wallet token-registry protocol and property validation.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const LIMIT: usize = 10 * 1024 * 1024;
const PROPERTIES: &[&str] = &["name", "description", "ticker", "url", "logo", "decimals"];

fn signature(value: &Value) -> bool {
    ["signature", "publicKey"]
        .iter()
        .all(|key| value[*key].as_str().is_some_and(|s| hex::decode(s).is_ok()))
}
fn property<'a>(row: &'a Value, name: &str) -> Result<Option<&'a Value>, ()> {
    let Some(p) = row.get(name).filter(|p| !p.is_null()) else {
        return Ok(None);
    };
    let Some(value) = p.get("value") else {
        return Err(());
    };
    if p.get("sequenceNumber")
        .filter(|v| !v.is_null())
        .is_some_and(|v| v.as_i64().is_none())
    {
        return Err(());
    }
    if let Some(signatures) = p.get("signatures").filter(|v| !v.is_null()) {
        if !signatures
            .as_array()
            .is_some_and(|xs| xs.iter().all(signature))
        {
            return Err(());
        }
    }
    Ok(Some(value))
}
fn text(value: Option<&Value>, min: usize, max: usize) -> Option<Value> {
    let s = value?.as_str()?;
    (min..=max).contains(&s.chars().count()).then(|| json!(s))
}
fn parse_row(row: &Value) -> Option<Value> {
    if row
        .get("owner")
        .filter(|v| !v.is_null())
        .is_some_and(|v| !signature(v))
    {
        return None;
    }
    let values = PROPERTIES
        .iter()
        .map(|name| property(row, name))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let name = text(values[0], 1, 50)?;
    let description = text(values[1], 0, 500)?;
    let ticker = text(values[2], 2, 6);
    let url = text(values[3], 0, 250).filter(|v| {
        reqwest::Url::parse(v.as_str().unwrap_or_default()).is_ok_and(|url| url.scheme() == "https")
    });
    let logo = values[4]
        .and_then(Value::as_str)
        .filter(|s| STANDARD.decode(s).is_ok_and(|bytes| bytes.len() <= 65536))
        .map(|s| json!(s));
    let decimals = values[5]
        .and_then(Value::as_u64)
        .filter(|n| *n <= 255)
        .map(|n| json!(n));
    let mut result = json!({"name":name,"description":description});
    for (key, value) in [
        ("ticker", ticker),
        ("url", url),
        ("logo", logo),
        ("decimals", decimals),
    ] {
        if let Some(value) = value {
            result[key] = value;
        }
    }
    Some(result)
}

/// None server and missing subjects are successful metadata absence. Fetch/parse
/// failures are surfaced as metadata_error without affecting the wallet ledger.
pub async fn fetch(
    server: Option<&reqwest::Url>,
    subjects: &[String],
) -> Result<BTreeMap<String, Value>, &'static str> {
    let Some(server) = server else {
        return Ok(BTreeMap::new());
    };
    if subjects.is_empty() {
        return Ok(BTreeMap::new());
    }
    let endpoint = server.join("metadata/query").map_err(|_| "fetch")?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|_| "fetch")?;
    let requested: BTreeSet<_> = subjects.iter().map(String::as_str).collect();
    let mut result = BTreeMap::new();
    // The reference supports batching. Bound each request and response while
    // still handling wallets with more assets than a single registry batch.
    for subjects in subjects.chunks(1000) {
        tracing::debug!(count = subjects.len(), "Fetching token registry metadata");
        let body = serde_json::to_vec(&json!({"subjects":subjects,"properties":PROPERTIES}))
            .map_err(|_| "parse")?;
        let mut response = client
            .post(endpoint.clone())
            .header("content-type", "application/json")
            .body(body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|_| "fetch")?;
        if response.content_length().is_some_and(|n| n > LIMIT as u64) {
            return Err("parse");
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "fetch")? {
            if bytes.len().saturating_add(chunk.len()) > LIMIT {
                return Err("parse");
            }
            bytes.extend(chunk);
        }
        let decoded: Value = serde_json::from_slice(&bytes).map_err(|_| "parse")?;
        let rows = decoded["subjects"].as_array().ok_or("parse")?;
        for row in rows {
            if let Some(subject) = row["subject"]
                .as_str()
                .filter(|subject| requested.contains(subject))
            {
                if let Some(metadata) = parse_row(row) {
                    result.insert(subject.into(), metadata);
                }
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn p(value: Value) -> Value {
        json!({"value":value})
    }
    #[test]
    fn required_and_optional_properties_follow_reference_validation() {
        let base = json!({"subject":"aa","name":p(json!("Asset")),"description":p(json!(""))});
        let mut row = base.clone();
        row["ticker"] = p(json!("X"));
        row["url"] = p(json!("http://insecure.example/"));
        row["logo"] = p(json!("invalid base64"));
        row["decimals"] = p(json!(256));
        assert_eq!(
            parse_row(&row),
            Some(json!({"name":"Asset","description":""}))
        );
        row["decimals"] = p(json!(6));
        row["ticker"] = p(json!("TOK"));
        assert_eq!(parse_row(&row).unwrap()["decimals"], 6);
        row["ticker"] = json!("malformed property wrapper");
        assert!(parse_row(&row).is_none());
        let mut invalid = base;
        invalid["name"] = p(json!(""));
        assert!(parse_row(&invalid).is_none());
    }
    #[tokio::test]
    async fn real_registry_batches_subjects_preserves_policy_and_bounds_failures() {
        use axum::{extract::Json, routing::post, Router};
        let app = Router::new().route("/prefix/metadata/query",post(|Json(req):Json<Value>| async move {
            assert_eq!(req["properties"],json!(PROPERTIES));
            Json(json!({"subjects":[
                {"subject":"aa01","name":p(json!("First")),"description":p(json!("one")),"decimals":p(json!(6))},
                {"subject":"bb01","name":p(json!("Second")),"description":p(json!("two"))},
                {"subject":"unrequested","name":p(json!("Ignore")),"description":p(json!(""))}
            ]}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = reqwest::Url::parse(&format!(
            "http://{}/prefix/",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let result = fetch(
            Some(&url),
            &["aa01".into(), "bb01".into(), "missing".into()],
        )
        .await
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result["aa01"]["name"], "First");
        assert_eq!(result["bb01"]["name"], "Second");
        assert!(!result["bb01"].as_object().unwrap().contains_key("decimals"));
        assert!(fetch(None, &["aa01".into()]).await.unwrap().is_empty());
        task.abort();
        let _ = task.await;
        assert_eq!(fetch(Some(&url), &["aa01".into()]).await, Err("fetch"));
    }
}
