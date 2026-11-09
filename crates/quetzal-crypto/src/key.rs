use ed25519_bip32::{DerivationScheme, XPrv, XPub};
use serde::{Deserialize, Serialize};

/// Extended private key wrapper. XPrv already zeroes its memory on Drop.
pub struct Bip32PrivateKey(XPrv);

impl Bip32PrivateKey {
    pub fn from_xprv(xprv: XPrv) -> Self {
        Bip32PrivateKey(xprv)
    }

    pub fn as_xprv(&self) -> &XPrv {
        &self.0
    }

    pub fn public_key(&self) -> Bip32PublicKey {
        Bip32PublicKey(self.0.public())
    }

    /// Derive a child key. `index >= 0x80000000` = hardened.
    pub fn derive(&self, index: u32) -> Self {
        Bip32PrivateKey(self.0.derive(DerivationScheme::V2, index))
    }

    pub fn derive_byron(&self, index: u32) -> Self {
        let raw = self.0.as_ref();
        let key = ed25519_bip32_legacy::XPrv::from_extended_and_chaincode(
            raw[..64].try_into().expect("extended secret length"),
            raw[64..].try_into().expect("chain code length"),
        );
        let child = key.derive(ed25519_bip32_legacy::DerivationScheme::V1, index);
        let raw = child.as_ref();
        Bip32PrivateKey(XPrv::from_extended_and_chaincode(
            raw[..64].try_into().expect("extended secret length"),
            raw[64..].try_into().expect("chain code length"),
        ))
    }

    /// The full 96-byte extended key (64 extended secret + 32 chain code).
    pub fn to_raw_bytes(&self) -> Vec<u8> {
        self.0.as_ref().to_vec()
    }

    /// Reconstruct from 96 raw bytes.
    pub fn from_raw_bytes(bytes: &[u8]) -> Result<Self, crate::CryptoError> {
        XPrv::from_slice_verified(bytes)
            .map(Bip32PrivateKey)
            .map_err(|_| crate::CryptoError::InvalidKey)
    }
}

/// Extended public key (32-byte point + 32-byte chain code).
/// Safe to clone and serialize.
#[derive(Clone, Debug)]
pub struct Bip32PublicKey(XPub);

impl Bip32PublicKey {
    pub fn derive_byron(&self, index: u32) -> Result<Self, crate::CryptoError> {
        ed25519_bip32_legacy::XPub::from_bytes(self.to_raw_bytes())
            .derive(ed25519_bip32_legacy::DerivationScheme::V1, index)
            .map(|child| {
                let bytes: [u8; 64] = child.as_ref().try_into().expect("extended public length");
                Bip32PublicKey(XPub::from_bytes(bytes))
            })
            .map_err(|_| crate::CryptoError::HardenedPublicDerivation)
    }
    pub fn from_xpub(xpub: XPub) -> Self {
        Bip32PublicKey(xpub)
    }

    pub fn as_xpub(&self) -> &XPub {
        &self.0
    }

    /// Soft-derive a public child key (index must be < 0x80000000).
    pub fn derive(&self, index: u32) -> Result<Self, crate::CryptoError> {
        self.0
            .derive(DerivationScheme::V2, index)
            .map(Bip32PublicKey)
            .map_err(|_| crate::CryptoError::HardenedPublicDerivation)
    }

    /// Full 64-byte extended public key (32 point + 32 chain code).
    pub fn to_raw_bytes(&self) -> [u8; 64] {
        let raw = self.0.as_ref();
        let mut out = [0u8; 64];
        out.copy_from_slice(raw);
        out
    }

    /// The 32-byte compressed Ed25519 verification key only (no chain code).
    pub fn to_vk_bytes(&self) -> [u8; 32] {
        let mut arr = [0u8; 32];
        arr.copy_from_slice(self.0.public_key_slice());
        arr
    }

    pub fn from_raw_bytes(bytes: &[u8]) -> Result<Self, crate::CryptoError> {
        XPub::from_slice(bytes)
            .map(Bip32PublicKey)
            .map_err(|_| crate::CryptoError::InvalidKey)
    }
}

impl Serialize for Bip32PublicKey {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0.as_ref()))
    }
}

impl<'de> Deserialize<'de> for Bip32PublicKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        Bip32PublicKey::from_raw_bytes(&bytes).map_err(serde::de::Error::custom)
    }
}
