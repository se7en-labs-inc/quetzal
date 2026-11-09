//! CIP-83 salted AES-256-CBC with PBKDF2-HMAC-SHA256.
use aes_gcm::aes::{
    cipher::{BlockDecrypt, BlockEncrypt, KeyInit},
    Aes256,
};
use zeroize::Zeroizing;

fn material(password: &[u8], salt: &[u8; 8]) -> Zeroizing<[u8; 48]> {
    let mut derived = Zeroizing::new([0; 48]);
    let mut hmac = cryptoxide::hmac::Hmac::new(cryptoxide::sha2::Sha256::new(), password);
    cryptoxide::pbkdf2::pbkdf2(&mut hmac, salt, 10_000, derived.as_mut());
    derived
}

pub fn encrypt(password: &[u8], salt: &[u8; 8], message: &[u8]) -> Vec<u8> {
    let key = material(password, salt);
    let cipher = Aes256::new_from_slice(&key[..32]).expect("fixed AES key length");
    let mut previous = [0; 16];
    previous.copy_from_slice(&key[32..]);
    let padding = 16 - message.len() % 16;
    let mut padded = Zeroizing::new(message.to_vec());
    padded.resize(message.len() + padding, padding as u8);
    let mut result = b"Salted__".to_vec();
    result.extend_from_slice(salt);
    for chunk in padded.as_chunks_mut::<16>().0 {
        for (byte, prior) in chunk.iter_mut().zip(previous) {
            *byte ^= prior;
        }
        cipher.encrypt_block(chunk.into());
        previous.copy_from_slice(chunk);
        result.extend_from_slice(chunk);
    }
    result
}

pub fn decrypt(password: &[u8], message: &[u8]) -> Result<Zeroizing<Vec<u8>>, &'static str> {
    if message.len() < 32 || !message.starts_with(b"Salted__") {
        return Err("ErrMissingSalt");
    }
    if !(message.len() - 16).is_multiple_of(16) {
        return Err("ErrCannotDecryptPayload WrongPayloadSize");
    }
    let salt = message[8..16].try_into().expect("checked salt length");
    let key = material(password, salt);
    let cipher = Aes256::new_from_slice(&key[..32]).expect("fixed AES key length");
    let mut previous = [0; 16];
    previous.copy_from_slice(&key[32..]);
    let mut result = Zeroizing::new(message[16..].to_vec());
    for chunk in result.as_chunks_mut::<16>().0 {
        let mut encrypted = [0; 16];
        encrypted.copy_from_slice(chunk);
        cipher.decrypt_block(chunk.into());
        for (byte, prior) in chunk.iter_mut().zip(previous) {
            *byte ^= prior;
        }
        previous = encrypted;
    }
    let padding = usize::from(
        *result
            .last()
            .ok_or("ErrCannotDecryptPayload EmptyPayload")?,
    );
    if padding == 0
        || padding > 16
        || !result[result.len() - padding..]
            .iter()
            .all(|byte| usize::from(*byte) == padding)
    {
        return Err("ErrCannotDecryptPayload EmptyPayload");
    }
    let unpadded_length = result.len() - padding;
    result.truncate(unpadded_length);
    Ok(result)
}
