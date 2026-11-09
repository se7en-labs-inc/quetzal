//! Ledger bootstrap witnesses for owned Byron random and Icarus inputs.
use crate::{WalletError, WalletService};
use pallas_addresses::byron::{AddrType, AddressPayload, ByronAddress, SpendingData};
use pallas_codec::minicbor::bytes::ByteVec;
use pallas_primitives::{conway, Fragment, NonEmptySet};
use quetzal_primitives::{UtxoEntry, WalletId, WalletType};

pub(crate) fn payload(address: &str) -> Result<AddressPayload, WalletError> {
    let address =
        ByronAddress::from_base58(address).map_err(|e| WalletError::TxBuild(e.to_string()))?;
    let payload = address
        .decode()
        .map_err(|e| WalletError::TxBuild(e.to_string()))?;
    if ByronAddress::from_decoded(payload.clone()).to_vec() != address.to_vec() {
        return Err(WalletError::TxBuild(
            "invalid bootstrap address checksum".into(),
        ));
    }
    Ok(payload)
}

/// Exact attribute sizes are needed even when no password is available for a
/// fee preview. The public key, signature and chain code have fixed lengths.
pub(crate) fn preview(inputs: &[UtxoEntry]) -> Result<Vec<conway::BootstrapWitness>, WalletError> {
    let mut seen = std::collections::BTreeSet::new();
    let mut witnesses = Vec::new();
    for input in inputs {
        let address = input.address.as_str();
        if !matches!(
            address.parse::<pallas_addresses::Address>(),
            Ok(pallas_addresses::Address::Byron(_))
        ) || !seen.insert(address)
        {
            continue;
        }
        let attributes = pallas_codec::minicbor::to_vec(&payload(address)?.attributes)
            .map_err(|e| WalletError::TxBuild(e.to_string()))?;
        witnesses.push(conway::BootstrapWitness {
            public_key: vec![0; 32].into(),
            signature: vec![0; 64].into(),
            chain_code: vec![0; 32].into(),
            attributes: attributes.into(),
        });
    }
    Ok(witnesses)
}

