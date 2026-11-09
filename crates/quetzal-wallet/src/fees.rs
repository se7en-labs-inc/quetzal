use crate::WalletError;
use num_bigint::BigUint;
use num_rational::Ratio;
use quetzal_primitives::Coin;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize)]
pub struct RationalPrice {
    pub numerator: u64,
    pub denominator: u64,
}
impl RationalPrice {
    pub fn ratio(&self) -> Result<Ratio<BigUint>, WalletError> {
        if self.denominator == 0 {
            return Err(WalletError::TxBuild("zero price denominator".into()));
        }
        Ok(Ratio::new(
            BigUint::from(self.numerator),
            BigUint::from(self.denominator),
        ))
    }
}
pub fn ceil_ratio(value: Ratio<BigUint>) -> BigUint {
    (value.numer() + value.denom() - BigUint::from(1u8)) / value.denom()
}
pub fn execution_fee(
    memory: u64,
    steps: u64,
    memory_price: &RationalPrice,
    step_price: &RationalPrice,
) -> Result<BigUint, WalletError> {
    Ok(ceil_ratio(
        memory_price.ratio()? * Ratio::from_integer(BigUint::from(memory))
            + step_price.ratio()? * Ratio::from_integer(BigUint::from(steps)),
    ))
}
pub(crate) fn serialize_uint<S: serde::Serializer>(
    value: &BigUint,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let number: serde_json::Number = value
        .to_string()
        .parse()
        .map_err(serde::ser::Error::custom)?;
    number.serialize(serializer)
}
pub fn estimated_witness_bytes(count: u64, bootstrap: bool) -> u64 {
    if count == 0 {
        0
    } else {
        4 + count * if bootstrap { 180 } else { 101 }
    }
}
#[derive(Debug, Deserialize)]
pub struct FeeRequest {
    pub transaction_bytes: u64,
    pub vkey_witnesses: u64,
    pub bootstrap_witnesses: u64,
    pub coefficient: u64,
    pub constant: u64,
    pub maximum_transaction_bytes: u64,
    pub execution_memory: u64,
    pub execution_steps: u64,
    pub memory_price: RationalPrice,
    pub step_price: RationalPrice,
}
#[derive(Debug, Serialize)]
pub struct FeeBreakdown {
    pub vkey_witness_bytes: u64,
    pub bootstrap_witness_bytes: u64,
    #[serde(serialize_with = "serialize_uint")]
    pub estimated_bytes: BigUint,
    #[serde(serialize_with = "serialize_uint")]
    pub linear_fee: BigUint,
    #[serde(serialize_with = "serialize_uint")]
    pub execution_fee: BigUint,
    #[serde(serialize_with = "serialize_uint")]
    pub total_fee: BigUint,
    pub within_size_limit: bool,
}
impl FeeRequest {
    pub fn calculate(&self) -> Result<FeeBreakdown, WalletError> {
        if self.vkey_witnesses > 65536
            || self.bootstrap_witnesses > 65536
            || self.maximum_transaction_bytes > u64::from(u32::MAX)
        {
            return Err(WalletError::TxBuild("fee estimation bound".into()));
        }
        let vkey_witness_bytes = estimated_witness_bytes(self.vkey_witnesses, false);
        let bootstrap_witness_bytes = estimated_witness_bytes(self.bootstrap_witnesses, true);
        let estimated_bytes = BigUint::from(self.transaction_bytes)
            + BigUint::from(vkey_witness_bytes)
            + BigUint::from(bootstrap_witness_bytes);
        let linear_fee = &estimated_bytes * BigUint::from(self.coefficient);
        let execution_fee = execution_fee(
            self.execution_memory,
            self.execution_steps,
            &self.memory_price,
            &self.step_price,
        )?;
        let total_fee = &linear_fee + &execution_fee + BigUint::from(self.constant);
        let within_size_limit = estimated_bytes <= BigUint::from(self.maximum_transaction_bytes);
        Ok(FeeBreakdown {
            vkey_witness_bytes,
            bootstrap_witness_bytes,
            estimated_bytes,
            linear_fee,
            execution_fee,
            total_fee,
            within_size_limit,
        })
    }
}

pub struct FeeParams {
    /// Per-byte coefficient in the ledger's linear fee formula.
    pub min_fee_a: u64,
    /// Constant term in lovelace.
    pub min_fee_b: u64,
}

impl FeeParams {
    pub fn estimate_fee(&self, tx_size_bytes: usize) -> Coin {
        Coin(self.min_fee_b + self.min_fee_a * tx_size_bytes as u64)
    }
}

pub const MAINNET_DEFAULT_FEE_PARAMS: FeeParams = FeeParams {
    min_fee_a: 44,
    min_fee_b: 155381,
};
