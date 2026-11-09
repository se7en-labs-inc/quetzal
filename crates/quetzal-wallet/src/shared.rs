//! Shared-wallet cosigner activation and ledger-native spending scripts.
use crate::{
    wallet::{
        derive_cosigner_key_hashes, encode_native_script, native_script_hash, template_cosigners,
        template_is_complete,
    },
    WalletError, WalletService,
};
use pallas_addresses::Network;
use pallas_primitives::conway;
use quetzal_primitives::{
    Address, AddressEntry, AddressRole, AddressState, DerivationPath, UtxoEntry, WalletId,
};
use serde_json::Value;

fn reject(code: &'static str, message: impl Into<String>) -> WalletError {
    WalletError::SharedWallet {
        code,
        message: message.into(),
    }
}

fn addresses(
    payment: &Value,
    delegation: Option<&Value>,
    account: u32,
    gap: u32,
    network: Network,
) -> Result<Vec<AddressEntry>, WalletError> {
    let payment_keys = template_cosigners(payment)?;
    let mut entries = Vec::new();
    for index in 0..gap {
        let stake = delegation
            .map(|template| {
                let keys = template_cosigners(template)?;
                native_script_hash(
                    &template["template"],
                    &derive_cosigner_key_hashes(&keys, 2, index)?,
                )
            })
            .transpose()?;
        for role in [0, 1] {
            let hash = native_script_hash(
                &payment["template"],
                &derive_cosigner_key_hashes(&payment_keys, role, index)?,
            )?;
            let address = if let Some(stake) = stake {
                Address(
                    pallas_addresses::ShelleyAddress::new(
                        network,
                        pallas_addresses::ShelleyPaymentPart::script_hash(hash),
                        pallas_addresses::ShelleyDelegationPart::Script(stake),
                    )
                    .to_bech32()
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?,
                )
            } else {
                quetzal_address::script_enterprise_address(hash, network)?
            };
            entries.push(AddressEntry {
                address,
                path: DerivationPath {
                    account_ix: account,
                    role: if role == 0 {
                        AddressRole::External
                    } else {
                        AddressRole::Internal
                    },
                    address_ix: index,
                },
                state: AddressState::Unused,
            });
        }
    }
    Ok(entries)
}

impl WalletService {
    pub async fn require_shared_active(&self, id: &WalletId) -> Result<(), WalletError> {
        self.get_shared_wallet(id).await?;
        let payment = self.get_script_template(id, b"payment_script").await?;
        let delegation = self.get_script_template(id, b"delegation_script").await?;
        if !payment.as_ref().is_some_and(template_is_complete)
            || delegation
                .as_ref()
                .is_some_and(|t| !template_is_complete(t))
        {
            return Err(reject("shared_wallet_incomplete", "This shared wallet is incomplete. Add the missing cosigner keys before creating transactions."));
        }
        Ok(())
    }

