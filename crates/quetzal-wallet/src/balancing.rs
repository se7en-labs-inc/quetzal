//! Balance owned, unsigned partial transactions without changing wallet state.
use crate::{WalletError, WalletService};
use pallas_codec::minicbor;
use pallas_primitives::{alonzo, conway, Fragment};
use quetzal_network::statequery::ProtocolParams;
use quetzal_primitives::{AddressRole, Coin, UtxoEntry, UtxoId, Value, WalletId};
use std::collections::BTreeSet;

fn reject(code: &'static str, message: &str) -> WalletError {
    WalletError::Balancing {
        code,
        message: message.into(),
    }
}

// The ledger wallet encoder uses indefinite input sets above 23 elements.
// Preserve every other field from the typed Pallas encoding verbatim.
fn encode_transaction(transaction: &conway::Tx) -> Result<Vec<u8>, WalletError> {
    fn encode(transaction: &conway::Tx) -> Result<Vec<u8>, String> {
        let raw = transaction.encode_fragment().map_err(|e| e.to_string())?;
        if transaction.transaction_body.inputs.len() <= 23 {
            return Ok(raw);
        }
        let parts = crate::txsize::Parts::decode(&raw).map_err(str::to_owned)?;
        let mut decoder = minicbor::Decoder::new(parts.body);
        let fields = decoder
            .map()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "expected definite transaction body map".to_owned())?;
        for _ in 0..fields {
            let key = decoder.u64().map_err(|e| e.to_string())?;
            let start = decoder.position();
            decoder.skip().map_err(|e| e.to_string())?;
            if key == 0 {
                let mut encoder = minicbor::Encoder::new(Vec::new());
                encoder
                    .tag(minicbor::data::Tag::new(258))
                    .and_then(|e| e.begin_array())
                    .map_err(|e| e.to_string())?;
                for input in transaction.transaction_body.inputs.iter() {
                    encoder.encode(input).map_err(|e| e.to_string())?;
                }
                encoder.end().map_err(|e| e.to_string())?;
                let mut corrected = vec![0x84];
                corrected.extend_from_slice(&parts.body[..start]);
                corrected.extend(encoder.into_writer());
                corrected.extend_from_slice(&parts.body[decoder.position()..]);
                corrected.extend_from_slice(parts.witness_bytes);
                corrected.extend_from_slice(parts.success);
                corrected.extend_from_slice(parts.auxiliary);
                return Ok(corrected);
            }
        }
        Err("transaction body has no input set".into())
    }
    encode(transaction).map_err(WalletError::TxBuild)
}

pub fn validate(transaction: &conway::Tx) -> Result<(), WalletError> {
    let body = &transaction.transaction_body;
    if body.collateral.is_some() {
        return Err(reject(
            "balance_tx_existing_collateral",
            "I cannot balance transactions with pre-defined collateral.",
        ));
    }
    if body.collateral_return.is_some() {
        return Err(reject("balance_tx_existing_return_collateral", "Balancing transactions with pre-defined collateral return outputs is not yet supported."));
    }
    if body.total_collateral.is_some() {
        return Err(reject(
            "balance_tx_existing_total_collateral",
            "I cannot balance transactions with pre-defined total collateral.",
        ));
    }
    if transaction.transaction_witness_set.vkeywitness.is_some()
        || transaction
            .transaction_witness_set
            .bootstrap_witness
            .is_some()
    {
        return Err(reject(
            "balance_tx_existing_key_witnesses",
            "I cannot balance transactions with existing key witnesses.",
        ));
    }
    if body.mint.is_some()
        || body.withdrawals.is_some()
        || body.certificates.is_some()
        || body.proposal_procedures.is_some()
        || body.voting_procedures.is_some()
        || body.donation.is_some()
        || body.treasury_value.is_some()
        || body.script_data_hash.is_some()
        || body.reference_inputs.is_some()
        || body.required_signers.is_some()
        || transaction.transaction_witness_set.native_script.is_some()
        || transaction.transaction_witness_set.redeemer.is_some()
        || transaction.transaction_witness_set.plutus_data.is_some()
        || transaction
            .transaction_witness_set
            .plutus_v1_script
            .is_some()
        || transaction
            .transaction_witness_set
            .plutus_v2_script
            .is_some()
        || transaction
            .transaction_witness_set
            .plutus_v3_script
            .is_some()
        || !transaction.success
    {
        return Err(WalletError::UnsupportedTransaction("Balancing script execution, mint/burn, withdrawals and certificates is not implemented.".into()));
    }
    Ok(())
}

