use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;
use thiserror::Error;

/// Lovelace amount. Serialised as a quoted decimal string in JSON to avoid
/// JavaScript integer precision loss above 2^53.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Coin(pub u64);

impl Coin {
    pub const ZERO: Coin = Coin(0);

    #[must_use]
    pub fn checked_add(self, rhs: Coin) -> Option<Coin> {
        self.0.checked_add(rhs.0).map(Coin)
    }

    #[must_use]
    pub fn checked_sub(self, rhs: Coin) -> Option<Coin> {
        self.0.checked_sub(rhs.0).map(Coin)
    }

    pub fn as_lovelace(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Coin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u64> for Coin {
    fn from(v: u64) -> Self {
        Coin(v)
    }
}

impl From<Coin> for u64 {
    fn from(c: Coin) -> u64 {
        c.0
    }
}

impl Serialize for Coin {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for Coin {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse::<u64>().map(Coin).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Error)]
pub enum CoinError {
    #[error("coin overflow")]
    Overflow,
    #[error("coin underflow (insufficient funds)")]
    Underflow,
}
