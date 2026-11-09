//! Cardano detailed-schema transaction metadata encoding.
use pallas_codec::minicbor::Encoder;
use serde_json::Value;

use crate::WalletError;

fn invalid() -> WalletError {
    WalletError::TxBuild("Invalid transaction metadata".into())
}

pub fn decode_ledger(metadata: &pallas_primitives::alonzo::Metadata) -> Result<Value, WalletError> {
    use ciborium::value::Value as Cbor;
    fn detailed(value: &Cbor, depth: usize) -> Result<Value, WalletError> {
        if depth > 64 {
            return Err(invalid());
        }
        match value {
            Cbor::Integer(number) => {
                let number: Value = serde_json::from_str(&i128::from(*number).to_string())
                    .map_err(|_| invalid())?;
                Ok(serde_json::json!({"int":number}))
            }
            Cbor::Bytes(bytes) => Ok(serde_json::json!({"bytes":hex::encode(bytes)})),
            Cbor::Text(text) => Ok(serde_json::json!({"string":text})),
            Cbor::Array(items) => Ok(
                serde_json::json!({"list":items.iter().map(|value|detailed(value,depth+1)).collect::<Result<Vec<_>,_>>()?}),
            ),
            Cbor::Map(entries) => {
                let mut pairs = Vec::new();
                for (key, value) in entries {
                    pairs.push(serde_json::json!({"k":detailed(key,depth+1)?,"v":detailed(value,depth+1)?}));
                }
                Ok(serde_json::json!({"map":pairs}))
            }
            _ => Err(invalid()),
        }
    }
    let encoded = pallas_codec::minicbor::to_vec(metadata).map_err(|_| invalid())?;
    let value: Cbor = ciborium::de::from_reader(encoded.as_slice()).map_err(|_| invalid())?;
    let Cbor::Map(entries) = value else {
        return Err(invalid());
    };
    let mut metadata = serde_json::Map::new();
    for (key, value) in entries {
        let Cbor::Integer(key) = key else {
            return Err(invalid());
        };
        let key = u64::try_from(i128::from(key)).map_err(|_| invalid())?;
        metadata.insert(key.to_string(), detailed(&value, 0)?);
    }
    Ok(Value::Object(metadata))
}

pub fn encode(metadata: &Value) -> Result<Vec<u8>, WalletError> {
    let labels = metadata.as_object().ok_or_else(invalid)?;
    let mut labels = labels
        .iter()
        .map(|(label, value)| {
            label
                .parse::<u64>()
                .map(|label| (label, value))
                .map_err(|_| invalid())
        })
        .collect::<Result<Vec<_>, _>>()?;
    labels.sort_by_key(|(label, _)| *label);
    if labels.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(invalid());
    }
    let mut encoder = Encoder::new(Vec::new());
    encoder.map(labels.len() as u64).map_err(|_| invalid())?;
    for (label, value) in labels {
        encoder.u64(label).map_err(|_| invalid())?;
        encode_value(value, &mut encoder, 0)?;
    }
    Ok(encoder.into_writer())
}

fn encode_value(
    value: &Value,
    encoder: &mut Encoder<Vec<u8>>,
    depth: usize,
) -> Result<(), WalletError> {
    if depth > 64 {
        return Err(invalid());
    }
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(invalid)?;
    let (kind, value) = object.iter().next().ok_or_else(invalid)?;
    match kind.as_str() {
        "int" => {
            let number = value
                .as_number()
                .ok_or_else(invalid)?
                .to_string()
                .parse::<i128>()
                .map_err(|_| invalid())?;
            if number < -(u64::MAX as i128) || number > u64::MAX as i128 {
                return Err(invalid());
            }
            encoder
                .int(pallas_codec::minicbor::data::Int::try_from(number).map_err(|_| invalid())?)
                .map_err(|_| invalid())?;
        }
        "string" => {
            let string = value
                .as_str()
                .filter(|s| s.len() <= 64)
                .ok_or_else(invalid)?;
            encoder.str(string).map_err(|_| invalid())?;
        }
        "bytes" => {
            let bytes = hex::decode(value.as_str().ok_or_else(invalid)?).map_err(|_| invalid())?;
            if bytes.len() > 64 {
                return Err(invalid());
            }
            encoder.bytes(&bytes).map_err(|_| invalid())?;
        }
        "list" => {
            let values = value.as_array().ok_or_else(invalid)?;
            encoder.array(values.len() as u64).map_err(|_| invalid())?;
            for value in values {
                encode_value(value, encoder, depth + 1)?;
            }
        }
        "map" => {
            let values = value.as_array().ok_or_else(invalid)?;
            encoder.map(values.len() as u64).map_err(|_| invalid())?;
            for pair in values {
                let pair = pair
                    .as_object()
                    .filter(|pair| pair.len() == 2)
                    .ok_or_else(invalid)?;
                encode_value(pair.get("k").ok_or_else(invalid)?, encoder, depth + 1)?;
                encode_value(pair.get("v").ok_or_else(invalid)?, encoder, depth + 1)?;
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}
