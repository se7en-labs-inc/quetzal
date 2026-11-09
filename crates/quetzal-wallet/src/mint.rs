//! Native-asset supply changes and their authorizing ledger scripts.
use crate::{selection::Bundle, WalletError};
use pallas_primitives::conway;
use std::collections::BTreeMap;

#[derive(serde::Deserialize)]
pub struct Action {
    pub policy_script_template: serde_json::Value,
    #[serde(default)]
    pub asset_name: String,
    pub operation: Operation,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Mint {
        quantity: u64,
        receiving_address: Option<String>,
    },
    Burn {
        quantity: u64,
    },
}

#[derive(Default)]
pub struct Minting {
    pub quantities: BTreeMap<String, BTreeMap<String, i64>>,
    pub scripts: BTreeMap<[u8; 28], conway::NativeScript>,
}

impl Minting {
    pub fn add(
        &mut self,
        policy: [u8; 28],
        name: &[u8],
        quantity: i64,
        script: conway::NativeScript,
    ) -> Result<(), WalletError> {
        if name.len() > 32 || quantity == 0 {
            return Err(WalletError::TxBuild("invalid mint quantity or name".into()));
        }
        let mut encoded = vec![0];
        encoded.extend(
            pallas_codec::minicbor::to_vec(&script)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?,
        );
        if *pallas_crypto::hash::Hasher::<224>::hash(&encoded) != policy {
            return Err(WalletError::TxBuild(
                "mint policy does not match its script".into(),
            ));
        }
        let names = self.quantities.entry(hex::encode(policy)).or_default();
        let total = names.entry(hex::encode(name)).or_default();
        *total = total
            .checked_add(quantity)
            .ok_or_else(|| WalletError::TxBuild("mint quantity overflow".into()))?;
        names.retain(|_, quantity| *quantity != 0);
        self.quantities.retain(|_, names| !names.is_empty());
        self.scripts.insert(policy, script);
        Ok(())
    }

    pub fn accounting(&self) -> (Bundle, Bundle) {
        let mut minted = Bundle::default();
        let mut burned = Bundle::default();
        for (policy, names) in &self.quantities {
            for (name, quantity) in names {
                let bundle = if *quantity > 0 {
                    &mut minted
                } else {
                    &mut burned
                };
                bundle
                    .assets
                    .insert(format!("{policy}.{name}"), quantity.unsigned_abs());
            }
        }
        (minted, burned)
    }

    pub fn ledger(&self) -> Result<Option<conway::Mint>, WalletError> {
        let mut policies = Vec::new();
        for (policy, names) in &self.quantities {
            let policy: [u8; 28] = hex::decode(policy)
                .map_err(|e| WalletError::TxBuild(e.to_string()))?
                .try_into()
                .map_err(|_| WalletError::TxBuild("invalid mint policy".into()))?;
            let mut assets = Vec::new();
            for (name, quantity) in names {
                let name = hex::decode(name).map_err(|e| WalletError::TxBuild(e.to_string()))?;
                if name.len() > 32 {
                    return Err(WalletError::TxBuild("invalid mint asset name".into()));
                }
                if *quantity != 0 {
                    assets.push((
                        name.into(),
                        (*quantity)
                            .try_into()
                            .map_err(|_| WalletError::TxBuild("zero mint quantity".into()))?,
                    ));
                }
            }
            if let Some(assets) = conway::NonEmptyKeyValuePairs::from_vec(assets) {
                policies.push((policy.into(), assets));
            }
        }
        Ok(conway::NonEmptyKeyValuePairs::from_vec(policies))
    }
}

impl crate::WalletService {
    /// Sign native policies that actually use this wallet's persisted CIP-1855 key.
    pub async fn sign_minting_policy(
        &self,
        wallet: &quetzal_primitives::WalletId,
        passphrase: &[u8],
        tx: &conway::Tx,
        body_hash: &[u8; 32],
    ) -> Result<Option<([u8; 32], [u8; 64])>, WalletError> {
        let Some(mint) = &tx.transaction_body.mint else {
            return Ok(None);
        };
        let Some(scripts) = &tx.transaction_witness_set.native_script else {
            return Ok(None);
        };
        let public = match self.policy_xpub(wallet).await {
            Ok(public) => public,
            Err(WalletError::MissingPolicyPublicKey) => return Ok(None),
            Err(error) => return Err(error),
        };
        let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&public)?;
        let key_hash = *pallas_crypto::hash::Hasher::<224>::hash(&public.to_vk_bytes());
        fn uses_key(script: &conway::NativeScript, key: &[u8; 28]) -> bool {
            match script {
                conway::NativeScript::ScriptPubkey(hash) => **hash == *key,
                conway::NativeScript::ScriptAll(scripts)
                | conway::NativeScript::ScriptAny(scripts)
                | conway::NativeScript::ScriptNOfK(_, scripts) => {
                    scripts.iter().any(|script| uses_key(script, key))
                }
                _ => false,
            }
        }
        let mut needed = false;
        for script in scripts.iter().filter(|script| uses_key(script, &key_hash)) {
            let mut bytes = vec![0];
            bytes.extend(
                pallas_codec::minicbor::to_vec(script)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))?,
            );
            let policy = pallas_crypto::hash::Hasher::<224>::hash(&bytes);
            needed |= mint.iter().any(|(hash, _)| *hash == policy);
        }
        if !needed {
            return Ok(None);
        }
        let root = self.decrypt_root(wallet, passphrase).await?;
        let root = quetzal_crypto::Bip32PrivateKey::from_raw_bytes(&root)?;
        let key = quetzal_crypto::policy_account_key(&root);
        // A stale or incorrectly imported public key must never result in a
        // signature from a different policy key.
        if key.public_key().to_raw_bytes() != public.to_raw_bytes() {
            return Err(WalletError::TxBuild(
                "policy public key differs from root".into(),
            ));
        }
        Ok(Some((
            key.public_key().to_vk_bytes(),
            quetzal_crypto::sign(&key, body_hash),
        )))
    }
}