fn difference(total: &Value, requested: &Value, fee: u64) -> Option<Value> {
    let mut change = total.clone();
    change.coin = change
        .coin
        .checked_sub(requested.coin)?
        .checked_sub(Coin(fee))?;
    for (policy, names) in &requested.assets {
        for (name, quantity) in names {
            let current = change.assets.get_mut(policy)?.get_mut(name)?;
            *current = current.checked_sub(*quantity)?;
        }
    }
    change.assets.retain(|_, names| {
        names.retain(|_, quantity| *quantity > 0);
        !names.is_empty()
    });
    Some(change)
}

fn change_output(address: &[u8], value: &Value) -> Result<conway::TransactionOutput, WalletError> {
    let amount = if value.assets.is_empty() {
        alonzo::Value::Coin(value.coin.0)
    } else {
        let policies = value
            .assets
            .iter()
            .map(|(policy, names)| {
                let policy: [u8; 28] = hex::decode(policy)
                    .map_err(|error| WalletError::TxBuild(error.to_string()))?
                    .try_into()
                    .map_err(|_| WalletError::TxBuild("invalid policy hash".into()))?;
                let names = names
                    .iter()
                    .map(|(name, quantity)| {
                        let name = hex::decode(name)
                            .map_err(|error| WalletError::TxBuild(error.to_string()))?;
                        Ok((pallas_codec::utils::Bytes::from(name), *quantity))
                    })
                    .collect::<Result<Vec<_>, WalletError>>()?;
                Ok((pallas_crypto::hash::Hash::<28>::from(policy), names.into()))
            })
            .collect::<Result<Vec<_>, WalletError>>()?;
        alonzo::Value::Multiasset(value.coin.0, policies.into())
    };
    Ok(conway::TransactionOutput::Legacy(
        alonzo::TransactionOutput {
            address: address.to_vec().into(),
            amount,
            datum_hash: None,
        },
    ))
}

/// Empty unsigned prototype used by delegation-fee estimation.
pub fn fee_prototype() -> Result<Vec<u8>, WalletError> {
    let mut encoder = minicbor::Encoder::new(Vec::new());
    encoder
        .array(4)
        .and_then(|e| e.map(4))
        .and_then(|e| e.u8(0))
        .and_then(|e| e.tag(minicbor::data::Tag::new(258)))
        .and_then(|e| e.array(0))
        .and_then(|e| e.u8(1))
        .and_then(|e| e.array(0))
        .and_then(|e| e.u8(2))
        .and_then(|e| e.u8(0))
        .and_then(|e| e.u8(3))
        .and_then(|e| e.u64(u64::MAX))
        .and_then(|e| e.map(0))
        .and_then(|e| e.bool(true))
        .and_then(|e| e.null())
        .map_err(|error| WalletError::TxBuild(error.to_string()))?;
    Ok(encoder.into_writer())
}

