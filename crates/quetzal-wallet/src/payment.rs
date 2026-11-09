//! Payment construction from actual UTxOs and live ledger fee parameters.
use pallas_addresses::Address as LedgerAddress;
use pallas_primitives::{conway, Fragment, NonEmptySet};
use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
use quetzal_network::statequery::ProtocolParams;
use quetzal_primitives::{AddressEntry, AddressRole, Coin, UtxoEntry, Value, WalletId};

use crate::{WalletError, WalletService};

#[derive(Clone)]
pub struct Payment {
    pub address: String,
    pub amount: Coin,
    pub assets: std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>>,
}
impl Payment {
    fn value(&self) -> Value {
        Value {
            coin: self.amount,
            assets: self.assets.clone(),
        }
    }
}

pub struct ConstructedPayment {
    pub cbor: Vec<u8>,
    pub fee: Coin,
    pub inputs: Vec<UtxoEntry>,
    pub change: Option<(AddressEntry, Value)>,
    pub outputs: Vec<Payment>,
}

#[derive(Clone, Copy)]
pub struct TransactionOptions<'a> {
    pub start: u64,
    pub expiry: u64,
    pub network: u8,
    pub metadata: Option<&'a serde_json::Value>,
    pub mint: Option<&'a crate::mint::Minting>,
    pub staking: Option<&'a crate::staking::Operation>,
}

fn output(address: LedgerAddress, value: &Value) -> Result<Output, WalletError> {
    let mut output = Output::new(address, value.coin.0);
    for (policy, names) in &value.assets {
        let policy: [u8; 28] = hex::decode(policy)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?
            .try_into()
            .map_err(|_| WalletError::TxBuild("invalid policy hash".into()))?;
        for (name, quantity) in names {
            if *quantity == 0 {
                continue;
            }
            let name = hex::decode(name).map_err(|e| WalletError::TxBuild(e.to_string()))?;
            output = output
                .add_asset(policy.into(), name, *quantity)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        }
    }
    Ok(output)
}

fn build(
    inputs: &[UtxoEntry],
    payments: &[Payment],
    change: &AddressEntry,
    change_value: &Value,
    fee: Coin,
    options: TransactionOptions<'_>,
) -> Result<Vec<u8>, WalletError> {
    let TransactionOptions {
        start,
        expiry,
        network,
        metadata,
        mint,
        staking,
    } = options;
    let mut staging = StagingTransaction::new()
        .network_id(network)
        .fee(fee.0)
        .invalid_from_slot(expiry);
    for input in inputs {
        staging = staging.input(Input::new(
            input.id.tx_hash.0.into(),
            u64::from(input.id.output_index),
        ));
    }
    for payment in payments {
        let address = payment
            .address
            .parse::<LedgerAddress>()
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        staging = staging.output(output(address, &payment.value())?);
    }
    if change_value.coin != Coin::ZERO || !change_value.assets.is_empty() {
        let address = change
            .address
            .as_str()
            .parse::<LedgerAddress>()
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        staging = staging.output(output(address, change_value)?);
    }
    let encoded = staging
        .build_conway_raw()
        .map(|built| built.tx_bytes.0)
        .map_err(|e| WalletError::TxBuild(e.to_string()))?;
    let mut tx =
        conway::Tx::decode_fragment(&encoded).map_err(|e| WalletError::TxBuild(e.to_string()))?;
    // Plain wallet outputs use the compact, ledger-supported legacy array.
    // The map form is needed only when an inline datum or reference script is present.
    for output in &mut tx.transaction_body.outputs {
        if let conway::TransactionOutput::PostAlonzo(value) = output {
            let amount = pallas_codec::minicbor::to_vec(&value.value)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            *output = conway::TransactionOutput::Legacy(conway::LegacyTransactionOutput {
                address: value.address.clone(),
                amount: pallas_codec::minicbor::decode(&amount)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?,
                datum_hash: None,
            });
        }
    }
    tx.transaction_body.validity_interval_start = Some(start);
    if let Some(staking) = staking {
        tx.transaction_body.certificates = NonEmptySet::from_vec(staking.certificates.clone());
        tx.transaction_body.withdrawals = conway::NonEmptyKeyValuePairs::from_vec(
            staking
                .withdrawals
                .iter()
                .map(|(address, coin)| (address.clone().into(), *coin))
                .collect(),
        );
    }
    if let Some(mint) = mint {
        tx.transaction_body.mint = mint.ledger()?;
        tx.transaction_witness_set.native_script =
            NonEmptySet::from_vec(mint.scripts.values().cloned().collect());
    }
    if let Some(metadata) = metadata {
        let encoded_metadata = crate::metadata::encode(metadata)?;
        let metadata = pallas_codec::minicbor::decode(&encoded_metadata)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let auxiliary =
            conway::AuxiliaryData::PostAlonzo(pallas_primitives::alonzo::PostAlonzoAuxiliaryData {
                metadata: Some(metadata),
                native_scripts: None,
                plutus_scripts: None,
            });
        let auxiliary_bytes = pallas_codec::minicbor::to_vec(&auxiliary)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        tx.transaction_body.auxiliary_data_hash = Some(
            pallas_crypto::hash::Hasher::<256>::hash(&auxiliary_bytes)
                .as_ref()
                .to_vec()
                .into(),
        );
        tx.auxiliary_data = pallas_codec::utils::Nullable::Some(auxiliary);
    }
    tx.encode_fragment()
        .map_err(|e| WalletError::TxBuild(e.to_string()))
}

