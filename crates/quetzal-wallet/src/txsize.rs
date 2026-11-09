//! Ledger transaction sizes and wallet key-witness estimates.
use crate::{fees::estimated_witness_bytes, WalletError};
use pallas_codec::minicbor::{Decoder, Encoder};
use pallas_primitives::{conway, Fragment};
use quetzal_primitives::{TxHash, UtxoEntry, UtxoId};
use serde::Serialize;
use std::collections::HashSet;

pub(crate) struct Parts<'a> {
    pub body: &'a [u8],
    pub witnesses: Vec<(u64, &'a [u8])>,
    pub witness_bytes: &'a [u8],
    pub success: &'a [u8],
    pub auxiliary: &'a [u8],
}

/// Ledger identity hashes the original body encoding, including array framing.
pub fn body_hash(bytes: &[u8]) -> Result<[u8; 32], &'static str> {
    Ok(*pallas_crypto::hash::Hasher::<256>::hash(
        Parts::decode(bytes)?.body,
    ))
}

/// Replace witnesses without re-encoding any signed transaction body bytes.
pub fn replace_witnesses(
    bytes: &[u8],
    witnesses: &conway::WitnessSet,
) -> Result<Vec<u8>, &'static str> {
    let parts = Parts::decode(bytes)?;
    let encoded = pallas_codec::minicbor::to_vec(witnesses).map_err(|_| "invalid_input")?;
    Ok(parts.assemble(&encoded))
}
impl<'a> Parts<'a> {
    pub fn decode(bytes: &'a [u8]) -> Result<Self, &'static str> {
        conway::Tx::decode_fragment(bytes).map_err(|_| "decode_failed")?;
        let mut decoder = Decoder::new(bytes);
        if decoder.array().map_err(|_| "decode_failed")? != Some(4) {
            return Err("decode_failed");
        }
        let start = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        let body = &bytes[start..decoder.position()];
        let start = decoder.position();
        let length = decoder.map().map_err(|_| "decode_failed")?;
        let mut witnesses = Vec::new();
        loop {
            if let Some(length) = length {
                if witnesses.len() == length as usize {
                    break;
                }
            } else if decoder.datatype().map_err(|_| "decode_failed")?
                == pallas_codec::minicbor::data::Type::Break
            {
                decoder.skip().map_err(|_| "decode_failed")?;
                break;
            }
            let key = decoder.u64().map_err(|_| "decode_failed")?;
            let field_start = decoder.position();
            decoder.skip().map_err(|_| "decode_failed")?;
            witnesses.push((key, &bytes[field_start..decoder.position()]));
        }
        let witness_bytes = &bytes[start..decoder.position()];
        let start = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        let success = &bytes[start..decoder.position()];
        let start = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        let auxiliary = &bytes[start..decoder.position()];
        if decoder.position() != bytes.len() {
            return Err("decode_failed");
        }
        Ok(Self {
            body,
            witnesses,
            witness_bytes,
            success,
            auxiliary,
        })
    }
    pub fn ledger_size(&self) -> usize {
        1 + self.body.len() + self.witness_bytes.len() + self.auxiliary.len()
    }
    pub fn assemble(&self, witnesses: &[u8]) -> Vec<u8> {
        let mut encoded = vec![0x84];
        encoded.extend_from_slice(self.body);
        encoded.extend_from_slice(witnesses);
        encoded.extend_from_slice(self.success);
        encoded.extend_from_slice(self.auxiliary);
        encoded
    }
}
#[derive(Debug, Serialize)]
pub struct SizeEstimate {
    pub estimated_bytes: u64,
    pub original_ledger_bytes: u64,
    pub bootstrap_bytes: u64,
    pub within_size_limit: bool,
    pub transaction_id: String,
}
pub fn estimate_signed(
    bytes: &[u8],
    keys: u64,
    boots: u64,
    maximum: u64,
    major: u64,
) -> Result<SizeEstimate, &'static str> {
    if keys > 65536 || boots > 65536 || maximum > u64::from(u32::MAX) || !(9..=12).contains(&major)
    {
        return Err("invalid_input");
    }
    let parts = Parts::decode(bytes)?;
    let retained: Vec<_> = parts
        .witnesses
        .iter()
        .filter(|(key, _)| *key != 0 && *key != 2)
        .collect();
    let mut encoder = Encoder::new(Vec::new());
    encoder
        .map(retained.len() as u64 + u64::from(keys > 0))
        .map_err(|_| "invalid_input")?;
    let mut witness_size = encoder.into_writer().len() as u64;
    for (key, raw) in retained {
        let mut encoder = Encoder::new(Vec::new());
        encoder.u64(*key).map_err(|_| "invalid_input")?;
        witness_size += encoder.into_writer().len() as u64 + raw.len() as u64;
    }
    if keys > 0 {
        witness_size += 1 + 3 + if keys <= 23 { 1 } else { 2 } + keys * 101;
    }
    let bootstrap_bytes = estimated_witness_bytes(boots, true);
    let estimated_bytes =
        1 + parts.body.len() as u64 + witness_size + parts.auxiliary.len() as u64 + bootstrap_bytes;
    Ok(SizeEstimate {
        estimated_bytes,
        original_ledger_bytes: parts.ledger_size() as u64,
        bootstrap_bytes,
        within_size_limit: estimated_bytes <= maximum,
        transaction_id: hex::encode(pallas_crypto::hash::Hasher::<256>::hash(parts.body)),
    })
}

