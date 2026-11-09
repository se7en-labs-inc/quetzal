//! Ordered CIP-83 metadata transformations, preserving unrelated fields.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};

const EXHAUSTED: &str = "metadata_payload_exhausted";
fn convert(value: &Value, internal: bool) -> Result<Value, &'static str> {
    let (kind, payload) = if internal {
        let object = value
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or("invalid_input")?;
        let (kind, payload) = object.iter().next().ok_or("invalid_input")?;
        (kind.as_str(), payload)
    } else {
        let kind = value["type"].as_str().ok_or("invalid_input")?;
        let payload = match kind {
            "bytes" => &value["hex"],
            "list" => &value["items"],
            "map" => &value["entries"],
            _ => &value["value"],
        };
        (kind, payload)
    };
    match kind {
        "int" | "number" => Ok(if internal {
            json!({"type":"number","value":payload})
        } else {
            json!({"int":payload})
        }),
        "string" | "text" => Ok(if internal {
            json!({"type":"text","value":payload})
        } else {
            json!({"string":payload})
        }),
        "bytes" => Ok(if internal {
            json!({"type":"bytes","hex":payload})
        } else {
            json!({"bytes":payload})
        }),
        "list" => {
            let items = payload
                .as_array()
                .ok_or("invalid_input")?
                .iter()
                .map(|value| convert(value, internal))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(if internal {
                json!({"type":"list","items":items})
            } else {
                json!({"list":items})
            })
        }
        "map" => {
            let entries = payload
                .as_array()
                .ok_or("invalid_input")?
                .iter()
                .map(|pair| {
                    let key = convert(&pair[if internal { "k" } else { "key" }], internal)?;
                    let value = convert(&pair[if internal { "v" } else { "value" }], internal)?;
                    Ok(if internal {
                        json!({"key":key,"value":value})
                    } else {
                        json!({"k":key,"v":value})
                    })
                })
                .collect::<Result<Vec<_>, &'static str>>()?;
            Ok(if internal {
                json!({"type":"map","entries":entries})
            } else {
                json!({"map":entries})
            })
        }
        _ => Err("invalid_input"),
    }
}

pub fn detailed(
    metadata: &Value,
    password: &[u8],
    salt: Option<[u8; 8]>,
) -> Result<Value, &'static str> {
    let entries = metadata.as_object().ok_or("invalid_input")?.iter().map(|(label,value)| {
        Ok(json!({"label":label.parse::<u64>().map_err(|_|"invalid_input")?,"value":convert(value,true)?}))
    }).collect::<Result<Vec<_>,&'static str>>()?;
    let transformed = transform(&json!(entries), password, salt)?;
    let mut result = serde_json::Map::new();
    for entry in transformed["entries"].as_array().ok_or("invalid_input")? {
        result.insert(
            entry["label"].as_u64().ok_or("invalid_input")?.to_string(),
            convert(&entry["value"], false)?,
        );
    }
    Ok(Value::Object(result))
}
fn text(value: &Value, expected: &str) -> bool {
    value["type"] == "text" && value["value"] == expected
}
fn node(value: &str) -> Value {
    json!({"type":"text","value":value})
}
fn strings(value: &Value) -> Option<Vec<String>> {
    if value["type"] != "list" {
        return None;
    }
    value["items"]
        .as_array()?
        .iter()
        .map(|item| {
            (item["type"] == "text")
                .then(|| item["value"].as_str().map(str::to_owned))
                .flatten()
        })
        .collect()
}

