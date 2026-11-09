use crate::CryptoError;
use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Key, Nonce,
};
use argon2::{Algorithm, Argon2, Params, Version};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// New persisted keys use the reference-compatible V2 envelope. The untagged
/// legacy map remains readable so existing prototype records can be unlocked.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EncryptedKey {
    V2(crate::rootkey::RootKey),
    Byron(ByronEncryptedKey),
    Prototype(PrototypeEncryptedKey),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ByronEncryptedKey {
    encrypted_root: Vec<u8>,
    password_hash: Vec<u8>,
    #[serde(default)]
    scheme: LegacyScheme,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LegacyScheme {
    #[default]
    Pbkdf2,
    Scrypt,
}

impl EncryptedKey {
    pub fn encrypt_byron(key_bytes: &[u8], passphrase: &[u8]) -> Result<Self, CryptoError> {
        let key = crate::Bip32PrivateKey::from_raw_bytes(key_bytes)?;
        let mut salt = [0u8; 32];
        OsRng.fill_bytes(&mut salt);
        Self::encrypt_legacy(&key, passphrase, LegacyScheme::Pbkdf2, &salt)
    }

    pub fn encrypt_legacy(
        key: &crate::Bip32PrivateKey,
        passphrase: &[u8],
        scheme: LegacyScheme,
        salt: &[u8],
    ) -> Result<Self, CryptoError> {
        let prepared = match scheme {
            LegacyScheme::Pbkdf2 => Zeroizing::new(passphrase.to_vec()),
            LegacyScheme::Scrypt => crate::password::prepare_scrypt(passphrase)?,
        };
        let password_hash = match scheme {
            LegacyScheme::Pbkdf2 => crate::password::hash_pbkdf2(passphrase, salt)?,
            LegacyScheme::Scrypt => crate::password::hash_scrypt(passphrase, salt)?,
        };
        Ok(Self::Byron(ByronEncryptedKey {
            encrypted_root: crate::password::legacy_serialization(key, &prepared)?,
            password_hash,
            scheme,
        }))
    }
    pub fn encrypt(key_bytes: &[u8], passphrase: &[u8]) -> Result<Self, CryptoError> {
        let key = crate::Bip32PrivateKey::from_raw_bytes(key_bytes)?;
        crate::rootkey::RootKey::encrypt(&key, passphrase, &mut OsRng)
            .map(Self::V2)
            .map_err(|_| CryptoError::InvalidKey)
    }

    pub fn decrypt(&self, passphrase: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        match self {
            Self::V2(root) => root
                .decrypt(passphrase)
                .map(|key| Zeroizing::new(key.to_raw_bytes()))
                .map_err(|error| {
                    if error == crate::rootkey::RootError::Authentication {
                        CryptoError::InvalidPassphrase
                    } else {
                        CryptoError::InvalidKey
                    }
                }),
            Self::Prototype(key) => key.decrypt(passphrase),
            Self::Byron(key) => {
                let accepted = match key.scheme {
                    LegacyScheme::Pbkdf2 => {
                        crate::password::check_pbkdf2(passphrase, &key.password_hash)
                    }
                    LegacyScheme::Scrypt => {
                        crate::password::check_scrypt(passphrase, &key.password_hash)
                    }
                };
                if !accepted {
                    return Err(CryptoError::InvalidPassphrase);
                }
                let prepared = match key.scheme {
                    LegacyScheme::Pbkdf2 => Zeroizing::new(passphrase.to_vec()),
                    LegacyScheme::Scrypt => crate::password::prepare_scrypt(passphrase)?,
                };
                crate::password::decrypt_legacy(&key.encrypted_root, &prepared)
            }
        }
    }

    pub fn envelope(&self) -> Option<&[u8]> {
        match self {
            Self::V2(root) => Some(root.serialized()),
            Self::Prototype(_) | Self::Byron(_) => None,
        }
    }

    pub fn serialized_root(&self) -> Option<&[u8]> {
        match self {
            Self::V2(root) => Some(root.serialized()),
            Self::Byron(key) => Some(&key.encrypted_root),
            Self::Prototype(_) => None,
        }
    }

    pub fn credential_scheme(&self) -> &'static str {
        match self {
            Self::V2(_) | Self::Prototype(_) => "argon2id",
            Self::Byron(key) => match key.scheme {
                LegacyScheme::Pbkdf2 => "pbkdf2",
                LegacyScheme::Scrypt => "scrypt",
            },
        }
    }

    pub fn public_key(&self) -> Result<[u8; 64], CryptoError> {
        match self {
            Self::V2(root) => root.public_key().map_err(|_| CryptoError::InvalidKey),
            Self::Byron(key) => key
                .encrypted_root
                .get(64..)
                .ok_or(CryptoError::InvalidKey)?
                .try_into()
                .map_err(|_| CryptoError::InvalidKey),
            Self::Prototype(_) => Err(CryptoError::InvalidKey),
        }
    }
}

