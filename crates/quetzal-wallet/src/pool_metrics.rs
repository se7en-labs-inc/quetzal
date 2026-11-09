//! Exact rational stake metrics, with a single final binary64 rounding.
use crate::WalletError;
use num_bigint::BigUint;
use num_rational::Ratio;
use num_traits::ToPrimitive;

pub fn percentage(numerator: u64, denominator: u64) -> Result<f64, WalletError> {
    if denominator == 0 || numerator > denominator {
        return Err(WalletError::TxBuild("invalid pool percentage".into()));
    }
    let scaled = u128::from(numerator) * 10000;
    let denominator = u128::from(denominator);
    let whole = scaled / denominator;
    let remainder = scaled % denominator;
    let rounded = whole
        + u128::from(
            remainder > denominator - remainder
                || (remainder == denominator - remainder && whole % 2 == 1),
        );
    Ok(rounded as f64 / 100.0)
}

pub fn saturation(numerator: u64, denominator: u64, target_pools: u32) -> Result<f64, WalletError> {
    if denominator == 0 {
        return Err(WalletError::TxBuild("zero pool stake denominator".into()));
    }
    Ratio::new(
        BigUint::from(numerator) * target_pools,
        BigUint::from(denominator),
    )
    .to_f64()
    .filter(|value| value.is_finite())
    .ok_or_else(|| WalletError::TxBuild("pool saturation overflow".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rounds_percentage_ties_even_and_preserves_exact_saturation() {
        assert_eq!(percentage(1, 32).unwrap(), 3.12);
        assert_eq!(percentage(3, 32).unwrap(), 9.38);
        assert_eq!(percentage(1, 3).unwrap(), 33.33);
        assert_eq!(saturation(1, 3, 3).unwrap(), 1.0);
        assert_eq!(saturation(u64::MAX, u64::MAX, 500).unwrap(), 500.0);
        assert!(percentage(2, 1).is_err());
        assert!(saturation(1, 0, 1).is_err());
    }
}
