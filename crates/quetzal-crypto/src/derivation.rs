use crate::{Bip32PrivateKey, CryptoError};
use ed25519_bip32::XPrv;
use hmac::Hmac;
use hmac::Mac;
use sha2::Sha512;
use zeroize::Zeroizing;

/// Hardened derivation offset per BIP-32 / CIP-1852.
pub const HARDENED: u32 = 0x8000_0000;

/// CIP-1852 purpose index (1852' = 0x8000073C)
pub const PURPOSE_1852: u32 = HARDENED | 1852;
/// BIP-44 purpose index (44' = 0x8000002C)
pub const PURPOSE_44: u32 = HARDENED | 44;
/// Cardano coin type index (1815' = 0x80000717)
pub const COIN_TYPE_ADA: u32 = HARDENED | 1815;

/// Byron random roots use a CBOR/hash sandwich and rejection-sampled root key.
pub fn derive_byron_master_key(entropy: &[u8]) -> Result<Bip32PrivateKey, CryptoError> {
    let mut encoder = pallas_codec::minicbor::Encoder::new(Vec::new());
    encoder
        .bytes(entropy)
        .map_err(|_| CryptoError::KeyDerivation)?;
    let digest = crate::hashing::blake2b(&encoder.into_writer(), 32)?;
    let mut encoder = pallas_codec::minicbor::Encoder::new(Vec::new());
    encoder
        .bytes(&digest)
        .map_err(|_| CryptoError::KeyDerivation)?;
    let seed = Zeroizing::new(encoder.into_writer());
    for index in 1..=1000 {
        let mut mac =
            <Hmac<Sha512> as Mac>::new_from_slice(&seed).map_err(|_| CryptoError::KeyDerivation)?;
        mac.update(format!("Root Seed Chain {index}").as_bytes());
        let output = Zeroizing::new(mac.finalize().into_bytes().to_vec());
        let left = output[..32]
            .try_into()
            .map_err(|_| CryptoError::KeyDerivation)?;
        let right = output[32..]
            .try_into()
            .map_err(|_| CryptoError::KeyDerivation)?;
        if let Ok(key) = XPrv::from_nonextended_noforce(left, right) {
            return Ok(Bip32PrivateKey::from_xprv(key));
        }
    }
    Err(CryptoError::KeyDerivation)
}

pub fn raw_mnemonic_seed(
    sentence: &str,
    password: &str,
) -> Result<Zeroizing<[u8; 64]>, CryptoError> {
    let mut seed = Zeroizing::new([0; 64]);
    let salt = Zeroizing::new(format!("mnemonic{password}"));
    pbkdf2_hmac_sha512(sentence.as_bytes(), salt.as_bytes(), 2048, seed.as_mut())?;
    Ok(seed)
}

pub fn derive_ledger_master_key(sentence: &str) -> Result<Bip32PrivateKey, CryptoError> {
    use sha2::Sha256;
    let seed = raw_mnemonic_seed(sentence, "")?;
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(b"ed25519 seed")
        .map_err(|_| CryptoError::KeyDerivation)?;
    mac.update(&[1]);
    mac.update(seed.as_ref());
    let chain = mac.finalize().into_bytes();
    let mut input = Zeroizing::new(seed.to_vec());
    for _ in 0..1000 {
        let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(b"ed25519 seed")
            .map_err(|_| CryptoError::KeyDerivation)?;
        mac.update(&input);
        let output = mac.finalize().into_bytes();
        if output[31] & 0x20 != 0 {
            *input = output.to_vec();
            continue;
        }
        let mut raw = Zeroizing::new([0; 96]);
        raw[..64].copy_from_slice(&output);
        raw[64..].copy_from_slice(&chain);
        return Ok(Bip32PrivateKey::from_xprv(XPrv::normalize_bytes_ed25519(
            *raw,
        )));
    }
    Err(CryptoError::KeyDerivation)
}

pub fn valid_wallet_path(style: &str, path: &[u32]) -> bool {
    if path.is_empty() {
        return true;
    }
    if style == "Byron" {
        return path.len() == 2;
    }
    let purpose = match style {
        "Shelley" => 1852,
        "Shared" => 1854,
        "Icarus" => 44,
        _ => return false,
    } | HARDENED;
    let prefix =
        path.len() >= 3 && path[0] == purpose && path[1] == COIN_TYPE_ADA && path[2] >= HARDENED;
    prefix && (path.len() == 3 || (path.len() == 5 && path[3] <= 2 && path[4] < HARDENED))
}