    pub async fn patch_shared_cosigners(
        &self,
        id: &WalletId,
        delegation: bool,
        patch: &Value,
        network: Network,
    ) -> Result<(), WalletError> {
        let _patch = self.shared_templates.lock().await;
        let wallet = self.get_shared_wallet(id).await?;
        let mut payment = self
            .get_script_template(id, b"payment_script")
            .await?
            .ok_or_else(|| WalletError::TxBuild("missing payment template".into()))?;
        let mut staking = self.get_script_template(id, b"delegation_script").await?;
        if template_is_complete(&payment) && staking.as_ref().is_none_or(template_is_complete) {
            return Err(reject("shared_wallet_active", "It looks like you've tried to add a cosigner key for a shared wallet that is active. This can be done only for an incomplete shared wallet."));
        }
        let target = if delegation {
            staking.as_mut().ok_or_else(|| reject("shared_wallet_no_delegation_template", "It looks like you've tried to add a cosigner key to a shared wallet's delegation template. This cannot be done for a wallet that does not define any delegation template."))?
        } else {
            &mut payment
        };
        let previous =
            serde_json::to_vec(target).map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let own_key = self.get_account_xpub_bytes(id).await?;
        let updates = patch
            .as_object()
            .filter(|p| !p.is_empty())
            .ok_or_else(|| WalletError::TxBuild("expected cosigner keys".into()))?;
        let mut candidate = target.clone();
        let kind = if delegation { "delegation" } else { "payment" };
        // Validate against a private candidate; rejected patches have no writes.
        for (cosigner, encoded) in updates {
            fn contains(node: &Value, cosigner: &str) -> bool {
                node.as_str() == Some(cosigner)
                    || node
                        .as_array()
                        .is_some_and(|a| a.iter().any(|n| contains(n, cosigner)))
                    || node
                        .as_object()
                        .is_some_and(|o| o.values().any(|n| contains(n, cosigner)))
            }
            if !contains(&candidate["template"], cosigner) {
                return Err(reject("shared_wallet_no_such_cosigner", format!("It looks like you've tried to add a cosigner key to a shared wallet's {kind} template for a non-existing cosigner index: {}", cosigner.trim_start_matches("cosigner#"))));
            }
            let encoded = encoded
                .as_str()
                .ok_or_else(|| WalletError::TxBuild("invalid cosigner key".into()))?;
            let (hrp, bytes) = quetzal_address::codec::bech32_decode(encoded)?;
            if hrp != "acct_shared_xvk" {
                return Err(WalletError::TxBuild("expected acct_shared_xvk".into()));
            }
            let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&bytes)?;
            if let Some(old) = candidate["cosigners"][cosigner].as_str() {
                let (_, old) = quetzal_address::codec::bech32_decode(old)?;
                if old == own_key {
                    return Err(reject("shared_wallet_cannot_update_key", "It looks like you've tried to update the key of a cosigner having a shared wallet's account key. Only other cosigner key(s) can be updated."));
                }
            }
            if template_cosigners(&candidate)?
                .values()
                .any(|key| key.to_raw_bytes() == public.to_raw_bytes())
            {
                return Err(reject("shared_wallet_key_already_exists", format!("It looks like you've tried to add a cosigner key to a shared wallet's {kind} template that is already ascribed to another cosigner. Please make sure to assign a different key to each cosigner.")));
            }
            candidate["cosigners"][cosigner] = Value::String(encoded.into());
        }
        *target = candidate;
        let updated =
            serde_json::to_vec(target).map_err(|e| WalletError::TxBuild(e.to_string()))?;
        let entries = if template_is_complete(&payment)
            && staking.as_ref().is_none_or(template_is_complete)
        {
            addresses(
                &payment,
                staking.as_ref(),
                wallet.account_index,
                wallet.address_pool_gap,
                network,
            )?
        } else {
            Vec::new()
        };
        self.db.patch_shared_template(
            id,
            if delegation {
                b"delegation_script"
            } else {
                b"payment_script"
            },
            &previous,
            &updated,
            &entries,
        )
    }

    pub(crate) async fn shared_input_scripts(
        &self,
        id: &WalletId,
        inputs: &[UtxoEntry],
    ) -> Result<Vec<conway::NativeScript>, WalletError> {
        let template = self
            .get_script_template(id, b"payment_script")
            .await?
            .ok_or_else(|| WalletError::TxBuild("missing payment template".into()))?;
        let keys = template_cosigners(&template)?;
        let entries = self.get_addresses(id).await?;
        let mut scripts = std::collections::BTreeMap::new();
        for input in inputs {
            let entry = entries
                .iter()
                .find(|e| e.address == input.address)
                .ok_or_else(|| WalletError::TxBuild("unknown shared input".into()))?;
            let hashes =
                derive_cosigner_key_hashes(&keys, entry.path.role.index(), entry.path.address_ix)?;
            let mut encoded = Vec::new();
            encode_native_script(&template["template"], &hashes, &mut encoded)?;
            let script = pallas_codec::minicbor::decode(&encoded)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?;
            scripts.insert(encoded, script);
        }
        Ok(scripts.into_values().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn shared_delegation_indices_match_pinned_reference_addresses() {
        let root = quetzal_crypto::derive_icarus_master_key(&[0; 20], b"").unwrap();
        let account = quetzal_crypto::shared_account_key(&root, 0).public_key();
        let template = json!({"cosigners":{"cosigner#0":quetzal_address::codec::bech32_encode("acct_shared_xvk", &account.to_raw_bytes()).unwrap()}, "template":{"all":["cosigner#0"]}});
        let entries = addresses(&template, Some(&template), 0, 3, Network::Testnet).unwrap();
        let expected = [
            "addr_test1xz5qel86pqxkzdfrl6e0c2mjkjrrr24esuht692gdsll4mn85xyccz7ju400augvx3gemnn9k6nqy2exyfke8qetpafqdmsn60",
            "addr_test1xqw25er393mrmhv33de37rug5j6ct4ktkrruvxwhxt4ejp6l04junxyzruqamfjkrv45cdkdttmxuu5qn98a6qxwtjtq35nhun",
            "addr_test1xr2578gzn32cs5tr9vexs9lwm3nkwf49eu6lnglsjjg5j9m5vc3cjssyzlmldswwqcg53s32zp6pc0zsu7582crdudcqda50mt",
        ];
        assert_eq!(
            entries
                .iter()
                .filter(|e| e.path.role == AddressRole::External)
                .map(|e| e.address.as_str())
                .collect::<Vec<_>>(),
            expected
        );
    }

    #[tokio::test]
    async fn peers_activate_atomically_and_sign_with_their_shared_account_keys() {
        let path = std::env::temp_dir().join(format!("quetzal-shared-{}", uuid::Uuid::new_v4()));
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        let password = b"public-shared-test-password";
        let mut wallets = Vec::new();
        for (index, entropy) in [[45; 20], [46; 20]].iter().enumerate() {
            let name = format!("cosigner#{index}");
            let template =
                json!({"cosigners":{&name:"self"},"template":{"all":["cosigner#0","cosigner#1"]}});
            wallets.push(
                service
                    .create_shared_wallet(crate::wallet::SharedWalletCreation {
                        name: name.clone(),
                        entropy,
                        passphrase: password,
                        account_ix: 0,
                        cosigners: HashMap::from([(name, None)]),
                        payment_template: &template,
                        delegation_template: Some(&template),
                        network: Network::Testnet,
                    })
                    .await
                    .unwrap(),
            );
        }
        let keys = [
            service
                .get_account_xpub_bytes(&wallets[0].id)
                .await
                .unwrap(),
            service
                .get_account_xpub_bytes(&wallets[1].id)
                .await
                .unwrap(),
        ];
        for index in 0..2 {
            let id = &wallets[index].id;
            assert!(service.require_shared_active(id).await.is_err());
            let original = service
                .get_script_template(id, b"payment_script")
                .await
                .unwrap();
            let invalid = json!({"cosigner#9":quetzal_address::codec::bech32_encode("acct_shared_xvk", &keys[1-index]).unwrap()});
            assert!(service
                .patch_shared_cosigners(id, false, &invalid, Network::Testnet)
                .await
                .is_err());
            assert_eq!(
                service
                    .get_script_template(id, b"payment_script")
                    .await
                    .unwrap(),
                original
            );
            assert!(service.get_addresses(id).await.unwrap().is_empty());
            let patch = json!({format!("cosigner#{}",1-index):quetzal_address::codec::bech32_encode("acct_shared_xvk", &keys[1-index]).unwrap()});
            let (payment, delegation) = tokio::join!(
                service.patch_shared_cosigners(id, false, &patch, Network::Testnet),
                service.patch_shared_cosigners(id, true, &patch, Network::Testnet)
            );
            payment.unwrap();
            delegation.unwrap();
            service.require_shared_active(id).await.unwrap();
            let entries = service.get_addresses(id).await.unwrap();
            assert_eq!(entries.len(), 40);
            assert!(service
                .patch_shared_cosigners(id, false, &patch, Network::Testnet)
                .await
                .is_err());
            let hash = [81; 32];
            let signatures = service
                .sign_transaction_inputs(id, password, &[entries[0].path.clone()], &hash)
                .await
                .unwrap();
            let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&keys[index])
                .unwrap()
                .derive(entries[0].path.role.index())
                .unwrap()
                .derive(entries[0].path.address_ix)
                .unwrap();
            assert_eq!(signatures[0].0, public.to_vk_bytes());
            assert!(quetzal_crypto::verify(
                &signatures[0].0,
                &hash,
                &signatures[0].1
            ));
        }
        let first = service.get_addresses(&wallets[0].id).await.unwrap();
        let second = service.get_addresses(&wallets[1].id).await.unwrap();
        assert_eq!(
            first.iter().map(|e| &e.address).collect::<Vec<_>>(),
            second.iter().map(|e| &e.address).collect::<Vec<_>>()
        );
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
