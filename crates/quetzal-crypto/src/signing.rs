use crate::Bip32PrivateKey;

pub fn sign_seed(seed: &[u8; 32], message: &[u8]) -> ([u8; 32], [u8; 64]) {
    use pallas_crypto::key::ed25519::SecretKey;
    let secret = SecretKey::from(*seed);
    let mut signature = [0; 64];
    signature.copy_from_slice(secret.sign(message).as_ref());
    (secret.public_key().into(), signature)
}

/// Sign `message` with the private key, returning a 64-byte Ed25519 signature.
pub fn sign(key: &Bip32PrivateKey, message: &[u8]) -> [u8; 64] {
    let sig = key.as_xprv().sign::<Vec<u8>>(message);
    let mut arr = [0u8; 64];
    arr.copy_from_slice(sig.to_bytes());
    arr
}

/// Verify an Ed25519 signature against a 32-byte public key.
pub fn verify(public_key_bytes: &[u8; 32], message: &[u8], signature_bytes: &[u8; 64]) -> bool {
    use ed25519_bip32::{Signature, XPub};
    // Build an XPub with a dummy chain code (only the public key portion is used for verify)
    let mut raw = [0u8; 64];
    raw[..32].copy_from_slice(public_key_bytes);
    let xpub = XPub::from_bytes(raw);
    let sig = Signature::<Vec<u8>>::from_slice(signature_bytes).expect("64 bytes");
    xpub.verify(message, &sig)
}

/// Legacy cardano-crypto verifier. Donna accepts S modulo the group order,
/// unlike the canonical-scalar verifier used for ordinary Ed25519 signatures.
pub fn verify_extended(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    if signature[63] & 0xe0 != 0 {
        return false;
    }
    let mut wide = [0; 64];
    wide[..32].copy_from_slice(&signature[32..]);
    let scalar = cryptoxide::curve25519::Scalar::reduce_from_wide_bytes(&wide);
    let mut normalized = *signature;
    normalized[32..].copy_from_slice(&scalar.to_bytes());
    verify(public_key, message, &normalized)
}
