//! Conway minimum transaction fees, including aggregate execution and reference scripts.
use crate::{
    fees::{ceil_ratio, RationalPrice},
    txsize::Parts,
};
use num_bigint::BigUint;
use num_rational::Ratio;
use pallas_codec::minicbor::Encoder;
use pallas_primitives::{conway, Fragment};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct ExecutionUnits {
    pub memory: u64,
    pub steps: u64,
}
#[derive(Debug, Deserialize)]
pub struct MinimumFeeRequest {
    pub transaction_cbor_hex: String,
    pub coefficient: u64,
    pub constant: u64,
    pub memory_price: RationalPrice,
    pub step_price: RationalPrice,
    pub reference_script_price: RationalPrice,
    pub reference_script_bytes: u64,
    pub execution_units: Vec<ExecutionUnits>,
}
#[derive(Debug, Serialize)]
pub struct MinimumFee {
    #[serde(serialize_with = "crate::fees::serialize_uint")]
    pub minimum_fee: BigUint,
    #[serde(serialize_with = "crate::fees::serialize_uint")]
    pub execution_fee: BigUint,
    #[serde(serialize_with = "crate::fees::serialize_uint")]
    pub reference_script_fee: BigUint,
    pub ledger_size: usize,
    pub transaction_cbor_hex: String,
    pub reference_stride: u64,
    pub reference_multiplier: serde_json::Value,
}
impl MinimumFeeRequest {
    pub fn calculate(&self) -> Result<MinimumFee, &'static str> {
        if self.execution_units.len() > 16 || self.reference_script_bytes > 200000 {
            return Err("invalid_input");
        }
        let original = hex::decode(&self.transaction_cbor_hex).map_err(|_| "invalid_input")?;
        let parts = Parts::decode(&original)?;
        let tx = conway::Tx::decode_fragment(&original).map_err(|_| "decode_failed")?;
        let mut memory = BigUint::from(0u8);
        let mut steps = BigUint::from(0u8);
        let mut updated = original.clone();
        if self.execution_units.is_empty() {
            if let Some(redeemers) = &tx.transaction_witness_set.redeemer {
                match redeemers {
                    conway::Redeemers::List(redeemers) => {
                        for redeemer in redeemers.iter() {
                            memory += redeemer.ex_units.mem;
                            steps += redeemer.ex_units.steps;
                        }
                    }
                    conway::Redeemers::Map(redeemers) => {
                        for (_, redeemer) in redeemers.iter() {
                            memory += redeemer.ex_units.mem;
                            steps += redeemer.ex_units.steps;
                        }
                    }
                }
            }
        } else {
            let mut encoder = Encoder::new(Vec::new());
            encoder
                .map(self.execution_units.len() as u64)
                .map_err(|_| "invalid_input")?;
            for (index, units) in self.execution_units.iter().enumerate() {
                memory += units.memory;
                steps += units.steps;
                encoder
                    .array(2)
                    .map_err(|_| "invalid_input")?
                    .u8(0)
                    .map_err(|_| "invalid_input")?
                    .u64(index as u64)
                    .map_err(|_| "invalid_input")?;
                encoder
                    .array(2)
                    .map_err(|_| "invalid_input")?
                    .u8(0)
                    .map_err(|_| "invalid_input")?
                    .array(2)
                    .map_err(|_| "invalid_input")?
                    .u64(units.memory)
                    .map_err(|_| "invalid_input")?
                    .u64(units.steps)
                    .map_err(|_| "invalid_input")?;
            }
            let redeemers = encoder.into_writer();
            let mut witnesses: Vec<_> = parts
                .witnesses
                .iter()
                .filter(|(key, _)| *key != 5)
                .map(|(key, raw)| (*key, raw.to_vec()))
                .collect();
            witnesses.push((5, redeemers));
            witnesses.sort_by_key(|(key, _)| *key);
            let mut encoder = Encoder::new(Vec::new());
            encoder
                .map(witnesses.len() as u64)
                .map_err(|_| "invalid_input")?;
            for (key, raw) in witnesses {
                encoder.u64(key).map_err(|_| "invalid_input")?;
                encoder.writer_mut().extend_from_slice(&raw);
            }
            updated = parts.assemble(&encoder.into_writer());
        }
        let execution_fee = ceil_ratio(
            self.memory_price.ratio().map_err(|_| "invalid_input")? * Ratio::from_integer(memory)
                + self.step_price.ratio().map_err(|_| "invalid_input")?
                    * Ratio::from_integer(steps),
        );
        let mut remaining = self.reference_script_bytes;
        let mut price = self
            .reference_script_price
            .ratio()
            .map_err(|_| "invalid_input")?;
        let multiplier = Ratio::new(BigUint::from(6u8), BigUint::from(5u8));
        let mut reference_total = Ratio::from_integer(BigUint::from(0u8));
        while remaining > 0 {
            let bytes = remaining.min(25600);
            reference_total += &price * Ratio::from_integer(BigUint::from(bytes));
            remaining -= bytes;
            price *= &multiplier;
        }
        let reference_script_fee = reference_total.to_integer();
        let ledger_size = Parts::decode(&updated)?.ledger_size();
        let minimum_fee = BigUint::from(self.coefficient) * BigUint::from(ledger_size)
            + BigUint::from(self.constant)
            + &execution_fee
            + &reference_script_fee;
        Ok(MinimumFee {
            minimum_fee,
            execution_fee,
            reference_script_fee,
            ledger_size,
            transaction_cbor_hex: hex::encode(updated),
            reference_stride: 25600,
            reference_multiplier: serde_json::json!({"numerator":6,"denominator":5}),
        })
    }
}
