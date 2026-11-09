//! Ledger minimum ADA and the wallet's conservative output estimate.
use num_bigint::BigUint;
use pallas_codec::minicbor;
use pallas_primitives::{alonzo, conway};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct MinimumCoin {
    #[serde(serialize_with = "crate::fees::serialize_uint")]
    pub minimum_coin: BigUint,
    #[serde(serialize_with = "crate::fees::serialize_uint")]
    pub wallet_minimum_coin: BigUint,
    pub below_minimum: bool,
    pub output_cbor_hex: String,
    pub wallet_output_cbor_hex: String,
}
pub fn minimum_coin(encoded: &[u8], coins_per_byte: u64) -> Result<MinimumCoin, &'static str> {
    let mut decoder = minicbor::Decoder::new(encoded);
    let mut output: conway::TransactionOutput = decoder.decode().map_err(|_| "decode_failed")?;
    if decoder.position() != encoded.len() {
        return Err("decode_failed");
    }
    let actual = minicbor::to_vec(&output).map_err(|_| "decode_failed")?;
    let original_coin = match &mut output {
        conway::TransactionOutput::Legacy(output) => match &mut output.amount {
            alonzo::Value::Coin(coin) | alonzo::Value::Multiasset(coin, _) => {
                let original = *coin;
                *coin = 45_000_000_000_000_000;
                original
            }
        },
        conway::TransactionOutput::PostAlonzo(output) => match &mut output.value {
            conway::Value::Coin(coin) | conway::Value::Multiasset(coin, _) => {
                let original = *coin;
                *coin = 45_000_000_000_000_000;
                original
            }
        },
    };
    let conservative = minicbor::to_vec(output).map_err(|_| "decode_failed")?;
    let cost = BigUint::from(coins_per_byte);
    let minimum_coin = BigUint::from(160 + actual.len()) * &cost;
    let wallet_minimum_coin = BigUint::from(160 + conservative.len()) * &cost;
    let below_minimum = BigUint::from(original_coin) < minimum_coin;
    Ok(MinimumCoin {
        minimum_coin,
        wallet_minimum_coin,
        below_minimum,
        output_cbor_hex: hex::encode(actual),
        wallet_output_cbor_hex: hex::encode(conservative),
    })
}
