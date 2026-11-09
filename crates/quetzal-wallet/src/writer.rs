//! Ledger witnesses and staking certificates shared by transaction writers.
use pallas_codec::minicbor;
use pallas_primitives::{conway, Fragment, NonEmptySet, StakeCredential};
use quetzal_crypto::Bip32PrivateKey;
use serde_json::{json, Value};

fn bytes(input: &Value, field: &str) -> Result<Vec<u8>, &'static str> {
    hex::decode(input[field].as_str().ok_or("invalid_input")?).map_err(|_| "invalid_input")
}
fn hash(input: &Value, field: &str) -> Result<pallas_crypto::hash::Hash<28>, &'static str> {
    let bytes: [u8; 28] = bytes(input, field)?
        .try_into()
        .map_err(|_| "invalid_input")?;
    Ok(bytes.into())
}

pub fn vkey_witness(body_hash: &[u8; 32], key: &Bip32PrivateKey) -> conway::VKeyWitness {
    conway::VKeyWitness {
        vkey: key.public_key().to_vk_bytes().to_vec().into(),
        signature: quetzal_crypto::sign(key, body_hash).to_vec().into(),
    }
}

pub fn bootstrap_witness(
    body_hash: &[u8; 32],
    key: &Bip32PrivateKey,
    address: &[u8],
    magic: Option<u32>,
) -> Result<conway::BootstrapWitness, &'static str> {
    use pallas_addresses::byron::AddrAttrProperty;
    let mut attributes = Vec::new();
    if let Ok(pallas_addresses::Address::Byron(address)) =
        pallas_addresses::Address::from_bytes(address)
    {
        let decoded = address.decode().map_err(|_| "invalid_input")?;
        for attribute in decoded.attributes.iter() {
            if matches!(attribute, AddrAttrProperty::DerivationPath(_)) {
                attributes.push(attribute.clone());
            }
        }
    }
    if let Some(magic) = magic {
        attributes.push(AddrAttrProperty::NetworkTag(
            minicbor::to_vec(magic).map_err(|_| "invalid_input")?.into(),
        ));
    }
    let attributes: pallas_addresses::byron::AddrAttrs = attributes.into();
    let attributes = minicbor::to_vec(attributes).map_err(|_| "invalid_input")?;
    let public = key.public_key().to_raw_bytes();
    Ok(conway::BootstrapWitness {
        public_key: public[..32].to_vec().into(),
        signature: quetzal_crypto::sign(key, body_hash).to_vec().into(),
        chain_code: public[32..].to_vec().into(),
        attributes: attributes.into(),
    })
}

/// Merge a witness without discarding existing scripts or duplicating keys.
pub fn merge_vkey(witnesses: &mut conway::WitnessSet, witness: conway::VKeyWitness) {
    let mut merged: Vec<_> = witnesses
        .vkeywitness
        .take()
        .map(|set| set.to_vec())
        .unwrap_or_default();
    merged.retain(|existing| existing.vkey != witness.vkey);
    merged.push(witness);
    merged.sort_by_key(|witness| pallas_crypto::hash::Hasher::<224>::hash(&witness.vkey));
    witnesses.vkeywitness = NonEmptySet::from_vec(merged);
}

pub enum StakeAction {
    Join(pallas_crypto::hash::Hash<28>),
    JoinRegistering(pallas_crypto::hash::Hash<28>),
    Quit,
    Vote(conway::DRep),
    VoteRegistering(conway::DRep),
}
pub fn certificates(
    credential: StakeCredential,
    deposit: Option<u64>,
    action: StakeAction,
) -> Result<Vec<conway::Certificate>, &'static str> {
    use conway::Certificate as C;
    Ok(match action {
        StakeAction::Join(pool) => vec![C::StakeDelegation(credential, pool)],
        StakeAction::JoinRegistering(pool) => vec![
            C::Reg(credential.clone(), deposit.ok_or("deposit_required")?),
            C::StakeDelegation(credential, pool),
        ],
        StakeAction::Quit => vec![C::UnReg(credential, deposit.ok_or("deposit_required")?)],
        StakeAction::Vote(drep) => vec![C::VoteDeleg(credential, drep)],
        StakeAction::VoteRegistering(drep) => vec![
            C::Reg(credential.clone(), deposit.ok_or("deposit_required")?),
            C::VoteDeleg(credential, drep),
        ],
    })
}

