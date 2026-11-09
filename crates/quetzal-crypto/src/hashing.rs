use blake2::digest::{Update, VariableOutput};
use sha2::{Digest, Sha256, Sha512};

pub fn sha256(message: &[u8]) -> [u8; 32] {
    Sha256::digest(message).into()
}

pub fn sha512(message: &[u8]) -> [u8; 64] {
    Sha512::digest(message).into()
}

pub fn blake2b(message: &[u8], output_bytes: usize) -> Result<Vec<u8>, crate::CryptoError> {
    if !(1..=64).contains(&output_bytes) {
        return Err(crate::CryptoError::InvalidKey);
    }
    let mut hash =
        blake2::Blake2bVar::new(output_bytes).map_err(|_| crate::CryptoError::InvalidKey)?;
    hash.update(message);
    let mut result = vec![0; output_bytes];
    hash.finalize_variable(&mut result)
        .map_err(|_| crate::CryptoError::InvalidKey)?;
    Ok(result)
}