impl WalletService {
    pub async fn sign_bootstrap_transaction(
        &self,
        wallet: &WalletId,
        password: &[u8],
        cbor: &[u8],
    ) -> Result<Vec<u8>, WalletError> {
        let meta = self.get_byron_wallet(wallet).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(
            &self.decrypt_root(wallet, password).await?,
        )?;
        let body_hash =
            crate::txsize::body_hash(cbor).map_err(|e| WalletError::TxBuild(e.into()))?;
        let mut tx =
            conway::Tx::decode_fragment(cbor).map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let utxos = self.get_utxos(wallet).await?;
        let addresses = self.get_addresses(wallet).await?;
        let mut witnesses = tx
            .transaction_witness_set
            .bootstrap_witness
            .take()
            .map(|set| set.to_vec())
            .unwrap_or_default();
        let mut signed = false;
        for input in tx.transaction_body.inputs.iter() {
            let Some(utxo) = utxos.iter().find(|utxo| {
                utxo.id.tx_hash.0 == *input.transaction_id
                    && u64::from(utxo.id.output_index) == input.index
            }) else {
                continue;
            };
            let entry = addresses
                .iter()
                .find(|entry| entry.address == utxo.address)
                .ok_or_else(|| WalletError::TxBuild("missing bootstrap derivation path".into()))?;
            let child = if meta.wallet_type == WalletType::Byron {
                root.derive_byron(entry.path.account_ix)
                    .derive_byron(entry.path.address_ix)
            } else {
                quetzal_crypto::derivation::icarus_account_key(&root, entry.path.account_ix)
                    .derive(entry.path.role.index())
                    .derive(entry.path.address_ix)
            };
            let public = child.public_key().to_raw_bytes();
            let payload = payload(entry.address.as_str())?;
            let expected = ByronAddress::from_decoded(AddressPayload::new(
                AddrType::PubKey,
                SpendingData::PubKey(ByteVec::from(public.to_vec())),
                payload.attributes.clone(),
            ));
            if expected.to_base58() != entry.address.as_str() {
                return Err(WalletError::TxBuild(
                    "bootstrap key does not match input address".into(),
                ));
            }
            let attributes = pallas_codec::minicbor::to_vec(&payload.attributes)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            let witness = conway::BootstrapWitness {
                public_key: public[..32].to_vec().into(),
                signature: quetzal_crypto::sign(&child, &body_hash).to_vec().into(),
                chain_code: public[32..].to_vec().into(),
                attributes: attributes.into(),
            };
            witnesses.retain(|old| {
                old.public_key != witness.public_key || old.attributes != witness.attributes
            });
            witnesses.push(witness);
            signed = true;
        }
        if !signed {
            return Err(WalletError::TxBuild("no owned bootstrap inputs".into()));
        }
        tx.transaction_witness_set.bootstrap_witness = NonEmptySet::from_vec(witnesses);
        crate::txsize::replace_witnesses(cbor, &tx.transaction_witness_set)
            .map_err(|e| WalletError::TxBuild(e.into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use quetzal_primitives::{Coin, TxHash, UtxoId, Value};

    #[tokio::test]
    async fn random_and_icarus_bootstrap_witnesses_authenticate_inputs_without_changing_the_body() {
        let directory =
            std::env::temp_dir().join(format!("quetzal-bootstrap-{}", rand::random::<u64>()));
        let service = WalletService::new(directory.to_str().unwrap()).unwrap();
        for style in ["random", "icarus"] {
            let password = b"public-bootstrap-password";
            let wallet = service
                .create_byron_wallet_style(style.into(), &[21; 16], password, Some(42), style)
                .await
                .unwrap();
            let address = if style == "random" {
                service
                    .create_random_address(&wallet.id, password, Some(0x80001234), Some(42))
                    .await
                    .unwrap()
            } else {
                service
                    .get_addresses(&wallet.id)
                    .await
                    .unwrap()
                    .into_iter()
                    .find(|entry| entry.path.role == quetzal_primitives::AddressRole::External)
                    .unwrap()
            };
            let input = UtxoEntry {
                id: UtxoId::new(TxHash([21; 32]), 0),
                address: address.address,
                value: Value::lovelace(Coin(100_000_000)),
            };
            service.db.insert_utxo(&wallet.id, &input).unwrap();
            let parameters = quetzal_network::statequery::ProtocolParams {
                min_fee_a: 100,
                min_fee_b: 100000,
                max_tx_size: 16384,
                ada_per_utxo_byte: 4310,
                key_deposit: 1000000,
                pool_deposit: 0,
                desired_pool_number: 3,
                max_collateral_inputs: 3,
                collateral_percentage: 150,
                max_value_size: 5000,
                memory_price: 0.0577,
                step_price: 0.0000721,
                era: 6,
            };
            let built = service
                .construct_sweep(
                    std::slice::from_ref(&input),
                    input.address.as_str(),
                    &parameters,
                    10000,
                    0,
                    None,
                )
                .unwrap();
            let hash = crate::txsize::body_hash(&built.cbor).unwrap();
            assert!(service
                .sign_bootstrap_transaction(&wallet.id, b"wrong-password", &built.cbor)
                .await
                .is_err());
            let signed = service
                .sign_bootstrap_transaction(&wallet.id, password, &built.cbor)
                .await
                .unwrap();
            assert_eq!(crate::txsize::body_hash(&signed).unwrap(), hash);
            let tx = conway::Tx::decode_fragment(&signed).unwrap();
            let witnesses = tx.transaction_witness_set.bootstrap_witness.unwrap();
            assert_eq!(witnesses.len(), 1);
            let witness = &witnesses[0];
            let public: [u8; 32] = witness.public_key.as_slice().try_into().unwrap();
            let signature: [u8; 64] = witness.signature.as_slice().try_into().unwrap();
            assert!(quetzal_crypto::signing::verify(&public, &hash, &signature));
            assert_eq!(
                witness.attributes.as_slice(),
                pallas_codec::minicbor::to_vec(
                    &payload(input.address.as_str()).unwrap().attributes
                )
                .unwrap()
            );
            assert_eq!(
                built.fee.0,
                parameters.min_fee_a * (signed.len() as u64 - 1) + parameters.min_fee_b
            );
            let repeated = service
                .sign_bootstrap_transaction(&wallet.id, password, &signed)
                .await
                .unwrap();
            assert_eq!(signed, repeated);
        }
        drop(service);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
