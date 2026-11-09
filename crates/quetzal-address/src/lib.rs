pub mod byron;
pub mod codec;
pub mod discovery;
pub mod icarus;
pub mod script;
pub mod shelley;

pub use discovery::{AddressDiscovery, GAP_LIMIT};
pub use icarus::icarus_address;
pub use shelley::{base_address, enterprise_address, script_enterprise_address, stake_address};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AddressError {
    #[error("pallas address error: {0}")]
    Pallas(#[from] pallas_addresses::Error),
    #[error("crypto error: {0}")]
    Crypto(#[from] quetzal_crypto::CryptoError),
    #[error("invalid address")]
    Invalid,
}
