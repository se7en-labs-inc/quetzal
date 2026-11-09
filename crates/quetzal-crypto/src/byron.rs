//! Legacy Byron address payload encryption.
use crate::CryptoError;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Nonce,
};
use pallas_codec::minicbor::{Decoder, Encoder};

pub fn payload_key(root_public: &[u8; 64]) -> Result<[u8; 32], CryptoError> {
    let mut key = [0u8; 32];
    crate::derivation::pbkdf2_hmac_sha512(root_public, b"address-hashing", 500, &mut key)?;
    Ok(key)
}
pub fn encrypt_path(key: &[u8; 32], account: u32, index: u32) -> Result<Vec<u8>, CryptoError> {
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .begin_array()
        .map_err(|_| CryptoError::KeyDerivation)?
        .u32(account)
        .map_err(|_| CryptoError::KeyDerivation)?
        .u32(index)
        .map_err(|_| CryptoError::KeyDerivation)?
        .end()
        .map_err(|_| CryptoError::KeyDerivation)?;
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::KeyDerivation)?;
    let encrypted = cipher
        .encrypt(
            Nonce::from_slice(b"serokellfore"),
            encoder.into_writer().as_slice(),
        )
        .map_err(|_| CryptoError::KeyDerivation)?;
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .bytes(&encrypted)
        .map_err(|_| CryptoError::KeyDerivation)?;
    Ok(encoder.into_writer())
}
pub fn decrypt_path(key: &[u8; 32], encoded: &[u8]) -> Result<Option<[u32; 2]>, CryptoError> {
    let mut decoder = Decoder::new(encoded);
    let encrypted = decoder.bytes().map_err(|_| CryptoError::InvalidKey)?;
    if decoder.position() != encoded.len() {
        return Err(CryptoError::InvalidKey);
    }
    let cipher = ChaCha20Poly1305::new_from_slice(key).map_err(|_| CryptoError::KeyDerivation)?;
    let plaintext = match cipher.decrypt(Nonce::from_slice(b"serokellfore"), encrypted) {
        Ok(plaintext) => plaintext,
        Err(_) => return Ok(None),
    };
    let mut decoder = Decoder::new(&plaintext);
    if decoder
        .array()
        .map_err(|_| CryptoError::InvalidKey)?
        .is_some()
    {
        return Err(CryptoError::InvalidKey);
    }
    let path = [
        decoder.u32().map_err(|_| CryptoError::InvalidKey)?,
        decoder.u32().map_err(|_| CryptoError::InvalidKey)?,
    ];
    if decoder.datatype().map_err(|_| CryptoError::InvalidKey)?
        != pallas_codec::minicbor::data::Type::Break
    {
        return Err(CryptoError::InvalidKey);
    }
    decoder.skip().map_err(|_| CryptoError::InvalidKey)?;
    if decoder.position() != plaintext.len() {
        return Err(CryptoError::InvalidKey);
    }
    Ok(Some(path))
}
