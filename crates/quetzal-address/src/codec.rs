//! Cardano address and credential encodings.
use crate::{script::NativeScript, AddressError};
use bech32::{primitives::decode::CheckedHrpstring, Bech32, Hrp};
use pallas_crypto::hash::Hasher;
use serde::Deserialize;
pub fn base58_encode(bytes: &[u8]) -> String {
    use base58::ToBase58;
    bytes.to_base58()
}
pub fn base58_decode(text: &str) -> Result<Vec<u8>, AddressError> {
    use base58::FromBase58;
    text.from_base58().map_err(|_| AddressError::Invalid)
}

pub fn bech32_encode(hrp: &str, bytes: &[u8]) -> Result<String, AddressError> {
    let hrp = Hrp::parse(hrp).map_err(|_| AddressError::Invalid)?;
    bech32::encode::<Bech32>(hrp, bytes).map_err(|_| AddressError::Invalid)
}
pub fn bech32_decode(text: &str) -> Result<(String, Vec<u8>), AddressError> {
    let decoded = CheckedHrpstring::new::<Bech32>(text).map_err(|_| AddressError::Invalid)?;
    let data = text.rsplit_once('1').ok_or(AddressError::Invalid)?.1;
    let symbols = data.len().checked_sub(6).ok_or(AddressError::Invalid)?;
    let padding = symbols * 5 % 8;
    if padding >= 5 {
        return Err(AddressError::Invalid);
    }
    if padding > 0 {
        let last = data.as_bytes()[symbols - 1].to_ascii_lowercase();
        let value = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l"
            .iter()
            .position(|symbol| *symbol == last)
            .ok_or(AddressError::Invalid)?;
        if value & ((1 << padding) - 1) != 0 {
            return Err(AddressError::Invalid);
        }
    }
    Ok((decoded.hrp().to_string(), decoded.byte_iter().collect()))
}
pub fn fingerprint(policy: &[u8], name: &[u8]) -> Result<String, AddressError> {
    if policy.len() != 28 || name.len() > 32 {
        return Err(AddressError::Invalid);
    }
    let mut payload = policy.to_vec();
    payload.extend_from_slice(name);
    bech32_encode("asset", Hasher::<160>::hash(&payload).as_ref())
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Credential {
    Key { hash_hex: String },
    Script { script: NativeScript },
}
impl Credential {
    fn digest(&self) -> Result<(u8, [u8; 28]), AddressError> {
        match self {
            Self::Key { hash_hex } => Ok((
                0,
                hex::decode(hash_hex)
                    .map_err(|_| AddressError::Invalid)?
                    .try_into()
                    .map_err(|_| AddressError::Invalid)?,
            )),
            Self::Script { script } => Ok((1, script.hash()?)),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AddressKind {
    Enterprise {
        payment: Credential,
    },
    Base {
        payment: Credential,
        stake: Credential,
    },
    Reward {
        stake: Credential,
    },
    Pointer {
        payment: Credential,
        slot: u64,
        transaction_index: u64,
        output_index: u64,
    },
}
fn variable_uint(mut number: u64, output: &mut Vec<u8>) {
    let mut bytes = vec![(number & 127) as u8];
    number >>= 7;
    while number > 0 {
        bytes.push((number & 127) as u8 | 128);
        number >>= 7;
    }
    output.extend(bytes.into_iter().rev());
}
pub fn address(kind: &AddressKind, network: u8) -> Result<(String, Vec<u8>), AddressError> {
    if network > 15 {
        return Err(AddressError::Invalid);
    }
    let mut bytes = Vec::new();
    match kind {
        AddressKind::Enterprise { payment } => {
            let (script, hash) = payment.digest()?;
            bytes.push(((6 + script) << 4) | network);
            bytes.extend_from_slice(&hash);
        }
        AddressKind::Base { payment, stake } => {
            let (p, ph) = payment.digest()?;
            let (s, sh) = stake.digest()?;
            bytes.push(((p + 2 * s) << 4) | network);
            bytes.extend_from_slice(&ph);
            bytes.extend_from_slice(&sh);
        }
        AddressKind::Reward { stake } => {
            let (script, hash) = stake.digest()?;
            bytes.push(((14 + script) << 4) | network);
            bytes.extend_from_slice(&hash);
        }
        AddressKind::Pointer {
            payment,
            slot,
            transaction_index,
            output_index,
        } => {
            let (script, hash) = payment.digest()?;
            bytes.push(((4 + script) << 4) | network);
            bytes.extend_from_slice(&hash);
            variable_uint(*slot, &mut bytes);
            variable_uint(*transaction_index, &mut bytes);
            variable_uint(*output_index, &mut bytes);
        }
    }
    let hrp = match (matches!(kind, AddressKind::Reward { .. }), network == 1) {
        (false, false) => "addr_test",
        (false, true) => "addr",
        (true, false) => "stake_test",
        (true, true) => "stake",
    };
    Ok((bech32_encode(hrp, &bytes)?, bytes))
}

pub fn governance_encode(role: &str, script: bool, hash: &[u8]) -> Result<String, AddressError> {
    if hash.len() != 28 {
        return Err(AddressError::Invalid);
    }
    let (hrp, header) = match role {
        "drep" => ("drep", 0x20),
        "committee_hot" => ("cc_hot", 0),
        "committee_cold" => ("cc_cold", 0x10),
        _ => return Err(AddressError::Invalid),
    };
    let mut bytes = vec![header | if script { 3 } else { 2 }];
    bytes.extend_from_slice(hash);
    bech32_encode(hrp, &bytes)
}
pub fn governance_decode(text: &str, script: bool) -> Result<Vec<u8>, AddressError> {
    let (hrp, bytes) = bech32_decode(text)?;
    if bytes.len() == 29 {
        let role = match bytes[0] >> 4 {
            0 => "cc_hot",
            1 => "cc_cold",
            2 => "drep",
            _ => return Err(AddressError::Invalid),
        };
        if hrp != role || bytes[0] & 15 != if script { 3 } else { 2 } {
            return Err(AddressError::Invalid);
        }
        Ok(bytes[1..].to_vec())
    } else if bytes.len() == 28
        && ((!script && ["drep_vkh", "cc_hot_vkh", "cc_cold_vkh"].contains(&hrp.as_str()))
            || (script
                && ["drep_script", "cc_hot_script", "cc_cold_script"].contains(&hrp.as_str())))
    {
        Ok(bytes)
    } else {
        Err(AddressError::Invalid)
    }
}
