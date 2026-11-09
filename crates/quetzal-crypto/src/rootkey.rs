//! cardano-crypto-wallet 0.2 authenticated extended-root-key envelopes.
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use pallas_codec::minicbor::{Decoder, Encoder};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::Bip32PrivateKey;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootKey(Vec<u8>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RootError {
    #[error("XPrvDecodeError")]
    Decode,
    #[error("XPrvAuthenticationFailed")]
    Authentication,
    #[error("XPrvInvalidSecretKey")]
    InvalidSecret,
    #[error("XPrvPublicKeyMismatch")]
    PublicMismatch,
    #[error("XPrvInternalError")]
    Internal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfParameters {
    pub memory_kib: u32,
    pub iterations: u32,
    pub lanes: u32,
    pub output_bytes: usize,
    pub salt_bytes: usize,
}
pub const PRODUCTION_KDF_PARAMETERS: KdfParameters = KdfParameters {
    memory_kib: 131072,
    iterations: 3,
    lanes: 1,
    output_bytes: 32,
    salt_bytes: 16,
};

struct Envelope {
    salt: [u8; 32],
    nonce: [u8; 24],
    public: [u8; 64],
    ciphertext: [u8; 64],
    tag: [u8; 16],
}

fn list(decoder: &mut Decoder<'_>, size: u64) -> Result<(), RootError> {
    if decoder.array().map_err(|_| RootError::Decode)? != Some(size) {
        return Err(RootError::Decode);
    }
    Ok(())
}

fn array<const N: usize>(decoder: &mut Decoder<'_>) -> Result<[u8; N], RootError> {
    decoder
        .bytes()
        .map_err(|_| RootError::Decode)?
        .try_into()
        .map_err(|_| RootError::Decode)
}

fn aad(public: &[u8; 64]) -> Vec<u8> {
    // Vec's minicbor writer cannot fail.
    let mut out = vec![
        0x88, 2, 1, 0x84, 0x1a, 0, 2, 0, 0, 3, 4, 0x18, 32, 1, 1, 0x18, 64, 0x58, 32,
    ];
    out.extend_from_slice(&public[..32]);
    out.extend_from_slice(&[0x58, 32]);
    out.extend_from_slice(&public[32..]);
    out
}

impl Envelope {
    fn parse(bytes: &[u8]) -> Result<Self, RootError> {
        let mut decoder = Decoder::new(bytes);
        list(&mut decoder, 5)?;
        let salt = array(&mut decoder)?;
        let nonce = array(&mut decoder)?;
        let associated = decoder.bytes().map_err(|_| RootError::Decode)?;
        let ciphertext = array(&mut decoder)?;
        let tag = array(&mut decoder)?;
        if decoder.position() != bytes.len() {
            return Err(RootError::Decode);
        }
        let mut decoder = Decoder::new(associated);
        list(&mut decoder, 8)?;
        if decoder.u64().map_err(|_| RootError::Decode)? != 2
            || decoder.u64().map_err(|_| RootError::Decode)? != 1
        {
            return Err(RootError::Decode);
        }
        list(&mut decoder, 4)?;
        for expected in [131072, 3, 4, 32, 1, 1, 64] {
            if decoder.u64().map_err(|_| RootError::Decode)? != expected {
                return Err(RootError::Decode);
            }
        }
        let key: [u8; 32] = array(&mut decoder)?;
        let chain: [u8; 32] = array(&mut decoder)?;
        if decoder.position() != associated.len() {
            return Err(RootError::Decode);
        }
        let mut public = [0; 64];
        public[..32].copy_from_slice(&key);
        public[32..].copy_from_slice(&chain);
        Ok(Self {
            salt,
            nonce,
            public,
            ciphertext,
            tag,
        })
    }

    fn serialize(&self) -> Result<Vec<u8>, RootError> {
        let mut encoder = Encoder::new(Vec::new());
        encoder.array(5).map_err(|_| RootError::Internal)?;
        for bytes in [
            self.salt.as_slice(),
            self.nonce.as_slice(),
            aad(&self.public).as_slice(),
            self.ciphertext.as_slice(),
            self.tag.as_slice(),
        ] {
            encoder.bytes(bytes).map_err(|_| RootError::Internal)?;
        }
        Ok(encoder.into_writer())
    }
}

fn wrapping_key(passphrase: &[u8], salt: &[u8; 32]) -> Result<Zeroizing<[u8; 32]>, RootError> {
    // The envelope declares parallelism 4, while the pinned C implementation
    // calls libsodium crypto_pwhash: Argon2id13, one lane and a 16-byte salt.
    let production = PRODUCTION_KDF_PARAMETERS;
    let params = Params::new(
        production.memory_kib,
        production.iterations,
        production.lanes,
        Some(production.output_bytes),
    )
    .map_err(|_| RootError::Internal)?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase, &salt[..production.salt_bytes], key.as_mut())
        .map_err(|_| RootError::Internal)?;
    Ok(key)
}

impl RootKey {
    pub fn parse(bytes: &[u8]) -> Result<Self, RootError> {
        if bytes.len() != 128 {
            Envelope::parse(bytes)?;
        }
        Ok(Self(bytes.to_vec()))
    }

    pub fn serialized(&self) -> &[u8] {
        &self.0
    }
    pub fn format(&self) -> &'static str {
        if self.0.len() == 128 {
            "v1"
        } else {
            "v2"
        }
    }

    pub fn public_key(&self) -> Result<[u8; 64], RootError> {
        if self.0.len() == 128 {
            return self.0[64..].try_into().map_err(|_| RootError::Decode);
        }
        Ok(Envelope::parse(&self.0)?.public)
    }

    pub fn create(
        master: &[u8],
        passphrase: &[u8],
        rng: &mut impl RngCore,
    ) -> Result<Self, RootError> {
        let bytes: [u8; 96] = master.try_into().map_err(|_| RootError::InvalidSecret)?;
        let key = Bip32PrivateKey::from_xprv(ed25519_bip32::XPrv::normalize_bytes_force3rd(bytes));
        Self::encrypt(&key, passphrase, rng)
    }

    pub fn encrypt(
        key: &Bip32PrivateKey,
        passphrase: &[u8],
        rng: &mut impl RngCore,
    ) -> Result<Self, RootError> {
        Self::encrypt_with_kdf(key, passphrase, rng, |password, salt, _| {
            wrapping_key(password, salt)
        })
    }

    /// Backend seam for explicit KDF failure tests. Entropy and parameters are
    /// identical to production; ordinary calls use the Argon2 implementation.
    pub fn encrypt_with_kdf(
        key: &Bip32PrivateKey,
        passphrase: &[u8],
        rng: &mut impl RngCore,
        kdf: impl FnOnce(&[u8], &[u8; 32], KdfParameters) -> Result<Zeroizing<[u8; 32]>, RootError>,
    ) -> Result<Self, RootError> {
        let mut salt = [0; 32];
        let mut nonce = [0; 24];
        rng.fill_bytes(&mut salt);
        rng.fill_bytes(&mut nonce);
        let public = key.public_key().to_raw_bytes();
        let wrapping = kdf(passphrase, &salt, PRODUCTION_KDF_PARAMETERS)?;
        let cipher = XChaCha20Poly1305::new_from_slice(wrapping.as_ref())
            .map_err(|_| RootError::Internal)?;
        let raw = Zeroizing::new(key.to_raw_bytes());
        let encrypted = cipher
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &raw[..64],
                    aad: &aad(&public),
                },
            )
            .map_err(|_| RootError::Internal)?;
        let ciphertext = encrypted[..64]
            .try_into()
            .map_err(|_| RootError::Internal)?;
        let tag = encrypted[64..]
            .try_into()
            .map_err(|_| RootError::Internal)?;
        Ok(Self(
            Envelope {
                salt,
                nonce,
                public,
                ciphertext,
                tag,
            }
            .serialize()?,
        ))
    }

    pub fn decrypt(&self, passphrase: &[u8]) -> Result<Bip32PrivateKey, RootError> {
        let envelope = Envelope::parse(&self.0)?;
        let wrapping = wrapping_key(passphrase, &envelope.salt)?;
        let cipher = XChaCha20Poly1305::new_from_slice(wrapping.as_ref())
            .map_err(|_| RootError::Internal)?;
        let mut encrypted = envelope.ciphertext.to_vec();
        encrypted.extend_from_slice(&envelope.tag);
        let secret = Zeroizing::new(
            cipher
                .decrypt(
                    XNonce::from_slice(&envelope.nonce),
                    Payload {
                        msg: &encrypted,
                        aad: &aad(&envelope.public),
                    },
                )
                .map_err(|_| RootError::Authentication)?,
        );
        let mut raw = Zeroizing::new(Vec::with_capacity(96));
        raw.extend_from_slice(&secret);
        raw.extend_from_slice(&envelope.public[32..]);
        let key = Bip32PrivateKey::from_raw_bytes(&raw).map_err(|_| RootError::InvalidSecret)?;
        if !bool::from(key.public_key().to_raw_bytes().ct_eq(&envelope.public)) {
            return Err(RootError::PublicMismatch);
        }
        Ok(key)
    }

    pub fn change_passphrase(
        &self,
        old: &[u8],
        new: &[u8],
        rng: &mut impl RngCore,
    ) -> Result<Self, RootError> {
        Self::encrypt(&self.decrypt(old)?, new, rng)
    }

    pub fn derive(
        &self,
        passphrase: &[u8],
        path: &[u32],
        rng: &mut impl RngCore,
    ) -> Result<Self, RootError> {
        if path.is_empty() {
            return Ok(self.clone());
        }
        let mut key = self.decrypt(passphrase)?;
        let mut encrypted = self.clone();
        for index in path {
            key = key.derive(*index);
            encrypted = Self::encrypt(&key, passphrase, rng)?;
        }
        Ok(encrypted)
    }
}
