//! CIP-1855 policy templates resolved to the wallet's persisted policy key.
use crate::WalletError;
use quetzal_address::script::NativeScript;
use serde_json::Value;

pub fn script(template: &Value, key_hash: &[u8; 28]) -> Result<NativeScript, WalletError> {
    fn resolve(
        node: &Value,
        key: &[u8; 28],
        signatures: &mut usize,
        depth: usize,
    ) -> Result<NativeScript, WalletError> {
        if depth > 8 {
            return Err(WalletError::InvalidPolicyTemplate);
        }
        if node.as_str() == Some("cosigner#0") {
            *signatures += 1;
            return Ok(NativeScript::Signature {
                key_hash_hex: hex::encode(key),
            });
        }
        let object = node
            .as_object()
            .filter(|object| object.len() == 1)
            .ok_or(WalletError::InvalidPolicyTemplate)?;
        for name in ["all", "any"] {
            if let Some(nodes) = object.get(name) {
                let nodes = nodes
                    .as_array()
                    .filter(|nodes| !nodes.is_empty() && nodes.len() <= 16)
                    .ok_or(WalletError::InvalidPolicyTemplate)?;
                let scripts = nodes
                    .iter()
                    .map(|node| resolve(node, key, signatures, depth + 1))
                    .collect::<Result<Vec<_>, _>>()?;
                return Ok(if name == "all" {
                    NativeScript::All { scripts }
                } else {
                    NativeScript::Any { scripts }
                });
            }
        }
        if let Some(some) = object.get("some") {
            let required = some["at_least"]
                .as_u64()
                .and_then(|n| u8::try_from(n).ok())
                .filter(|n| *n > 0)
                .ok_or(WalletError::InvalidPolicyTemplate)?;
            let nodes = some["from"]
                .as_array()
                .filter(|nodes| nodes.len() <= 16 && usize::from(required) <= nodes.len())
                .ok_or(WalletError::InvalidPolicyTemplate)?;
            let scripts = nodes
                .iter()
                .map(|node| resolve(node, key, signatures, depth + 1))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(NativeScript::AtLeast { required, scripts });
        }
        for name in ["active_from", "active_until"] {
            if let Some(slot) = object.get(name).and_then(Value::as_u64) {
                return Ok(if name == "active_from" {
                    NativeScript::After { slot }
                } else {
                    NativeScript::Before { slot }
                });
            }
        }
        Err(WalletError::InvalidPolicyTemplate)
    }
    let mut signatures = 0;
    let script = resolve(template, key_hash, &mut signatures, 0)?;
    if signatures != 1 || !script.valid() {
        return Err(WalletError::InvalidPolicyTemplate);
    }
    Ok(script)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn policy_hash_resolves_the_template_and_includes_native_script_domain() {
        let hash = [7; 28];
        let resolved =
            script(&json!({"all":["cosigner#0", {"active_until": 200}]}), &hash).unwrap();
        let native = pallas_primitives::conway::NativeScript::ScriptAll(vec![
            pallas_primitives::conway::NativeScript::ScriptPubkey(hash.into()),
            pallas_primitives::conway::NativeScript::InvalidHereafter(200),
        ]);
        let mut ledger_bytes = vec![0];
        ledger_bytes.extend(pallas_codec::minicbor::to_vec(native).unwrap());
        assert_eq!(
            resolved.hash().unwrap(),
            *pallas_crypto::hash::Hasher::<224>::hash(&ledger_bytes)
        );
        assert_ne!(resolved.hash().unwrap(), hash);
    }

    #[test]
    fn policy_rejects_missing_repeated_and_foreign_cosigners() {
        for template in [
            json!("cosigner#1"),
            json!({"active_until":200}),
            json!({"all":["cosigner#0","cosigner#0"]}),
            json!({"all":[]}),
            json!({"some":{"at_least":2,"from":["cosigner#0"]}}),
        ] {
            assert!(matches!(
                script(&template, &[7; 28]),
                Err(WalletError::InvalidPolicyTemplate)
            ));
        }
    }

    #[tokio::test]
    async fn policy_key_requires_authorization_and_is_persisted() {
        let path = std::env::temp_dir().join(format!("quetzal-policy-{}", rand::random::<u64>()));
        let service = crate::WalletService::new(path.to_str().unwrap()).unwrap();
        let wallet = service
            .create_wallet(
                "policy".into(),
                &[46; 20],
                b"public-test-password",
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        assert!(matches!(
            service.policy_xpub(&wallet.id).await,
            Err(WalletError::MissingPolicyPublicKey)
        ));
        assert!(matches!(
            service
                .derive_policy_xpub(&wallet.id, b"wrong password")
                .await,
            Err(WalletError::WrongPassphrase)
        ));
        assert!(matches!(
            service.policy_xpub(&wallet.id).await,
            Err(WalletError::MissingPolicyPublicKey)
        ));
        let public = service
            .write_policy_xpub(&wallet.id, b"public-test-password")
            .await
            .unwrap();
        assert_eq!(service.policy_xpub(&wallet.id).await.unwrap(), public);
        assert_eq!(
            service.db.get_policy_xpub(&wallet.id).unwrap(),
            Some(public)
        );
        let watch = service
            .create_watch_wallet(
                "watch".into(),
                &service.get_account_xpub_bytes(&wallet.id).await.unwrap(),
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        assert!(matches!(
            service.policy_xpub(&watch.id).await,
            Err(WalletError::MissingPolicyPublicKey)
        ));
        service.delete_wallet(&wallet.id).await.unwrap();
        assert!(matches!(
            service.policy_xpub(&wallet.id).await,
            Err(WalletError::NotFound(_))
        ));
        drop(service);
        std::fs::remove_dir_all(path).unwrap();
    }
}