#[derive(Debug, Serialize)]
pub struct WitnessCounts {
    pub vkey_witnesses: u64,
    pub bootstrap_witnesses: u64,
}
pub fn input_witness_counts(
    tx: &conway::Tx,
    utxos: &[UtxoEntry],
) -> Result<WitnessCounts, WalletError> {
    let mut counts = WitnessCounts {
        vkey_witnesses: 0,
        bootstrap_witnesses: 0,
    };
    let mut seen = HashSet::new();
    for input in tx.transaction_body.inputs.iter().chain(
        tx.transaction_body
            .collateral
            .iter()
            .flat_map(|inputs| inputs.iter()),
    ) {
        let id = UtxoId::new(TxHash(*input.transaction_id), input.index as u32);
        if !seen.insert(id.clone()) {
            continue;
        }
        let utxo = utxos
            .iter()
            .find(|utxo| utxo.id == id)
            .ok_or_else(|| WalletError::TxBuild("missing input UTxO".into()))?;
        use std::str::FromStr;
        let address = pallas_addresses::Address::from_str(utxo.address.as_str())
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        if matches!(address, pallas_addresses::Address::Byron(_)) {
            counts.bootstrap_witnesses += 1;
        } else if address
            .to_vec()
            .first()
            .is_some_and(|header| !matches!(header >> 4, 1 | 3 | 5 | 7))
        {
            counts.vkey_witnesses += 1;
        }
    }
    counts.vkey_witnesses += tx
        .transaction_body
        .required_signers
        .as_ref()
        .map_or(0, |signers| {
            signers.iter().collect::<HashSet<_>>().len() as u64
        });
    counts.vkey_witnesses += tx
        .transaction_body
        .withdrawals
        .as_ref()
        .map_or(0, |withdrawals| {
            withdrawals
                .iter()
                .filter(|(address, _)| address.first().is_some_and(|header| header >> 4 == 14))
                .count() as u64
        });
    for certificate in tx
        .transaction_body
        .certificates
        .iter()
        .flat_map(|certificates| certificates.iter())
    {
        let encoded = pallas_codec::minicbor::to_vec(certificate)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let mut decoder = Decoder::new(&encoded);
        decoder
            .array()
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let tag = decoder
            .u64()
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        if matches!(tag, 0 | 7) {
            continue;
        }
        if matches!(tag, 1 | 2 | 8..=13) {
            decoder
                .array()
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            if decoder
                .u64()
                .map_err(|e| WalletError::TxBuild(e.to_string()))?
                == 0
            {
                counts.vkey_witnesses += 1;
            }
        } else {
            counts.vkey_witnesses += 1;
        }
    }
    fn script_keys(script: &conway::NativeScript) -> u64 {
        match script {
            conway::NativeScript::ScriptPubkey(_) => 1,
            conway::NativeScript::ScriptAll(scripts)
            | conway::NativeScript::ScriptAny(scripts)
            | conway::NativeScript::ScriptNOfK(_, scripts) => scripts.iter().map(script_keys).sum(),
            _ => 0,
        }
    }
    let mut scripts = HashSet::new();
    for script in tx
        .transaction_witness_set
        .native_script
        .iter()
        .flat_map(|scripts| scripts.iter())
    {
        let encoded = pallas_codec::minicbor::to_vec(script)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        if scripts.insert(encoded) {
            counts.vkey_witnesses += script_keys(script);
        }
    }
    Ok(counts)
}
