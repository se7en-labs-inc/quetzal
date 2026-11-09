use pallas_addresses::byron::{AddrAttrs, AddrType, AddressPayload, ByronAddress, SpendingData};
use pallas_addresses::Network;
use pallas_codec::minicbor::bytes::ByteVec;
use quetzal_crypto::Bip32PublicKey;

use crate::AddressError;

/// Icarus (Byron-era, BIP-44) address.
///
/// Derives the address key at `m/.../0/ix` from `account_pub` and builds a
/// Byron PubKey address with empty attributes (no HD payload, no network tag
/// for mainnet; testnet adds a network tag).
///
/// The address is returned as a base58-encoded string.
pub fn icarus_address(
    account_pub: &Bip32PublicKey,
    ix: u32,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    // Derive address key at role=0 (external) / ix via soft derivation.
    let role_key = account_pub.derive(0)?;
    let addr_key = role_key.derive(ix)?;

    // Full 64-byte XPub (32-byte vk + 32-byte chain code) is the spending data.
    let xpub_bytes: [u8; 64] = addr_key.to_raw_bytes();

    let spending_data = SpendingData::PubKey(ByteVec::from(xpub_bytes.to_vec()));

    // Attributes: empty for mainnet; testnet uses network tag 0x01 (Magic 1097911063).
    let attributes: AddrAttrs = match network {
        Network::Mainnet => vec![].into(),
        Network::Testnet => {
            // Byron testnet magic = 1097911063 (0x41e9_6e17)
            // The network tag byte-encodes the protocol magic as big-endian u32.
            let magic: u32 = 1097911063;
            let tag_bytes = magic.to_be_bytes().to_vec();
            vec![pallas_addresses::byron::AddrAttrProperty::NetworkTag(
                ByteVec::from(tag_bytes),
            )]
            .into()
        }
        Network::Other(_) => vec![].into(),
    };

    let payload = AddressPayload::new(AddrType::PubKey, spending_data, attributes);
    let byron_addr = ByronAddress::from_decoded(payload);
    let base58 = byron_addr.to_base58();

    Ok(quetzal_primitives::Address(base58))
}