/// Argon2id parameters: 64 MiB memory, 3 iterations, 4 lanes.
const ARGON2_M_COST: u32 = 65536;
const ARGON2_T_COST: u32 = 3;
const ARGON2_P_COST: u32 = 4;
const ARGON2_OUTPUT_LEN: usize = 32;

/// An extended private key (96 bytes) encrypted with argon2id + AES-256-GCM.
/// Stored in sled. Serialised as CBOR.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrototypeEncryptedKey {
    /// Random 32-byte argon2 salt, freshly generated per encrypt() call.
    pub argon2_salt: [u8; 32],
    /// Random 12-byte AES-GCM nonce (96-bit), freshly generated per encrypt() call.
    pub aes_nonce: [u8; 12],
    /// AES-256-GCM ciphertext of the 96-byte XPrv extended key, with 16-byte GCM tag appended.
    pub ciphertext: Vec<u8>,
}

impl PrototypeEncryptedKey {
    /// Encrypt `key_bytes` (96-byte XPrv) with `passphrase`.
    ///
    /// Fresh random salt and nonce are generated on every call via OsRng.
    /// The 32-byte derived AES key is zeroized immediately after encryption.
    pub fn encrypt(key_bytes: &[u8], passphrase: &[u8]) -> Result<Self, CryptoError> {
        // Generate fresh random salt and nonce
        let mut argon2_salt = [0u8; 32];
        let mut aes_nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut argon2_salt);
        OsRng.fill_bytes(&mut aes_nonce_bytes);

        // Derive 32-byte AES key from passphrase using argon2id
        let mut aes_key_bytes = Zeroizing::new([0u8; ARGON2_OUTPUT_LEN]);
        let params = Params::new(
            ARGON2_M_COST,
            ARGON2_T_COST,
            ARGON2_P_COST,
            Some(ARGON2_OUTPUT_LEN),
        )
        .map_err(|_| CryptoError::Argon2)?;
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase, &argon2_salt, aes_key_bytes.as_mut())
            .map_err(|_| CryptoError::Argon2)?;

        // Encrypt with AES-256-GCM
        let key = Key::<Aes256Gcm>::from_slice(aes_key_bytes.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&aes_nonce_bytes);
        let ciphertext = cipher
            .encrypt(nonce, key_bytes)
            .map_err(|_| CryptoError::Aes)?;
        // aes_key_bytes is zeroized when dropped here

        Ok(PrototypeEncryptedKey {
            argon2_salt,
            aes_nonce: aes_nonce_bytes,
            ciphertext,
        })
    }

    /// Decrypt with `passphrase`. Returns the 96-byte XPrv in a Zeroizing buffer.
    ///
    /// Returns `Err(CryptoError::InvalidPassphrase)` if the passphrase is wrong
    /// (GCM tag verification fails). Never panics.
    pub fn decrypt(&self, passphrase: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
        // Derive AES key
        let mut aes_key_bytes = Zeroizing::new([0u8; ARGON2_OUTPUT_LEN]);
        let params = Params::new(
            ARGON2_M_COST,
            ARGON2_T_COST,
            ARGON2_P_COST,
            Some(ARGON2_OUTPUT_LEN),
        )
        .map_err(|_| CryptoError::Argon2)?;
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(passphrase, &self.argon2_salt, aes_key_bytes.as_mut())
            .map_err(|_| CryptoError::Argon2)?;

        let key = Key::<Aes256Gcm>::from_slice(aes_key_bytes.as_ref());
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(&self.aes_nonce);
        let plaintext = cipher
            .decrypt(nonce, self.ciphertext.as_slice())
            .map_err(|_| CryptoError::InvalidPassphrase)?;
        // aes_key_bytes zeroized on drop

        Ok(Zeroizing::new(plaintext))
    }
}