fn signed_size(bytes: &[u8], count: usize, inputs: &[UtxoEntry]) -> Result<usize, WalletError> {
    let mut tx =
        conway::Tx::decode_fragment(bytes).map_err(|e| WalletError::TxBuild(e.to_string()))?;
    let bootstrap = crate::bootstrap::preview(inputs)?;
    let witnesses = (0..count.saturating_sub(bootstrap.len()))
        .map(|_| conway::VKeyWitness {
            vkey: vec![0; 32].into(),
            signature: vec![0; 64].into(),
        })
        .collect();
    tx.transaction_witness_set.vkeywitness = NonEmptySet::from_vec(witnesses);
    tx.transaction_witness_set.bootstrap_witness = NonEmptySet::from_vec(bootstrap);
    tx.encode_fragment()
        .map(|bytes| bytes.len().saturating_sub(1))
        .map_err(|e| WalletError::TxBuild(e.to_string()))
}

impl WalletService {
    /// Sweep the selected ledger inputs without creating wallet change.
    pub fn construct_sweep(
        &self,
        inputs: &[UtxoEntry],
        destination: &str,
        parameters: &ProtocolParams,
        expiry: u64,
        network: u8,
        metadata: Option<&serde_json::Value>,
    ) -> Result<ConstructedPayment, WalletError> {
        let total = inputs.iter().try_fold(Value::default(), |total, input| {
            total
                .checked_add(&input.value)
                .ok_or_else(|| WalletError::TxBuild("migration value overflow".into()))
        })?;
        let change = AddressEntry {
            address: quetzal_primitives::Address(destination.into()),
            path: quetzal_primitives::DerivationPath::shelley_internal(0, 0),
            state: quetzal_primitives::AddressState::Unused,
        };
        let count = inputs
            .iter()
            .map(|input| input.address.as_str())
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        let mut fee = parameters.min_fee_b;
        for _ in 0..16 {
            let amount = total
                .coin
                .checked_sub(Coin(fee))
                .ok_or(WalletError::InsufficientFunds)?;
            let payment = Payment {
                address: destination.into(),
                amount,
                assets: total.assets.clone(),
            };
            let cbor = build(
                inputs,
                &[payment],
                &change,
                &Value::default(),
                Coin(fee),
                TransactionOptions {
                    start: 0,
                    expiry,
                    network,
                    metadata,
                    mint: None,
                    staking: None,
                },
            )?;
            let size = signed_size(&cbor, count, inputs)?;
            if size as u64 > parameters.max_tx_size {
                return Err(WalletError::TxTooLarge);
            }
            let tx = conway::Tx::decode_fragment(&cbor)
                .map_err(|error| WalletError::TxBuild(error.to_string()))?;
            let encoded = pallas_codec::minicbor::to_vec(&tx.transaction_body.outputs[0])
                .map_err(|error| WalletError::TxBuild(error.to_string()))?;
            let minimum = parameters
                .ada_per_utxo_byte
                .checked_mul(160 + encoded.len() as u64)
                .ok_or_else(|| WalletError::TxBuild("minimum UTxO overflow".into()))?;
            if amount.0 < minimum {
                return Err(WalletError::OutputTooSmall { minimum });
            }
            let next = parameters
                .min_fee_a
                .checked_mul(size as u64)
                .and_then(|fee| fee.checked_add(parameters.min_fee_b))
                .ok_or_else(|| WalletError::TxBuild("migration fee overflow".into()))?;
            if next == fee {
                return Ok(ConstructedPayment {
                    cbor,
                    fee: Coin(fee),
                    inputs: inputs.to_vec(),
                    change: None,
                    outputs: vec![Payment {
                        address: destination.into(),
                        amount,
                        assets: total.assets.clone(),
                    }],
                });
            }
            fee = next;
        }
        Err(WalletError::TxBuild(
            "migration fee calculation did not converge".into(),
        ))
    }

