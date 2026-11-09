pub mod byron;
pub mod derivation;
pub mod hashing;
pub mod key;
pub mod metadata;
pub mod passphrase;
pub mod password;
pub mod rootkey;
pub mod signing;

pub use derivation::{
    derive_icarus_master_key, icarus_account_key, icarus_address_key, policy_account_key,
    shared_account_key, shelley_account_key, shelley_address_key, shelley_stake_key, HARDENED,
};
pub use key::{Bip32PrivateKey, Bip32PublicKey};
pub use passphrase::EncryptedKey;
pub use signing::{sign, verify};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("invalid key material")]
    InvalidKey,
    #[error("hardened public key derivation is not possible")]
    HardenedPublicDerivation,
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("argon2id KDF failed")]
    Argon2,
    #[error("AES-GCM encryption/decryption failed")]
    Aes,
    #[error("wrong passphrase")]
    InvalidPassphrase,
}
