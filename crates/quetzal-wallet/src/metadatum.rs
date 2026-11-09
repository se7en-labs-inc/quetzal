//! Typed ledger metadata with recursive range diagnostics.
use crate::WalletError;
use serde::Deserialize;
use serde_json::{json, Number, Value};

#[derive(Debug)]
pub enum Metadatum {
    Number { value: Number },
    Text { value: String },
    Bytes { hex: String },
    List { items: Vec<Metadatum> },
    Map { entries: Vec<Pair> },
}

// Deserialize through Value so arbitrary-precision numbers are preserved
// across the enum tag, including integers outside serde's native i64 range.
impl<'de> Deserialize<'de> for Metadatum {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        let invalid = || serde::de::Error::custom("invalid metadatum");
        match value["type"].as_str().ok_or_else(invalid)? {
            "number" => Ok(Self::Number {
                value: value["value"].as_number().ok_or_else(invalid)?.clone(),
            }),
            "text" => Ok(Self::Text {
                value: value["value"].as_str().ok_or_else(invalid)?.to_owned(),
            }),
            "bytes" => Ok(Self::Bytes {
                hex: value["hex"].as_str().ok_or_else(invalid)?.to_owned(),
            }),
            "list" => Ok(Self::List {
                items: serde_json::from_value(value["items"].clone())
                    .map_err(serde::de::Error::custom)?,
            }),
            "map" => Ok(Self::Map {
                entries: serde_json::from_value(value["entries"].clone())
                    .map_err(serde::de::Error::custom)?,
            }),
            _ => Err(invalid()),
        }
    }
}
#[derive(Debug, Deserialize)]
pub struct Pair {
    pub key: Metadatum,
    pub value: Metadatum,
}
#[derive(Debug, Deserialize)]
pub struct Entry {
    pub label: u64,
    pub value: Metadatum,
}
impl Metadatum {
    fn detailed(
        &self,
        label: u64,
        depth: usize,
        errors: &mut Vec<Value>,
    ) -> Result<Value, WalletError> {
        if depth > 8 {
            return Err(WalletError::TxBuild("metadata nesting limit".into()));
        }
        match self {
            Self::Number { value } => {
                let number = value.to_string().parse::<i128>().map_err(|_| {
                    WalletError::TxBuild("metadata number is not an integer".into())
                })?;
                if number < -(u64::MAX as i128) || number > u64::MAX as i128 {
                    errors.push(json!({"label":label,"code":"number_out_of_range","actual":value}));
                }
                Ok(json!({"int":value}))
            }
            Self::Text { value } => {
                if value.len() > 64 {
                    errors.push(json!({"label":label,"code":"text_too_long","actual":value.len()}));
                }
                Ok(json!({"string":value}))
            }
            Self::Bytes { hex: encoded } => {
                let bytes = hex::decode(encoded)
                    .map_err(|_| WalletError::TxBuild("invalid metadata hex".into()))?;
                if bytes.len() > 64 {
                    errors
                        .push(json!({"label":label,"code":"bytes_too_long","actual":bytes.len()}));
                }
                Ok(json!({"bytes":encoded}))
            }
            Self::List { items } => {
                if items.len() > 32 {
                    return Err(WalletError::TxBuild("metadata list limit".into()));
                }
                Ok(
                    json!({"list":items.iter().map(|item|item.detailed(label,depth+1,errors)).collect::<Result<Vec<_>,_>>()?}),
                )
            }
            Self::Map { entries } => {
                if entries.len() > 32 {
                    return Err(WalletError::TxBuild("metadata map limit".into()));
                }
                let mut pairs = Vec::new();
                for pair in entries {
                    pairs.push(json!({"k":pair.key.detailed(label,depth+1,errors)?,"v":pair.value.detailed(label,depth+1,errors)?}));
                }
                Ok(json!({"map":pairs}))
            }
        }
    }
}
pub fn encode_entries(entries: Vec<Entry>) -> Result<Value, WalletError> {
    if entries.len() > 32 {
        return Err(WalletError::TxBuild("metadata label limit".into()));
    }
    let entries: std::collections::BTreeMap<_, _> = entries
        .into_iter()
        .map(|entry| (entry.label, entry.value))
        .collect();
    let mut errors = Vec::new();
    let mut object = serde_json::Map::new();
    for (label, value) in entries {
        object.insert(label.to_string(), value.detailed(label, 0, &mut errors)?);
    }
    if !errors.is_empty() {
        return Ok(json!({"valid":false,"errors":errors}));
    }
    Ok(
        json!({"valid":true,"cbor_hex":hex::encode(crate::metadata::encode(&Value::Object(object))?)}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_retains_full_unsigned_and_negative_ledger_boundaries() {
        let entries=serde_json::from_str(r#"[{"label":18446744073709551615,"value":{"type":"number","value":-18446744073709551615}},{"label":0,"value":{"type":"number","value":18446744073709551615}}]"#).unwrap();
        let result = encode_entries(entries).unwrap();
        assert_eq!(result["valid"], true);
        assert_eq!(
            result["cbor_hex"],
            "a2001bffffffffffffffff1bffffffffffffffff3bfffffffffffffffe"
        );
    }
    #[test]
    fn metadata_reports_overflow_without_rounding_the_integer() {
        let entries = serde_json::from_str(
            r#"[{"label":0,"value":{"type":"number","value":18446744073709551616}}]"#,
        )
        .unwrap();
        let result = encode_entries(entries).unwrap();
        assert_eq!(result["valid"], false);
        assert_eq!(
            result["errors"][0]["actual"].to_string(),
            "18446744073709551616"
        );
    }
}