pub fn transform(
    entries: &Value,
    password: &[u8],
    salt: Option<[u8; 8]>,
) -> Result<Value, &'static str> {
    let parsed = serde_json::from_value(entries.clone()).map_err(|_| "invalid_input")?;
    let checked = crate::metadatum::encode_entries(parsed).map_err(|_| "invalid_input")?;
    if checked["valid"] != true {
        return Err("invalid_input");
    }
    let mut rows = entries.as_array().ok_or("invalid_input")?.clone();
    rows.sort_by_key(|entry| entry["label"].as_u64());
    if rows
        .windows(2)
        .any(|pair| pair[0]["label"] == pair[1]["label"])
    {
        return Err("invalid_input");
    }
    let row = rows
        .iter_mut()
        .find(|row| row["label"] == 674)
        .ok_or(if salt.is_some() {
            "ErrIncorrectRawMetadata"
        } else {
            "ErrMissingMetadataKey"
        })?;
    let pairs = row["value"]["entries"]
        .as_array()
        .filter(|_| row["value"]["type"] == "map");
    let mut pairs = pairs.cloned().ok_or(if salt.is_some() {
        "ErrIncorrectRawMetadata"
    } else {
        "ErrMissingEncryptionMethod"
    })?;
    if let Some(salt) = salt {
        if !pairs
            .iter()
            .any(|pair| text(&pair["key"], "msg") && strings(&pair["value"]).is_some())
        {
            return Err("ErrIncorrectRawMetadata");
        }
        let mut updated = Vec::new();
        for pair in pairs {
            if text(&pair["key"], "msg") {
                let messages = strings(&pair["value"]).ok_or("ErrIncorrectRawMetadata")?;
                let plaintext = serde_json::to_vec(&messages).map_err(|_| "invalid_input")?;
                let encrypted = STANDARD.encode(quetzal_crypto::metadata::encrypt(
                    password, &salt, &plaintext,
                ));
                let chunks: Vec<_> = encrypted
                    .as_bytes()
                    .chunks(64)
                    .map(|chunk| node(std::str::from_utf8(chunk).expect("base64 ASCII")))
                    .collect();
                updated.push(json!({"key":node("msg"),"value":{"type":"list","items":chunks}}));
                updated.push(json!({"key":node("enc"),"value":node("basic")}));
            } else {
                updated.push(pair);
            }
        }
        let mut deduplicated = Vec::new();
        for pair in updated.into_iter().rev() {
            if !deduplicated.contains(&pair) {
                deduplicated.push(pair);
            }
        }
        deduplicated.reverse();
        row["value"]["entries"] = json!(deduplicated);
    } else {
        if !pairs
            .iter()
            .any(|pair| text(&pair["key"], "enc") && text(&pair["value"], "basic"))
        {
            return Err("ErrMissingEncryptionMethod");
        }
        let mut decrypted = Vec::new();
        for pair in &pairs {
            if text(&pair["key"], "msg") && pair["value"]["type"] == "list" {
                let chunks = strings(&pair["value"])
                    .ok_or("ErrMissingValidEncryptionPayload")?
                    .concat();
                if chunks.is_empty() {
                    return Err("ErrMissingValidEncryptionPayload");
                }
                let raw = STANDARD
                    .decode(chunks)
                    .map_err(|_| "ErrEncryptedPayloadWrongBase")?;
                let plaintext = quetzal_crypto::metadata::decrypt(password, &raw)?;
                let messages: Vec<String> = serde_json::from_slice(&plaintext)
                    .map_err(|_| "ErrCannotAesonDecodePayload")?;
                decrypted.push(json!({"type":"list","items":messages.iter().map(|message| node(message)).collect::<Vec<_>>()}));
            }
        }
        if decrypted.is_empty() {
            return Err("ErrMissingValidEncryptionPayload");
        }
        let mut decrypted = decrypted.into_iter();
        let mut updated = Vec::new();
        for mut pair in pairs.drain(..) {
            if text(&pair["key"], "enc") && text(&pair["value"], "basic") {
                continue;
            }
            if pair["key"]["type"] == "text" && pair["value"]["type"] != "text" {
                // Preserve the pinned implementation's exhaustion diagnostic.
                let next = decrypted.clone().next().ok_or(EXHAUSTED)?;
                if text(&pair["key"], "msg") {
                    pair["value"] = next;
                    decrypted.next();
                }
            }
            updated.push(pair);
        }
        row["value"]["entries"] = json!(updated);
    }
    Ok(json!({"entries":rows}))
}
