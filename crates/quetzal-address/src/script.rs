//! Native scripts: canonical serialization, credential hashes and timelock evaluation.
use crate::AddressError;
use pallas_codec::minicbor::Encoder;
use pallas_crypto::hash::Hasher;
use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeScript {
    Signature {
        key_hash_hex: String,
    },
    All {
        scripts: Vec<NativeScript>,
    },
    Any {
        scripts: Vec<NativeScript>,
    },
    AtLeast {
        required: u8,
        scripts: Vec<NativeScript>,
    },
    Before {
        slot: u64,
    },
    After {
        slot: u64,
    },
}

impl NativeScript {
    pub fn serialize(&self) -> Result<Vec<u8>, AddressError> {
        let mut encoder = Encoder::new(vec![0]);
        self.encode(&mut encoder, 0)?;
        Ok(encoder.into_writer())
    }

    fn encode(&self, encoder: &mut Encoder<Vec<u8>>, depth: usize) -> Result<(), AddressError> {
        if depth > 8 {
            return Err(AddressError::Invalid);
        }
        match self {
            Self::Signature { key_hash_hex } => {
                let hash = hex::decode(key_hash_hex).map_err(|_| AddressError::Invalid)?;
                if hash.len() != 28 {
                    return Err(AddressError::Invalid);
                }
                encoder
                    .array(2)
                    .map_err(|_| AddressError::Invalid)?
                    .u8(0)
                    .map_err(|_| AddressError::Invalid)?
                    .bytes(&hash)
                    .map_err(|_| AddressError::Invalid)?;
            }
            Self::All { scripts } | Self::Any { scripts } | Self::AtLeast { scripts, .. } => {
                if scripts.len() > 16 {
                    return Err(AddressError::Invalid);
                }
                if let Self::AtLeast { required, .. } = self {
                    encoder
                        .array(3)
                        .map_err(|_| AddressError::Invalid)?
                        .u8(3)
                        .map_err(|_| AddressError::Invalid)?
                        .u8(*required)
                        .map_err(|_| AddressError::Invalid)?;
                } else {
                    encoder
                        .array(2)
                        .map_err(|_| AddressError::Invalid)?
                        .u8(if matches!(self, Self::All { .. }) {
                            1
                        } else {
                            2
                        })
                        .map_err(|_| AddressError::Invalid)?;
                }
                encoder
                    .array(scripts.len() as u64)
                    .map_err(|_| AddressError::Invalid)?;
                for script in scripts {
                    script.encode(encoder, depth + 1)?;
                }
            }
            Self::Before { slot } | Self::After { slot } => {
                encoder
                    .array(2)
                    .map_err(|_| AddressError::Invalid)?
                    .u8(if matches!(self, Self::After { .. }) {
                        4
                    } else {
                        5
                    })
                    .map_err(|_| AddressError::Invalid)?
                    .u64(*slot)
                    .map_err(|_| AddressError::Invalid)?;
            }
        }
        Ok(())
    }

    pub fn hash(&self) -> Result<[u8; 28], AddressError> {
        Ok(*Hasher::<224>::hash(&self.serialize()?))
    }

    /// Structural ledger compatibility, independent of a particular signer set.
    pub fn valid(&self) -> bool {
        match self {
            Self::Signature { key_hash_hex } => {
                hex::decode(key_hash_hex).is_ok_and(|bytes| bytes.len() == 28)
            }
            Self::All { scripts } => scripts.iter().all(Self::valid),
            Self::Any { scripts } => scripts.iter().any(Self::valid),
            Self::AtLeast { required, scripts } => {
                usize::from(*required) <= scripts.iter().filter(|script| script.valid()).count()
            }
            Self::Before { .. } | Self::After { .. } => true,
        }
    }

    pub fn satisfied(
        &self,
        signers: &BTreeSet<[u8; 28]>,
        invalid_before: Option<u64>,
        invalid_hereafter: Option<u64>,
    ) -> bool {
        match self {
            Self::Signature { key_hash_hex } => hex::decode(key_hash_hex)
                .ok()
                .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
                .is_some_and(|hash| signers.contains(&hash)),
            Self::All { scripts } => scripts
                .iter()
                .all(|script| script.satisfied(signers, invalid_before, invalid_hereafter)),
            Self::Any { scripts } => scripts
                .iter()
                .any(|script| script.satisfied(signers, invalid_before, invalid_hereafter)),
            Self::AtLeast { required, scripts } => {
                usize::from(*required)
                    <= scripts
                        .iter()
                        .filter(|script| {
                            script.satisfied(signers, invalid_before, invalid_hereafter)
                        })
                        .count()
            }
            Self::Before { slot } => invalid_hereafter.is_some_and(|bound| bound <= *slot),
            Self::After { slot } => invalid_before.is_some_and(|bound| bound >= *slot),
        }
    }
}