    pub async fn construct_payment(
        &self,
        wallet: &WalletId,
        payments: &[Payment],
        parameters: &ProtocolParams,
        options: TransactionOptions<'_>,
    ) -> Result<ConstructedPayment, WalletError> {
        self.construct_payment_with_change(wallet, payments, parameters, options, None)
            .await
    }

    pub async fn construct_payment_with_change(
        &self,
        wallet: &WalletId,
        payments: &[Payment],
        parameters: &ProtocolParams,
        options: TransactionOptions<'_>,
        preferred_change: Option<&str>,
    ) -> Result<ConstructedPayment, WalletError> {
        let kind = self.get_wallet(wallet).await?.wallet_type;
        let shared = kind == quetzal_primitives::WalletType::Shared;
        if payments.is_empty() && options.mint.is_none() && options.staking.is_none() {
            return Err(WalletError::TxBuild("no payments".into()));
        }
        let utxos = self.available_utxos(wallet).await?;
        if utxos.is_empty() {
            return Err(WalletError::NoUtxos);
        }
        let by_id: std::collections::HashMap<_, _> = utxos
            .iter()
            .map(|utxo| {
                (
                    format!("{}#{}", utxo.id.tx_hash, utxo.id.output_index),
                    utxo,
                )
            })
            .collect();
        let mut addresses = self.db.get_addresses(wallet)?;
        addresses.sort_by_key(|entry| entry.path.address_ix);
        let allocated: std::collections::BTreeSet<_> = self
            .db
            .known_addresses(wallet)?
            .into_iter()
            .filter(|entry| entry.path.role == AddressRole::Internal)
            .map(|entry| entry.address.0)
            .collect();
        let preferred = preferred_change
            .map(|address| {
                addresses
                    .iter()
                    .find(|entry| entry.address.as_str() == address)
                    .ok_or_else(|| WalletError::TxBuild("unowned preferred change address".into()))
            })
            .transpose()?;
        let change_address = preferred
            .or_else(|| {
                addresses
                    .iter()
                    .find(|entry| {
                        entry.path.role == AddressRole::Internal
                            && entry.state == quetzal_primitives::AddressState::Unused
                            && !allocated.contains(entry.address.as_str())
                    })
                    .or_else(|| {
                        addresses.iter().find(|entry| {
                            entry.path.role == AddressRole::Internal
                                && entry.state == quetzal_primitives::AddressState::Unused
                        })
                    })
                    .or_else(|| {
                        (kind == quetzal_primitives::WalletType::Byron)
                            .then(|| {
                                addresses
                                    .iter()
                                    .find(|entry| {
                                        entry.state == quetzal_primitives::AddressState::Unused
                                    })
                                    .or_else(|| addresses.first())
                            })
                            .flatten()
                    })
            })
            .cloned()
            .ok_or_else(|| WalletError::TxBuild("no change address".into()))?;
        let mut payments = payments.to_vec();
        let preview = build(
            &[],
            &payments,
            &change_address,
            &Value::default(),
            Coin::ZERO,
            options,
        )?;
        let preview = conway::Tx::decode_fragment(&preview)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        for (payment, output) in payments.iter_mut().zip(&preview.transaction_body.outputs) {
            let encoded = pallas_codec::minicbor::to_vec(output)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            let minimum = crate::minimum_coin::minimum_coin(&encoded, parameters.ada_per_utxo_byte)
                .map_err(|e| WalletError::TxBuild(e.into()))?
                .wallet_minimum_coin;
            if payment.amount == Coin::ZERO {
                payment.amount = Coin(
                    minimum
                        .try_into()
                        .map_err(|_| WalletError::TxBuild("minimum UTxO overflow".into()))?,
                );
            }
        }
        let mut fee = parameters.min_fee_b;
        let mut change_floor = 0;
        for _ in 0..16 {
            let (mut extra_in, mut extra_out) = options
                .mint
                .map(crate::mint::Minting::accounting)
                .unwrap_or_default();
            if let Some(staking) = options.staking {
                extra_in.coin = staking
                    .withdrawals
                    .values()
                    .try_fold(staking.deposit_returned, |total, coin| {
                        total.checked_add(*coin)
                    })
                    .ok_or_else(|| WalletError::TxBuild("withdrawal amount overflow".into()))?;
                extra_out.coin = staking.deposit_taken;
            }
            let selection = crate::selection::select_accounted(
                &crate::selection::Request {
                    utxos: by_id
                        .iter()
                        .map(|(id, utxo)| crate::selection::Input {
                            id: id.clone(),
                            value: crate::selection::Bundle::from_value(&utxo.value),
                            eligible: true,
                        })
                        .collect(),
                    outputs: payments
                        .iter()
                        .map(|payment| crate::selection::Bundle::from_value(&payment.value()))
                        .collect(),
                    fee,
                    minimum_change_coin: change_floor,
                    maximum_inputs: utxos.len(),
                },
                &extra_in,
                &extra_out,
            )?;
            let selected = selection
                .inputs
                .iter()
                .map(|id| {
                    by_id
                        .get(id)
                        .map(|utxo| (*utxo).clone())
                        .ok_or_else(|| WalletError::TxBuild("selected unknown input".into()))
                })
                .collect::<Result<Vec<_>, _>>()?;
            let change = selection
                .change
                .into_iter()
                .next()
                .unwrap_or_default()
                .into_value()?;
            let mut cbor = build(
                &selected,
                &payments,
                &change_address,
                &change,
                Coin(fee),
                options,
            )?;
            let mut count = selected
                .iter()
                .map(|input| input.address.as_str())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            if options.mint.is_some_and(|mint| !mint.quantities.is_empty()) {
                count += 1;
            }
            if options.staking.is_some_and(|staking| {
                !staking.certificates.is_empty() || !staking.withdrawals.is_empty()
            }) {
                count += 1;
            }
            if shared {
                let mut scripts = self.shared_input_scripts(wallet, &selected).await?;
                if let Some(mint) = options.mint {
                    scripts.extend(mint.scripts.values().cloned());
                }
                fn keys(
                    script: &conway::NativeScript,
                    unique: &mut std::collections::BTreeSet<[u8; 28]>,
                ) {
                    match script {
                        conway::NativeScript::ScriptPubkey(key) => {
                            unique.insert(**key);
                        }
                        conway::NativeScript::ScriptAll(nodes)
                        | conway::NativeScript::ScriptAny(nodes)
                        | conway::NativeScript::ScriptNOfK(_, nodes) => {
                            for node in nodes {
                                keys(node, unique);
                            }
                        }
                        _ => {}
                    }
                }
                let mut unique = std::collections::BTreeSet::new();
                for script in &scripts {
                    keys(script, &mut unique);
                }
                count = unique.len();
                let mut tx = conway::Tx::decode_fragment(&cbor)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?;
                tx.transaction_witness_set.native_script = NonEmptySet::from_vec(scripts);
                cbor = crate::txsize::replace_witnesses(&cbor, &tx.transaction_witness_set)
                    .map_err(|e| WalletError::TxBuild(e.into()))?;
            }
            let size = signed_size(&cbor, count, &selected)?;
            if size as u64 > parameters.max_tx_size {
                return Err(WalletError::TxTooLarge);
            }
            let tx = conway::Tx::decode_fragment(&cbor)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            let mut retry_change = false;
            for (index, output) in tx.transaction_body.outputs.iter().enumerate() {
                let encoded = pallas_codec::minicbor::to_vec(output)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?;
                let minimum = parameters
                    .ada_per_utxo_byte
                    .checked_mul(160 + encoded.len() as u64)
                    .ok_or_else(|| WalletError::TxBuild("minimum UTxO overflow".into()))?;
                if index < payments.len() {
                    let conservative: u64 =
                        crate::minimum_coin::minimum_coin(&encoded, parameters.ada_per_utxo_byte)
                            .map_err(|e| WalletError::TxBuild(e.into()))?
                            .wallet_minimum_coin
                            .try_into()
                            .map_err(|_| WalletError::TxBuild("minimum UTxO overflow".into()))?;
                    if payments[index].amount.0 < conservative {
                        return Err(WalletError::OutputCoinTooSmall {
                            index,
                            specified: payments[index].amount.0,
                            minimum: conservative,
                        });
                    }
                } else if change.coin.0 < minimum {
                    change_floor = minimum;
                    retry_change = true;
                }
            }
            if retry_change {
                continue;
            }
            let next_fee = parameters
                .min_fee_a
                .checked_mul(size as u64)
                .and_then(|fee| fee.checked_add(parameters.min_fee_b))
                .ok_or_else(|| WalletError::TxBuild("fee overflow".into()))?;
            if next_fee == fee {
                return Ok(ConstructedPayment {
                    cbor,
                    fee: Coin(fee),
                    inputs: selected,
                    outputs: payments,
                    change: if change.coin == Coin::ZERO && change.assets.is_empty() {
                        None
                    } else {
                        Some((change_address, change))
                    },
                });
            }
            fee = next_fee;
        }
        Err(WalletError::TxBuild(
            "fee calculation did not converge".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn payment_conserves_native_assets_and_authenticates_metadata() {
        let path = format!("/tmp/quetzal-payment-test-{}", rand::random::<u64>());
        let service = WalletService::new(&path).unwrap();
        let wallet = service
            .create_wallet(
                "payment-test".into(),
                &[42; 20],
                b"public-test-passphrase",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let addresses = service.get_addresses(&wallet.id).await.unwrap();
        let input_address = addresses
            .iter()
            .find(|entry| entry.path.role == AddressRole::External && entry.path.address_ix == 0)
            .unwrap();
        let destination = addresses
            .iter()
            .find(|entry| entry.path.role == AddressRole::External && entry.path.address_ix == 1)
            .unwrap();
        let policy = "11".repeat(28);
        let other_policy = "22".repeat(28);
        let mut input_value = Value::lovelace(Coin(100_000_000));
        input_value
            .assets
            .entry(policy.clone())
            .or_default()
            .insert("ff".into(), 7);
        input_value
            .assets
            .entry(other_policy.clone())
            .or_default()
            .insert("00".into(), 3);
        let input = UtxoEntry {
            id: quetzal_primitives::UtxoId::new(quetzal_primitives::TxHash([1; 32]), 0),
            address: input_address.address.clone(),
            value: input_value.clone(),
        };
        service.db.insert_utxo(&wallet.id, &input).unwrap();
        let mut assets =
            std::collections::BTreeMap::<String, std::collections::BTreeMap<String, u64>>::new();
        assets
            .entry(policy.clone())
            .or_default()
            .insert("ff".into(), 3);
        let parameters = ProtocolParams {
            min_fee_a: 100,
            min_fee_b: 100000,
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
        };
        let metadata = serde_json::json!({"42":{"string":"public-test-metadata"}});
        let built = service
            .construct_payment(
                &wallet.id,
                &[Payment {
                    address: destination.address.0.clone(),
                    amount: Coin(2_000_000),
                    assets,
                }],
                &parameters,
                TransactionOptions {
                    start: 0,
                    expiry: 10000,
                    network: 0,
                    metadata: Some(&metadata),
                    mint: None,
                    staking: None,
                },
            )
            .await
            .unwrap();
        let tx = pallas_traverse::MultiEraTx::decode(&built.cbor).unwrap();
        let mut output_total = Value::default();
        for output in tx.outputs() {
            output_total = output_total
                .checked_add(&crate::ledger::value(&output.value()))
                .unwrap();
        }
        assert_eq!(input_value.assets, output_total.assets);
        assert_eq!(
            input_value.coin,
            output_total.coin.checked_add(built.fee).unwrap()
        );
        assert_eq!(
            crate::metadata::decode_ledger(tx.metadata().as_alonzo().unwrap()).unwrap(),
            metadata
        );
        let decoded = conway::Tx::decode_fragment(&built.cbor).unwrap();
        let pallas_codec::utils::Nullable::Some(auxiliary) = decoded.auxiliary_data else {
            panic!("missing metadata");
        };
        let auxiliary = pallas_codec::minicbor::to_vec(auxiliary).unwrap();
        assert_eq!(
            decoded
                .transaction_body
                .auxiliary_data_hash
                .unwrap()
                .as_slice(),
            pallas_crypto::hash::Hasher::<256>::hash(&auxiliary).as_ref()
        );
        assert_eq!(
            service
                .get_wallet(&wallet.id)
                .await
                .unwrap()
                .assets_available
                .len(),
            2
        );
        let swept = service
            .construct_sweep(
                std::slice::from_ref(&input),
                destination.address.as_str(),
                &parameters,
                10000,
                0,
                None,
            )
            .unwrap();
        let sweep = pallas_traverse::MultiEraTx::decode(&swept.cbor).unwrap();
        assert_eq!(sweep.outputs().len(), 1);
        let output = crate::ledger::value(&sweep.outputs()[0].value());
        assert_eq!(output.assets, input_value.assets);
        assert_eq!(
            output.coin.checked_add(swept.fee).unwrap(),
            input_value.coin
        );
        assert!(swept.change.is_none());
        assert_eq!(
            sweep.outputs()[0].address().unwrap().to_string(),
            destination.address.as_str()
        );
        service.delete_wallet(&wallet.id).await.unwrap();
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