impl WalletService {
    pub async fn balance_partial(
        &self,
        wallet: &WalletId,
        raw: &[u8],
        parameters: &ProtocolParams,
    ) -> Result<Vec<u8>, WalletError> {
        self.get_wallet(wallet).await?;
        let mut transaction = conway::Tx::decode_fragment(raw)
            .map_err(|error| WalletError::TxBuild(error.to_string()))?;
        validate(&transaction)?;
        let mut available = self.available_utxos(wallet).await?;
        if available.is_empty() {
            return Err(reject("no_utxos_available", "Unable to create a transaction because the wallet has no unspent transaction outputs (UTxOs) available. A transaction must spend at least one UTxO in order to be accepted for inclusion on the Cardano blockchain."));
        }
        available.sort_by(|a, b| {
            a.id.tx_hash
                .0
                .cmp(&b.id.tx_hash.0)
                .then(a.id.output_index.cmp(&b.id.output_index))
        });
        let mut selected = Vec::<UtxoEntry>::new();
        let mut seen = BTreeSet::new();
        for input in transaction.transaction_body.inputs.iter() {
            let id = UtxoId::new(
                quetzal_primitives::TxHash(*input.transaction_id),
                u32::try_from(input.index)
                    .map_err(|_| reject("unresolved_inputs", "Input index is out of range."))?,
            );
            if !seen.insert((id.tx_hash.0, id.output_index)) {
                return Err(reject(
                    "unresolved_inputs",
                    "The transaction contains duplicate inputs.",
                ));
            }
            selected.push(available.iter().find(|entry|entry.id == id).cloned().ok_or_else(|| reject("unresolved_inputs", "The transaction references inputs that are unavailable to this wallet."))?);
        }
        let mut remaining = available
            .into_iter()
            .filter(|entry| !seen.contains(&(entry.id.tx_hash.0, entry.id.output_index)));
        let mut total = selected.iter().try_fold(Value::default(), |total, input| {
            total
                .checked_add(&input.value)
                .ok_or_else(|| WalletError::TxBuild("input value overflow".into()))
        })?;
        let requested = pallas_traverse::MultiEraTx::decode(raw)
            .map_err(|error| WalletError::TxBuild(error.to_string()))?
            .outputs()
            .iter()
            .try_fold(Value::default(), |total, output| {
                total
                    .checked_add(&crate::ledger::value(&output.value()))
                    .ok_or_else(|| WalletError::TxBuild("output value overflow".into()))
            })?;
        let mut addresses = self.get_addresses(wallet).await?;
        addresses.sort_by_key(|entry| entry.path.address_ix);
        let change_address = addresses
            .iter()
            .find(|entry| entry.path.role == AddressRole::Internal)
            .ok_or_else(|| WalletError::TxBuild("no change address".into()))?;
        let change_address =
            pallas_addresses::Address::from_bech32(change_address.address.as_str())
                .map_err(|error| WalletError::TxBuild(error.to_string()))?
                .to_vec();
        let outputs = transaction.transaction_body.outputs.clone();
        for output in &outputs {
            let encoded = minicbor::to_vec(output)
                .map_err(|error| WalletError::TxBuild(error.to_string()))?;
            let minimum = crate::minimum_coin::minimum_coin(&encoded, parameters.ada_per_utxo_byte)
                .map_err(|error| WalletError::TxBuild(error.into()))?;
            if minimum.below_minimum {
                return Err(WalletError::OutputTooSmall {
                    minimum: minimum
                        .minimum_coin
                        .try_into()
                        .map_err(|_| WalletError::TxBuild("minimum UTxO overflow".into()))?,
                });
            }
        }
        let mut fee = parameters.min_fee_b;
        for _ in 0..32 {
            let change = loop {
                if let Some(change) = difference(&total, &requested, fee) {
                    if !selected.is_empty() {
                        break change;
                    }
                }
                let input = remaining.next().ok_or(WalletError::InsufficientFunds)?;
                total = total
                    .checked_add(&input.value)
                    .ok_or_else(|| WalletError::TxBuild("input value overflow".into()))?;
                selected.push(input);
            };
            transaction.transaction_body.outputs = outputs.clone();
            if change.coin != Coin::ZERO || !change.assets.is_empty() {
                let output = change_output(&change_address, &change)?;
                let encoded = minicbor::to_vec(&output)
                    .map_err(|error| WalletError::TxBuild(error.to_string()))?;
                let minimum = parameters
                    .ada_per_utxo_byte
                    .checked_mul(160 + encoded.len() as u64)
                    .ok_or_else(|| WalletError::TxBuild("minimum UTxO overflow".into()))?;
                if change.coin.0 < minimum {
                    let input = remaining
                        .next()
                        .ok_or(WalletError::OutputTooSmall { minimum })?;
                    total = total
                        .checked_add(&input.value)
                        .ok_or_else(|| WalletError::TxBuild("input value overflow".into()))?;
                    selected.push(input);
                    continue;
                }
                transaction.transaction_body.outputs.push(output);
            }
            selected.sort_by(|a, b| {
                a.id.tx_hash
                    .0
                    .cmp(&b.id.tx_hash.0)
                    .then(a.id.output_index.cmp(&b.id.output_index))
            });
            transaction.transaction_body.inputs = selected
                .iter()
                .map(|input| conway::TransactionInput {
                    transaction_id: input.id.tx_hash.0.into(),
                    index: u64::from(input.id.output_index),
                })
                .collect::<Vec<_>>()
                .into();
            transaction.transaction_body.fee = fee;
            let encoded = encode_transaction(&transaction)?;
            let size = crate::txsize::estimate_signed(
                &encoded,
                selected.len() as u64,
                0,
                parameters.max_tx_size,
                9,
            )
            .map_err(|error| WalletError::TxBuild(error.into()))?;
            if !size.within_size_limit {
                return Err(reject("transaction_is_too_big", "I was not able to balance the transaction without exceeding the maximum transaction size."));
            }
            let next_fee = parameters
                .min_fee_a
                .checked_mul(size.estimated_bytes)
                .and_then(|fee| fee.checked_add(parameters.min_fee_b))
                .ok_or_else(|| WalletError::TxBuild("fee overflow".into()))?;
            if next_fee == fee {
                return Ok(encoded);
            }
            fee = next_fee;
        }
        Err(WalletError::TxBuild(
            "partial transaction fee did not converge".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parameters() -> ProtocolParams {
        ProtocolParams {
            min_fee_a: 44,
            min_fee_b: 155381,
            max_tx_size: 16384,
            ada_per_utxo_byte: 4310,
            key_deposit: 2_000_000,
            pool_deposit: 500_000_000,
            desired_pool_number: 3,
            max_collateral_inputs: 3,
            collateral_percentage: 150,
            max_value_size: 5000,
            memory_price: 0.0577,
            step_price: 0.0000721,
            era: 6,
        }
    }

    #[test]
    fn input_boundaries_preserve_body_identity_and_exact_signed_size() {
        for count in [23, 24, 255, 256] {
            let mut tx = conway::Tx::decode_fragment(&fee_prototype().unwrap()).unwrap();
            tx.transaction_body.inputs = (0..count)
                .map(|index| conway::TransactionInput {
                    transaction_id: [1; 32].into(),
                    index,
                })
                .collect::<Vec<_>>()
                .into();
            let canonical = tx.encode_fragment().unwrap();
            let encoded = encode_transaction(&tx).unwrap();
            let parts = crate::txsize::Parts::decode(&encoded).unwrap();
            let mut decoder = minicbor::Decoder::new(parts.body);
            decoder.map().unwrap();
            assert_eq!(decoder.u8().unwrap(), 0);
            decoder.tag().unwrap();
            assert_eq!(
                decoder.array().unwrap(),
                if count <= 23 { Some(count) } else { None }
            );
            assert_eq!(
                encoded.len() as isize - canonical.len() as isize,
                match count {
                    23 => 0,
                    24 | 255 => 0,
                    256 => -1,
                    _ => unreachable!(),
                }
            );
            let hash = crate::txsize::body_hash(&encoded).unwrap();
            assert_eq!(
                hash,
                *pallas_traverse::MultiEraTx::decode(&encoded)
                    .unwrap()
                    .hash()
            );
            let key = quetzal_crypto::derive_icarus_master_key(&[44; 20], b"").unwrap();
            let mut witnesses = tx.transaction_witness_set;
            crate::writer::merge_vkey(&mut witnesses, crate::writer::vkey_witness(&hash, &key));
            let signed = crate::txsize::replace_witnesses(&encoded, &witnesses).unwrap();
            assert_eq!(
                crate::txsize::Parts::decode(&signed).unwrap().body,
                parts.body
            );
            assert_eq!(crate::txsize::body_hash(&signed).unwrap(), hash);
            assert_eq!(
                crate::txsize::replace_witnesses(&signed, &witnesses).unwrap(),
                signed
            );
            let estimated =
                crate::txsize::estimate_signed(&encoded, 1, 0, u32::MAX as u64, 9).unwrap();
            assert_eq!(
                estimated.estimated_bytes as usize,
                crate::txsize::Parts::decode(&signed).unwrap().ledger_size()
            );
        }
    }

    #[test]
    fn collateral_guards_precede_funding_and_follow_reference_order() {
        let mut tx = conway::Tx::decode_fragment(&fee_prototype().unwrap()).unwrap();
        tx.transaction_body.total_collateral = Some(0);
        tx.transaction_body.collateral_return =
            Some(change_output(&[0; 57], &Value::default()).unwrap());
        tx.transaction_body.collateral =
            pallas_primitives::NonEmptySet::from_vec(vec![conway::TransactionInput {
                transaction_id: [0; 32].into(),
                index: 0,
            }]);
        for expected in [
            "balance_tx_existing_collateral",
            "balance_tx_existing_return_collateral",
            "balance_tx_existing_total_collateral",
        ] {
            assert!(
                matches!(validate(&tx), Err(WalletError::Balancing { code, .. }) if code == expected)
            );
            match expected {
                "balance_tx_existing_collateral" => tx.transaction_body.collateral = None,
                "balance_tx_existing_return_collateral" => {
                    tx.transaction_body.collateral_return = None
                }
                _ => tx.transaction_body.total_collateral = None,
            }
        }
        validate(&tx).unwrap();
    }

    #[tokio::test]
    async fn preselected_balance_conserves_assets_and_preserves_wallet_state() {
        let path = std::env::temp_dir().join(format!("qz-balance-{}", rand::random::<u64>()));
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "balance".into(),
                &[48; 20],
                b"public-test-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let addresses = service.get_addresses(&wallet.id).await.unwrap();
        let address = addresses
            .iter()
            .find(|entry| entry.path.role == AddressRole::External && entry.path.address_ix == 0)
            .unwrap();
        let destination = addresses
            .iter()
            .find(|entry| entry.path.role == AddressRole::External && entry.path.address_ix == 1)
            .unwrap();
        let mut value = Value::lovelace(Coin(50_000_000));
        value
            .assets
            .entry("11".repeat(28))
            .or_default()
            .insert("ff".into(), 7);
        let input = UtxoEntry {
            id: UtxoId::new(quetzal_primitives::TxHash([2; 32]), 24),
            address: address.address.clone(),
            value: value.clone(),
        };
        service.db.insert_utxo(&wallet.id, &input).unwrap();
        let extra = UtxoEntry {
            id: UtxoId::new(quetzal_primitives::TxHash([1; 32]), 0),
            address: address.address.clone(),
            value: Value::lovelace(Coin(10_000_000)),
        };
        service.db.insert_utxo(&wallet.id, &extra).unwrap();
        let mut tx = conway::Tx::decode_fragment(&fee_prototype().unwrap()).unwrap();
        tx.transaction_body.inputs = vec![conway::TransactionInput {
            transaction_id: [2; 32].into(),
            index: 24,
        }]
        .into();
        let mut payment = Value::lovelace(Coin(2_000_000));
        payment
            .assets
            .entry("11".repeat(28))
            .or_default()
            .insert("ff".into(), 3);
        tx.transaction_body.outputs = vec![change_output(
            &pallas_addresses::Address::from_bech32(destination.address.as_str())
                .unwrap()
                .to_vec(),
            &payment,
        )
        .unwrap()];
        let raw = tx.encode_fragment().unwrap();
        let balanced = service
            .balance_partial(&wallet.id, &raw, &parameters())
            .await
            .unwrap();
        let decoded = conway::Tx::decode_fragment(&balanced).unwrap();
        assert_eq!(decoded.transaction_body.inputs.len(), 1);
        assert_eq!(*decoded.transaction_body.inputs[0].transaction_id, [2; 32]);
        assert_eq!(
            decoded.transaction_body.outputs[0],
            tx.transaction_body.outputs[0]
        );
        assert_eq!(decoded.transaction_body.ttl, Some(u64::MAX));
        assert_eq!(decoded.transaction_body.validity_interval_start, None);
        let outputs = pallas_traverse::MultiEraTx::decode(&balanced)
            .unwrap()
            .outputs()
            .iter()
            .fold(Value::default(), |total, output| {
                total
                    .checked_add(&crate::ledger::value(&output.value()))
                    .unwrap()
            });
        assert_eq!(outputs.assets, value.assets);
        assert_eq!(
            outputs.coin.checked_add(Coin(decoded.transaction_body.fee)),
            Some(value.coin)
        );
        assert_eq!(
            service
                .balance_partial(&wallet.id, &raw, &parameters())
                .await
                .unwrap(),
            balanced
        );
        let mut small = parameters();
        small.max_tx_size = 200;
        assert!(matches!(
            service.balance_partial(&wallet.id, &raw, &small).await,
            Err(WalletError::Balancing {
                code: "transaction_is_too_big",
                ..
            })
        ));
        assert_eq!(service.get_utxos(&wallet.id).await.unwrap().len(), 2);
        assert!(service
            .get_transactions(&wallet.id, None, usize::MAX)
            .await
            .unwrap()
            .is_empty());
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
