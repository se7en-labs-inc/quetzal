use crate::TxHash;
use sled::IVec;

/// 8-byte big-endian encoding of a slot number.
/// Big-endian ensures lexicographic order == numeric order in sled.
pub fn slot_key(slot: u64) -> IVec {
    IVec::from(slot.to_be_bytes().as_ref())
}

/// Decode a slot from an 8-byte big-endian key prefix.
pub fn slot_from_key(bytes: &[u8]) -> Option<u64> {
    bytes.get(..8)?.try_into().ok().map(u64::from_be_bytes)
}

/// Composite key for `utxos:{wid}` tree: tx_hash[32] ++ out_idx[4 BE]
pub fn utxo_key(tx_hash: &TxHash, out_idx: u32) -> IVec {
    let mut buf = Vec::with_capacity(36);
    buf.extend_from_slice(&tx_hash.0);
    buf.extend_from_slice(&out_idx.to_be_bytes());
    IVec::from(buf)
}

/// Composite key for `txs:{wid}` tree: slot[8 BE] ++ tx_hash[32]
pub fn tx_history_key(slot: u64, tx_hash: &TxHash) -> IVec {
    let mut buf = Vec::with_capacity(40);
    buf.extend_from_slice(&slot.to_be_bytes());
    buf.extend_from_slice(&tx_hash.0);
    IVec::from(buf)
}

/// Encode a value as CBOR using ciborium.
pub fn cbor_encode<T: serde::Serialize>(
    value: &T,
) -> Result<Vec<u8>, ciborium::ser::Error<std::io::Error>> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(value, &mut buf)?;
    Ok(buf)
}

/// Decode a value from CBOR using ciborium.
pub fn cbor_decode<T: for<'de> serde::Deserialize<'de>>(
    bytes: &[u8],
) -> Result<T, ciborium::de::Error<std::io::Error>> {
    ciborium::de::from_reader(bytes)
}
