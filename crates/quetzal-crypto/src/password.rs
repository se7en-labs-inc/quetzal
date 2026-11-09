//! Password-hash formats used by cardano-wallet's persisted credentials.
use base64::{engine::general_purpose::STANDARD, Engine};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::{derivation::pbkdf2_hmac_sha512, hashing, CryptoError};

pub fn legacy_serialization(
    key: &crate::Bip32PrivateKey,
    password: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let raw = Zeroizing::new(key.to_raw_bytes());
    let mut result = vec![0; 128];
    if password.is_empty() {
        result[..64].copy_from_slice(&raw[..64]);
    } else {
        let mut material = Zeroizing::new([0; 40]);
        // sizeof(salt) in the pinned C implementation includes the NUL.
        pbkdf2_hmac_sha512(
            password,
            b"encrypted wallet salt\0",
            15000,
            material.as_mut(),
        )?;
        cryptoxide::chacha20::ChaCha20::new(&material[..32], &material[32..])
            .process(&raw[..64], &mut result[..64]);
    }
    let public = key.public_key().to_raw_bytes();
    result[64..].copy_from_slice(&public);
    Ok(result)
}

pub fn hash_pbkdf2(password: &[u8], salt: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let length = u8::try_from(salt.len()).map_err(|_| CryptoError::InvalidPassphrase)?;
    let mut digest = Zeroizing::new([0; 64]);
    pbkdf2_hmac_sha512(password, salt, 20000, digest.as_mut())?;
    let mut result = Vec::with_capacity(1 + salt.len() + digest.len());
    result.push(length);
    result.extend_from_slice(salt);
    result.extend_from_slice(digest.as_ref());
    Ok(result)
}

pub fn decrypt_legacy(
    serialized: &[u8],
    password: &[u8],
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if serialized.len() != 128 {
        return Err(CryptoError::InvalidKey);
    }
    let mut raw = Zeroizing::new(vec![0u8; 96]);
    if password.is_empty() {
        raw[..64].copy_from_slice(&serialized[..64]);
    } else {
        let mut material = Zeroizing::new([0u8; 40]);
        pbkdf2_hmac_sha512(
            password,
            b"encrypted wallet salt\0",
            15000,
            material.as_mut(),
        )?;
        cryptoxide::chacha20::ChaCha20::new(&material[..32], &material[32..])
            .process(&serialized[..64], &mut raw[..64]);
    }
    raw[64..].copy_from_slice(&serialized[96..]);
    let key = crate::Bip32PrivateKey::from_raw_bytes(&raw)?;
    if !bool::from(key.public_key().to_raw_bytes()[..32].ct_eq(&serialized[64..96])) {
        return Err(CryptoError::InvalidKey);
    }
    Ok(raw)
}

pub fn check_pbkdf2(password: &[u8], stored: &[u8]) -> bool {
    let Some(&length) = stored.first() else {
        return false;
    };
    // Haskell extracts up to the declared salt length, then compares the whole
    // serialized result. Truncated/malformed hashes cannot pass that comparison.
    let end = (usize::from(length) + 1).min(stored.len());
    hash_pbkdf2(password, &stored[1..end]).is_ok_and(|hash| bool::from(hash.ct_eq(stored)))
}

pub fn check_scrypt(password: &[u8], stored: &[u8]) -> bool {
    let Ok(stored_text) = std::str::from_utf8(stored) else {
        return false;
    };
    let fields: Vec<_> = stored_text.split('|').collect();
    if fields.len() != 5 || fields[..3] != ["14", "8", "1"] {
        return false;
    }
    let (Ok(salt), Ok(expected)) = (STANDARD.decode(fields[3]), STANDARD.decode(fields[4])) else {
        return false;
    };
    if expected.is_empty() || expected.len() > 1024 {
        return false;
    }
    let prepared = if password.is_empty() {
        Vec::new()
    } else {
        match hashing::blake2b(password, 32) {
            Ok(hash) => hash,
            Err(_) => return false,
        }
    };
    let prepared = Zeroizing::new(prepared);
    let mut encoder = pallas_codec::minicbor::Encoder::new(Vec::new());
    if encoder.bytes(&prepared).is_err() {
        return false;
    }
    let mut derived = Zeroizing::new(vec![0; expected.len()]);
    cryptoxide::scrypt::scrypt(
        &encoder.into_writer(),
        &salt,
        &cryptoxide::scrypt::ScryptParams::new(14, 8, 1),
        &mut derived,
    );
    let actual = format!(
        "14|8|1|{}|{}",
        STANDARD.encode(salt),
        STANDARD.encode(derived.as_slice())
    );
    bool::from(actual.as_bytes().ct_eq(stored))
}

pub fn prepare_scrypt(password: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if password.is_empty() {
        Ok(Zeroizing::new(Vec::new()))
    } else {
        hashing::blake2b(password, 32).map(Zeroizing::new)
    }
}

pub fn hash_scrypt(password: &[u8], salt: &[u8]) -> Result<Vec<u8>, CryptoError> {
    let prepared = prepare_scrypt(password)?;
    let mut encoder = pallas_codec::minicbor::Encoder::new(Vec::new());
    encoder
        .bytes(&prepared)
        .map_err(|_| CryptoError::InvalidPassphrase)?;
    let mut derived = Zeroizing::new([0; 64]);
    cryptoxide::scrypt::scrypt(
        &encoder.into_writer(),
        salt,
        &cryptoxide::scrypt::ScryptParams::new(14, 8, 1),
        derived.as_mut(),
    );
    Ok(format!(
        "14|8|1|{}|{}",
        STANDARD.encode(salt),
        STANDARD.encode(derived.as_ref())
    )
    .into_bytes())
}
