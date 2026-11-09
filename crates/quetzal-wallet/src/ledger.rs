//! Era-directed decoding and ledger transaction identity.
use pallas_traverse::{Era, MultiEraTx};

pub fn value(value: &pallas_traverse::MultiEraValue<'_>) -> quetzal_primitives::Value {
    let mut result = quetzal_primitives::Value::lovelace(quetzal_primitives::Coin(value.coin()));
    if let pallas_primitives::alonzo::Value::Multiasset(_, policies) = value.into_alonzo() {
        for (policy, names) in policies.iter() {
            for (name, quantity) in names.iter() {
                if *quantity > 0 {
                    result
                        .assets
                        .entry(hex::encode(policy))
                        .or_default()
                        .insert(hex::encode(name.as_slice()), *quantity);
                }
            }
        }
    }
    result
}
pub fn asset_amounts(
    value: &quetzal_primitives::Value,
) -> Vec<quetzal_primitives::tx::AssetAmount> {
    value
        .assets
        .iter()
        .flat_map(|(policy, names)| {
            names.iter().map(
                move |(name, quantity)| quetzal_primitives::tx::AssetAmount {
                    policy_id: policy.clone(),
                    asset_name: name.clone(),
                    quantity: *quantity,
                },
            )
        })
        .collect()
}

pub fn withdrawals(
    tx: &MultiEraTx<'_>,
) -> Result<Vec<quetzal_primitives::tx::Withdrawal>, crate::WalletError> {
    tx.withdrawals_sorted_set()
        .into_iter()
        .map(|(address, coin)| {
            let address = pallas_addresses::Address::from_bytes(address)
                .map_err(|e| crate::WalletError::Cbor(e.to_string()))?;
            if !matches!(address, pallas_addresses::Address::Stake(_)) {
                return Err(crate::WalletError::Cbor(
                    "withdrawal is not a stake address".into(),
                ));
            }
            let address = address
                .to_bech32()
                .map_err(|e| crate::WalletError::Cbor(e.to_string()))?;
            Ok(quetzal_primitives::tx::Withdrawal {
                stake_address: address.into(),
                amount: quetzal_primitives::tx::CoinBundle::lovelace(coin.into()),
            })
        })
        .collect()
}

pub fn from_assets(
    coin: quetzal_primitives::Coin,
    assets: &[quetzal_primitives::tx::AssetAmount],
) -> Result<quetzal_primitives::Value, crate::WalletError> {
    let mut value = quetzal_primitives::Value::lovelace(coin);
    for asset in assets {
        let policy = hex::decode(&asset.policy_id)
            .map_err(|e| crate::WalletError::TxBuild(e.to_string()))?;
        let name = hex::decode(&asset.asset_name)
            .map_err(|e| crate::WalletError::TxBuild(e.to_string()))?;
        if policy.len() != 28 || name.len() > 32 {
            return Err(crate::WalletError::TxBuild(
                "invalid asset identifier".into(),
            ));
        }
        if asset.quantity == 0 {
            continue;
        }
        let quantity = value
            .assets
            .entry(hex::encode(policy))
            .or_default()
            .entry(hex::encode(name))
            .or_default();
        *quantity = quantity
            .checked_add(asset.quantity)
            .ok_or_else(|| crate::WalletError::TxBuild("asset quantity overflow".into()))?;
    }
    Ok(value)
}

pub fn roundtrip(era: &str, bytes: &[u8]) -> Result<(Vec<u8>, [u8; 32]), &'static str> {
    validate_cbor(bytes)?;
    let three_item = matches!(era, "shelley" | "allegra" | "mary" | "dijkstra");
    let era = match era {
        "byron" => Era::Byron,
        "shelley" => Era::Shelley,
        "allegra" => Era::Allegra,
        "mary" => Era::Mary,
        "alonzo" => Era::Alonzo,
        "babbage" => Era::Babbage,
        "conway" | "dijkstra" => Era::Conway,
        _ => return Err("invalid_input"),
    };
    if three_item {
        let mut decoder = pallas_codec::minicbor::Decoder::new(bytes);
        if decoder.array().map_err(|_| "decode_failed")? != Some(3) {
            return Err("decode_failed");
        }
        let body_start = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        let body_end = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        let witnesses_end = decoder.position();
        decoder.skip().map_err(|_| "decode_failed")?;
        if decoder.position() != bytes.len() {
            return Err("decode_failed");
        }
        let mut compatible = vec![0x84];
        compatible.extend_from_slice(&bytes[body_start..witnesses_end]);
        compatible.push(0xf5);
        compatible.extend_from_slice(&bytes[witnesses_end..]);
        MultiEraTx::decode_for_era(era, &compatible).map_err(|_| "decode_failed")?;
        Ok((
            bytes.to_vec(),
            *pallas_crypto::hash::Hasher::<256>::hash(&bytes[body_start..body_end]),
        ))
    } else {
        let tx = MultiEraTx::decode_for_era(era, bytes).map_err(|_| "decode_failed")?;
        Ok((tx.encode(), *tx.hash()))
    }
}

fn validate_cbor(bytes: &[u8]) -> Result<(), &'static str> {
    use ciborium::value::Value;
    fn unique(value: &Value) -> bool {
        match value {
            Value::Map(entries) => entries.iter().enumerate().all(|(index, (key, value))| {
                !entries[..index].iter().any(|(previous, _)| previous == key)
                    && unique(key)
                    && unique(value)
            }),
            Value::Array(items) => items.iter().all(unique),
            Value::Tag(_, value) => unique(value),
            _ => true,
        }
    }
    let mut cursor = std::io::Cursor::new(bytes);
    let value: Value = ciborium::de::from_reader(&mut cursor).map_err(|_| "decode_failed")?;
    if cursor.position() != bytes.len() as u64 || !unique(&value) {
        return Err("decode_failed");
    }
    Ok(())
}