/// Icarus master key derivation from mnemonic entropy.
///
/// This is the PBKDF2-SHA512 root key derivation used for Icarus/Shelley wallets
/// as specified in CIP-0003 (Icarus master key generation).
///
/// IMPORTANT: This PBKDF2 is part of the HD derivation algorithm (not storage encryption).
/// It derives the root HD key from the mnemonic. The storage encryption uses argon2id separately.
pub fn derive_icarus_master_key(
    entropy: &[u8],
    passphrase: &[u8],
) -> Result<Bip32PrivateKey, CryptoError> {
    // PBKDF2 with 4096 rounds, 96 bytes output
    let mut output = Zeroizing::new([0u8; 96]);
    pbkdf2_hmac_sha512(passphrase, entropy, 4096, output.as_mut())?;

    // Tweak bits per Ed25519 extended key spec
    let mut key_material = output.to_vec();
    // Bit-tweaking per IOHK Icarus derivation spec
    key_material[0] &= 0b1111_1000;
    key_material[31] &= 0b0001_1111;
    key_material[31] |= 0b0100_0000;

    let xprv = XPrv::from_slice_verified(&key_material).map_err(|_| CryptoError::KeyDerivation)?;
    Ok(Bip32PrivateKey::from_xprv(xprv))
}

/// Minimal PBKDF2-HMAC-SHA512 implementation.
/// Used ONLY for Icarus master key derivation (HD algorithm), not for storage encryption.
pub(crate) fn pbkdf2_hmac_sha512(
    password: &[u8],
    salt: &[u8],
    rounds: u32,
    out: &mut [u8],
) -> Result<(), CryptoError> {
    let block_len = 64usize; // SHA-512 output size
    let n_blocks = out.len().div_ceil(block_len);

    for block_idx in 1..=n_blocks {
        let mut u = {
            let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(password)
                .map_err(|_| CryptoError::KeyDerivation)?;
            mac.update(salt);
            mac.update(&(block_idx as u32).to_be_bytes());
            mac.finalize().into_bytes()
        };
        let mut xor = u.to_vec();

        for _ in 1..rounds {
            let mut mac = <Hmac<Sha512> as Mac>::new_from_slice(password)
                .map_err(|_| CryptoError::KeyDerivation)?;
            mac.update(&u);
            u = mac.finalize().into_bytes();
            for (a, b) in xor.iter_mut().zip(u.iter()) {
                *a ^= b;
            }
        }

        let start = (block_idx - 1) * block_len;
        let end = (start + block_len).min(out.len());
        out[start..end].copy_from_slice(&xor[..end - start]);
    }
    Ok(())
}

/// Derive a child private key. `index >= HARDENED` means hardened derivation.
pub fn derive_child(parent: &Bip32PrivateKey, index: u32) -> Bip32PrivateKey {
    parent.derive(index)
}

/// CIP-1852 Shelley account key: m/1852'/1815'/account_ix'
pub fn shelley_account_key(root: &Bip32PrivateKey, account: u32) -> Bip32PrivateKey {
    let purpose = derive_child(root, PURPOSE_1852);
    let coin_type = derive_child(&purpose, COIN_TYPE_ADA);
    derive_child(&coin_type, HARDENED | account)
}

/// CIP-1852 address key: m/1852'/1815'/account'/role/ix
pub fn shelley_address_key(account: &Bip32PrivateKey, role: u32, ix: u32) -> Bip32PrivateKey {
    let role_key = derive_child(account, role);
    derive_child(&role_key, ix)
}

/// CIP-1852 stake key: m/1852'/1815'/account'/2/0
pub fn shelley_stake_key(account: &Bip32PrivateKey) -> Bip32PrivateKey {
    shelley_address_key(account, 2, 0)
}

/// BIP-44 Icarus account: m/44'/1815'/account_ix'
pub fn icarus_account_key(root: &Bip32PrivateKey, account: u32) -> Bip32PrivateKey {
    let purpose = derive_child(root, PURPOSE_44);
    let coin_type = derive_child(&purpose, COIN_TYPE_ADA);
    derive_child(&coin_type, HARDENED | account)
}

/// BIP-44 Icarus address key: m/44'/1815'/account'/0/ix
pub fn icarus_address_key(account: &Bip32PrivateKey, ix: u32) -> Bip32PrivateKey {
    let change = derive_child(account, 0);
    derive_child(&change, ix)
}

/// CIP-1855 policy purpose index (1855' = 0x8000073F)
pub const PURPOSE_1855: u32 = HARDENED | 1855;

/// CIP-1855 policy account key: m/1855'/1815'/0'
pub fn policy_account_key(root: &Bip32PrivateKey) -> Bip32PrivateKey {
    let purpose = derive_child(root, PURPOSE_1855);
    let coin_type = derive_child(&purpose, COIN_TYPE_ADA);
    derive_child(&coin_type, HARDENED)
}

/// CIP-1854 multi-signature purpose index (1854' = 0x8000073E)
pub const PURPOSE_1854: u32 = HARDENED | 1854;

/// CIP-1854 shared wallet account key: m/1854'/1815'/account_ix'
pub fn shared_account_key(root: &Bip32PrivateKey, account: u32) -> Bip32PrivateKey {
    let purpose = derive_child(root, PURPOSE_1854);
    let coin_type = derive_child(&purpose, COIN_TYPE_ADA);
    derive_child(&coin_type, HARDENED | account)
}
