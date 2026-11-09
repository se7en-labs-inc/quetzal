//! Byron and Icarus address attributes and bootstrap address construction.
use crate::AddressError;
use pallas_addresses::byron::{
    AddrAttrProperty, AddrAttrs, AddrType, AddressPayload, ByronAddress, SpendingData,
};
use pallas_codec::minicbor::{self, bytes::ByteVec, Decoder, Encoder};
use quetzal_crypto::Bip32PrivateKey;

pub fn encrypt_attributes(
    key: &[u8; 32],
    account: u32,
    index: u32,
) -> Result<Vec<u8>, AddressError> {
    let encrypted = quetzal_crypto::byron::encrypt_path(key, account, index)?;
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(1)
        .map_err(|_| AddressError::Invalid)?
        .u8(1)
        .map_err(|_| AddressError::Invalid)?
        .bytes(&encrypted)
        .map_err(|_| AddressError::Invalid)?;
    Ok(encoder.into_writer())
}
pub fn decrypt_attributes(
    key: &[u8; 32],
    encoded: &[u8],
) -> Result<Option<[u32; 2]>, AddressError> {
    let mut decoder = Decoder::new(encoded);
    let length = decoder
        .map()
        .map_err(|_| AddressError::Invalid)?
        .ok_or(AddressError::Invalid)?;
    let mut path = None;
    for _ in 0..length {
        let tag = decoder.u8().map_err(|_| AddressError::Invalid)?;
        let value = decoder.bytes().map_err(|_| AddressError::Invalid)?;
        if tag == 1 && path.is_none() {
            path = Some(value);
        }
    }
    if decoder.position() != encoded.len() {
        return Err(AddressError::Invalid);
    }
    Ok(quetzal_crypto::byron::decrypt_path(
        key,
        path.ok_or(AddressError::Invalid)?,
    )?)
}
pub fn address(
    root: &Bip32PrivateKey,
    random: bool,
    account: u32,
    index: u32,
    magic: Option<u32>,
) -> Result<ByronAddress, AddressError> {
    let child = if random {
        root.derive_byron(account).derive_byron(index)
    } else {
        if account < 0x80000000 || index >= 0x80000000 {
            return Err(AddressError::Invalid);
        }
        root.derive(0x8000002c)
            .derive(0x80000717)
            .derive(account)
            .derive(0)
            .derive(index)
    };
    let mut attributes = Vec::new();
    if random {
        let key = quetzal_crypto::byron::payload_key(&root.public_key().to_raw_bytes())?;
        attributes.push(AddrAttrProperty::DerivationPath(ByteVec::from(
            quetzal_crypto::byron::encrypt_path(&key, account, index)?,
        )));
    }
    if let Some(magic) = magic {
        attributes.push(AddrAttrProperty::NetworkTag(ByteVec::from(
            minicbor::to_vec(magic).map_err(|_| AddressError::Invalid)?,
        )));
    }
    let attributes: AddrAttrs = attributes.into();
    let spending = SpendingData::PubKey(ByteVec::from(child.public_key().to_raw_bytes().to_vec()));
    Ok(ByronAddress::from_decoded(AddressPayload::new(
        AddrType::PubKey,
        spending,
        attributes,
    )))
}
pub fn inspect(encoded: &[u8], random: bool) -> Result<(), AddressError> {
    let address = ByronAddress::from_bytes(encoded)?;
    // Re-encoding recomputes CRC32 and rejects corrupted outer bytes.
    let payload = address.decode()?;
    if ByronAddress::from_decoded(payload.clone()).to_vec() != encoded {
        return Err(AddressError::Invalid);
    }
    let mut has_path = false;
    for attr in payload.attributes.iter() {
        match attr {
            AddrAttrProperty::DerivationPath(_) => {
                has_path = true;
            }
            AddrAttrProperty::NetworkTag(raw) => {
                let _: u32 = minicbor::decode(raw).map_err(|_| AddressError::Invalid)?;
            }
            _ => {}
        }
    }
    if random != has_path {
        return Err(AddressError::Invalid);
    }
    Ok(())
}

pub fn icarus_from_account(
    account: &quetzal_crypto::Bip32PublicKey,
    index: u32,
    magic: Option<u32>,
) -> Result<ByronAddress, AddressError> {
    icarus_from_account_role(account, 0, index, magic)
}

pub fn icarus_from_account_role(
    account: &quetzal_crypto::Bip32PublicKey,
    role: u32,
    index: u32,
    magic: Option<u32>,
) -> Result<ByronAddress, AddressError> {
    if role > 1 || index >= 0x80000000 {
        return Err(AddressError::Invalid);
    }
    let child = account.derive(role)?.derive(index)?;
    let mut attributes = Vec::new();
    if let Some(magic) = magic {
        attributes.push(AddrAttrProperty::NetworkTag(ByteVec::from(
            minicbor::to_vec(magic).map_err(|_| AddressError::Invalid)?,
        )));
    }
    Ok(ByronAddress::from_decoded(AddressPayload::new(
        AddrType::PubKey,
        SpendingData::PubKey(ByteVec::from(child.to_raw_bytes().to_vec())),
        attributes.into(),
    )))
}