pub fn execute(operation: &str, input: &Value) -> Result<Value, &'static str> {
    if operation == "wallet.certificates" {
        let stake =
            quetzal_crypto::Bip32PublicKey::from_raw_bytes(&bytes(input, "stake_xpub_hex")?)
                .map_err(|_| "invalid_input")?;
        let credential = if let Some(script) = input.get("stake_script") {
            let script = conway::NativeScript::ScriptPubkey(hash(script, "key_hash_hex")?);
            let mut preimage = vec![0];
            preimage.extend(minicbor::to_vec(script).map_err(|_| "invalid_input")?);
            StakeCredential::ScriptHash(pallas_crypto::hash::Hasher::<224>::hash(&preimage))
        } else {
            StakeCredential::AddrKeyhash(pallas_crypto::hash::Hasher::<224>::hash(
                &stake.to_vk_bytes(),
            ))
        };
        let action = input["action"].as_str().ok_or("invalid_input")?;
        match input["era"].as_str() {
            Some("conway") => {}
            Some("dijkstra") => return Err("unsupported_era"),
            _ => return Err("invalid_input"),
        }
        let drep = || match input["drep"]["type"].as_str() {
            Some("abstain") => Ok(conway::DRep::Abstain),
            Some("no-confidence") => Ok(conway::DRep::NoConfidence),
            Some("key") => Ok(conway::DRep::Key(hash(&input["drep"], "hash_hex")?)),
            Some("script") => Ok(conway::DRep::Script(hash(&input["drep"], "hash_hex")?)),
            _ => Err("invalid_input"),
        };
        let action = match action {
            "join" => StakeAction::Join(hash(input, "pool_hex")?),
            "join-registering" => StakeAction::JoinRegistering(hash(input, "pool_hex")?),
            "quit" => StakeAction::Quit,
            "vote" => StakeAction::Vote(drep()?),
            "vote-registering" => StakeAction::VoteRegistering(drep()?),
            _ => return Err("invalid_input"),
        };
        let certificates = certificates(credential, input["deposit"].as_u64(), action)?;
        return Ok(
            json!({"certificates_cbor_hex":certificates.into_iter().map(|cert|minicbor::to_vec(cert).map(hex::encode).map_err(|_|"invalid_input")).collect::<Result<Vec<_>,_>>()?}),
        );
    }
    if operation == "witness.script" {
        let script = conway::NativeScript::ScriptPubkey(hash(input, "script_key_hash_hex")?);
        let mut preimage = vec![0];
        preimage.extend(minicbor::to_vec(&script).map_err(|_| "invalid_input")?);
        let script_hash = pallas_crypto::hash::Hasher::<224>::hash(&preimage);
        let mut encoder = minicbor::Encoder::new(Vec::new());
        encoder
            .map(1)
            .and_then(|e| e.u8(1))
            .and_then(|e| e.tag(minicbor::data::Tag::new(258)))
            .and_then(|e| e.array(1))
            .and_then(|e| e.encode(script))
            .map_err(|_| "invalid_input")?;
        return Ok(
            json!({"script_hash_hex":script_hash.to_string(),"witnesses_cbor_hex":hex::encode(encoder.into_writer())}),
        );
    }
    let raw = bytes(input, "transaction_cbor_hex")?;
    let parts = crate::txsize::Parts::decode(&raw)?;
    let mut transaction = conway::Tx::decode_fragment(&raw).map_err(|_| "decode_failed")?;
    let legacy = bytes(input, "xprv_hex")?;
    let private =
        quetzal_crypto::password::decrypt_legacy(&legacy, &bytes(input, "passphrase_hex")?)
            .map_err(|_| "invalid_input")?;
    let key = Bip32PrivateKey::from_raw_bytes(&private).map_err(|_| "invalid_input")?;
    let body_hash = pallas_crypto::hash::Hasher::<256>::hash(parts.body);
    let address = bytes(input, "address_hex")?;
    let magic = input["network_magic"]
        .as_u64()
        .map(u32::try_from)
        .transpose()
        .map_err(|_| "invalid_input")?;
    let boot = match input["style"].as_str() {
        Some("Byron") => true,
        Some("Shelley") => false,
        _ => return Err("invalid_input"),
    };
    if operation == "witness.build" {
        let witness = if boot {
            minicbor::to_vec(bootstrap_witness(&body_hash, &key, &address, magic)?)
        } else {
            minicbor::to_vec(vkey_witness(&body_hash, &key))
        }
        .map_err(|_| "invalid_input")?;
        return Ok(json!({"witness_cbor_hex":hex::encode(witness)}));
    }
    if operation != "witness.sign" {
        return Err("unsupported_operation");
    }
    if input["resolve_inputs"].as_bool() != Some(false)
        && !transaction.transaction_body.inputs.is_empty()
    {
        if boot {
            let witness = bootstrap_witness(&body_hash, &key, &address, magic)?;
            let mut merged = transaction
                .transaction_witness_set
                .bootstrap_witness
                .take()
                .map(|set| set.to_vec())
                .unwrap_or_default();
            merged.retain(|existing| existing.public_key != witness.public_key);
            merged.push(witness);
            merged.sort_by(|a, b| a.public_key.as_slice().cmp(b.public_key.as_slice()));
            transaction.transaction_witness_set.bootstrap_witness = NonEmptySet::from_vec(merged);
        } else {
            merge_vkey(
                &mut transaction.transaction_witness_set,
                vkey_witness(&body_hash, &key),
            );
        }
    }
    let witnesses =
        minicbor::to_vec(transaction.transaction_witness_set).map_err(|_| "invalid_input")?;
    Ok(json!({"transaction_cbor_hex":hex::encode(parts.assemble(&witnesses))}))
}
