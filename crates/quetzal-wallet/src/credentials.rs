//! Authenticated, atomic credential transitions used by wallet operations.
use crate::{WalletError, WalletService};
use quetzal_crypto::{passphrase::LegacyScheme, Bip32PrivateKey, EncryptedKey};
use quetzal_primitives::{PassphraseScheme, WalletId, WalletMeta, WalletType};
use rand::RngCore;

pub struct UnlockedRoot {
    pub key: Bip32PrivateKey,
    pub access: &'static str,
    pub migration_warning: bool,
}

fn scheme(key: &EncryptedKey) -> PassphraseScheme {
    match key.credential_scheme() {
        "pbkdf2" => PassphraseScheme::Pbkdf2HmacSha512,
        "scrypt" => PassphraseScheme::Scrypt,
        _ => PassphraseScheme::Argon2idV2,
    }
}

fn authenticate(key: &EncryptedKey, password: &[u8]) -> Result<Bip32PrivateKey, WalletError> {
    let raw = key.decrypt(password).map_err(|error| match error {
        quetzal_crypto::CryptoError::InvalidPassphrase => WalletError::WrongPassphrase,
        error => WalletError::Crypto(error),
    })?;
    Bip32PrivateKey::from_raw_bytes(&raw).map_err(WalletError::from)
}

impl WalletService {
    /// Authenticate a recovery phrase by wallet identity, then replace the
    /// encrypted root and metadata atomically without needing the old password.
    pub async fn recover_passphrase(
        &self,
        id: &WalletId,
        entropy: &[u8],
        second_factor: &[u8],
        password: &[u8],
    ) -> Result<(), WalletError> {
        let (previous, meta) = self.credential_state(id).await?;
        if meta.wallet_type != WalletType::Shelley {
            return Err(WalletError::WrongMnemonic(id.to_hex()));
        }
        let key = quetzal_crypto::derive_icarus_master_key(entropy, second_factor)?;
        let expected = WalletId(*pallas_crypto::hash::Hasher::<160>::hash(
            &key.public_key().to_raw_bytes(),
        ));
        if expected != *id {
            return Err(WalletError::WrongMnemonic(id.to_hex()));
        }
        let replacement = EncryptedKey::V2(
            quetzal_crypto::rootkey::RootKey::encrypt(&key, password, &mut rand::rngs::OsRng)
                .map_err(|_| WalletError::TxBuild("root-key encryption failed".into()))?,
        );
        self.db.replace_credentials(
            id,
            previous.as_ref(),
            Some(&replacement),
            PassphraseScheme::Argon2idV2,
            Some(&crate::wallet::chrono_now()),
        )
    }
    pub async fn credential_state(
        &self,
        id: &WalletId,
    ) -> Result<(Option<EncryptedKey>, WalletMeta), WalletError> {
        let meta = self.get_wallet(id).await?;
        Ok((self.db.load_encrypted_key(id)?, meta))
    }

    /// Attach imported credentials without exposing private keys through HTTP.
    pub async fn import_credentials(
        &self,
        id: &WalletId,
        key: Option<&EncryptedKey>,
    ) -> Result<(), WalletError> {
        let (previous, meta) = self.credential_state(id).await?;
        self.db.replace_credentials(
            id,
            previous.as_ref(),
            key,
            key.map_or(meta.passphrase_scheme, scheme),
            None,
        )
    }

    /// The commit hook allows storage failure testing after encryption.
    pub async fn unlock_credentials(
        &self,
        id: &WalletId,
        password: &[u8],
        rng: &mut impl RngCore,
        before_migration_commit: impl FnOnce(&EncryptedKey) -> Result<(), WalletError>,
    ) -> Result<UnlockedRoot, WalletError> {
        self.unlock_credentials_with_backend(
            id,
            password,
            rng,
            |key, password, rng| quetzal_crypto::rootkey::RootKey::encrypt(key, password, rng),
            before_migration_commit,
        )
        .await
    }

    /// Swap only the encryption backend for an explicit failure test, retaining
    /// production authentication, migration, warning and persistence behavior.
    pub async fn unlock_credentials_with_backend<R: RngCore>(
        &self,
        id: &WalletId,
        password: &[u8],
        rng: &mut R,
        encrypt: impl FnOnce(
            &Bip32PrivateKey,
            &[u8],
            &mut R,
        ) -> Result<
            quetzal_crypto::rootkey::RootKey,
            quetzal_crypto::rootkey::RootError,
        >,
        before_migration_commit: impl FnOnce(&EncryptedKey) -> Result<(), WalletError>,
    ) -> Result<UnlockedRoot, WalletError> {
        let (encrypted, meta) = self.credential_state(id).await?;
        let encrypted = encrypted.ok_or_else(|| WalletError::NoRootKey(id.to_hex()))?;
        let key = authenticate(&encrypted, password)?;
        let access = if matches!(encrypted, EncryptedKey::Byron(_)) {
            "v1"
        } else {
            "v2"
        };
        let mut migration_warning = false;
        if access == "v1" && matches!(meta.wallet_type, WalletType::Shelley | WalletType::Shared) {
            let replacement = encrypt(&key, password, rng)
                .map(EncryptedKey::V2)
                .map_err(|_| WalletError::TxBuild("root-key migration encryption failed".into()))
                .and_then(|replacement| {
                    before_migration_commit(&replacement)?;
                    self.db.replace_credentials(
                        id,
                        Some(&encrypted),
                        Some(&replacement),
                        PassphraseScheme::Argon2idV2,
                        None,
                    )
                });
            if let Err(error) = replacement {
                tracing::warn!(wallet=%id, %error, "Root-key migration failed; retaining legacy credentials");
                migration_warning = true;
            }
        }
        Ok(UnlockedRoot {
            key,
            access,
            migration_warning,
        })
    }

    pub(crate) async fn decrypt_root(
        &self,
        id: &WalletId,
        password: &[u8],
    ) -> Result<zeroize::Zeroizing<Vec<u8>>, WalletError> {
        let unlocked = self
            .unlock_credentials(id, password, &mut rand::rngs::OsRng, |_| Ok(()))
            .await?;
        Ok(zeroize::Zeroizing::new(unlocked.key.to_raw_bytes()))
    }

    pub async fn change_credentials(
        &self,
        id: &WalletId,
        old: &[u8],
        new: &[u8],
        rng: &mut impl RngCore,
    ) -> Result<(), WalletError> {
        let (encrypted, meta) = self.credential_state(id).await?;
        let mut encrypted = encrypted.ok_or_else(|| WalletError::NoRootKey(id.to_hex()))?;
        let key = if matches!(encrypted, EncryptedKey::Byron(_))
            && matches!(meta.wallet_type, WalletType::Shelley | WalletType::Shared)
        {
            let unlocked = self.unlock_credentials(id, old, rng, |_| Ok(())).await?;
            encrypted = self
                .db
                .load_encrypted_key(id)?
                .ok_or_else(|| WalletError::NoRootKey(id.to_hex()))?;
            unlocked.key
        } else {
            authenticate(&encrypted, old)?
        };
        let replacement = if matches!(meta.wallet_type, WalletType::Shelley | WalletType::Shared) {
            EncryptedKey::V2(
                quetzal_crypto::rootkey::RootKey::encrypt(&key, new, rng)
                    .map_err(|_| WalletError::TxBuild("root-key encryption failed".into()))?,
            )
        } else {
            let mut salt = [0; 32];
            rand::rngs::OsRng.fill_bytes(&mut salt);
            EncryptedKey::encrypt_legacy(&key, new, LegacyScheme::Pbkdf2, &salt)?
        };
        self.db.replace_credentials(
            id,
            Some(&encrypted),
            Some(&replacement),
            scheme(&replacement),
            Some(&crate::wallet::chrono_now()),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mnemonic_recovery_authenticates_identity_and_persists_only_the_new_credentials() {
        let path = std::env::temp_dir().join(format!("quetzal-recovery-{}", uuid::Uuid::new_v4()));
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        let entropy = [37; 20];
        let factor = [19; 16];
        let wallet = service
            .create_wallet_with_second_factor(
                "recover".into(),
                &entropy,
                b"original-password",
                20,
                pallas_addresses::Network::Testnet,
                &factor,
            )
            .await
            .unwrap();
        let before = service.credential_state(&wallet.id).await.unwrap();
        assert!(matches!(
            service
                .recover_passphrase(&wallet.id, &entropy, &[], b"replacement-password")
                .await,
            Err(WalletError::WrongMnemonic(_))
        ));
        assert_eq!(
            serde_json::to_value(service.credential_state(&wallet.id).await.unwrap().0).unwrap(),
            serde_json::to_value(before.0).unwrap()
        );
        service
            .recover_passphrase(&wallet.id, &entropy, &factor, b"replacement-password")
            .await
            .unwrap();
        assert!(service
            .decrypt_root(&wallet.id, b"original-password")
            .await
            .is_err());
        let recovered = service
            .decrypt_root(&wallet.id, b"replacement-password")
            .await
            .unwrap();
        let expected = quetzal_crypto::derive_icarus_master_key(&entropy, &factor).unwrap();
        assert_eq!(recovered.as_slice(), expected.to_raw_bytes());
        let metadata = service.get_wallet(&wallet.id).await.unwrap();
        assert_eq!(metadata.name, "recover");
        drop(service);
        let reopened = WalletService::new(path.to_str().unwrap()).unwrap();
        assert_eq!(
            reopened
                .decrypt_root(&wallet.id, b"replacement-password")
                .await
                .unwrap()
                .as_slice(),
            expected.to_raw_bytes()
        );
        drop(reopened);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[tokio::test]
    async fn migration_failure_is_durable_and_commit_preserves_a_concurrent_chain_tip() {
        let path = std::env::temp_dir().join(format!("quetzal-migration-{}", uuid::Uuid::new_v4()));
        let service = WalletService::new(path.to_str().unwrap()).unwrap();
        let password = b"public-migration-test-password";
        let wallet = service
            .create_wallet(
                "migration".into(),
                &[60; 20],
                password,
                20,
                pallas_addresses::Network::Testnet,
            )
            .await
            .unwrap();
        let key = quetzal_crypto::derive_icarus_master_key(&[60; 20], b"").unwrap();
        let legacy =
            EncryptedKey::encrypt_legacy(&key, password, LegacyScheme::Pbkdf2, &[0; 32]).unwrap();
        service
            .import_credentials(&wallet.id, Some(&legacy))
            .await
            .unwrap();
        let before = service.credential_state(&wallet.id).await.unwrap();
        let failed = service
            .unlock_credentials(&wallet.id, password, &mut rand::rngs::OsRng, |_| {
                Err(WalletError::TxBuild("backend unavailable".into()))
            })
            .await
            .unwrap();
        assert_eq!(failed.access, "v1");
        assert!(failed.migration_warning);
        let after = service.credential_state(&wallet.id).await.unwrap();
        assert_eq!(
            quetzal_primitives::cbor_encode(&before.0).unwrap(),
            quetzal_primitives::cbor_encode(&after.0).unwrap()
        );
        assert_eq!(before.1.passphrase_scheme, after.1.passphrase_scheme);
        assert_eq!(
            before.1.passphrase_last_updated_at,
            after.1.passphrase_last_updated_at
        );
        let success = service
            .unlock_credentials(&wallet.id, password, &mut rand::rngs::OsRng, |_| {
                service.db.update_wallet_tip(&wallet.id, 99, 42, true)
            })
            .await
            .unwrap();
        assert!(!success.migration_warning);
        let migrated = service.credential_state(&wallet.id).await.unwrap();
        assert!(matches!(migrated.0, Some(EncryptedKey::V2(_))));
        assert_eq!(migrated.1.tip_slot, 99);
        assert_eq!(migrated.1.tip_height, 42);
        assert_eq!(
            before.1.passphrase_last_updated_at,
            migrated.1.passphrase_last_updated_at
        );
        assert!(matches!(
            service
                .update_passphrase(&wallet.id, b"incorrect-password", password)
                .await,
            Err(WalletError::WrongPassphrase)
        ));
        drop(service);
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
        let reopened = loop {
            match WalletService::new(path.to_str().unwrap()) {
                Ok(service) => break service,
                Err(WalletError::Db(error))
                    if error.to_string().contains("could not acquire lock")
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await
                }
                Err(error) => panic!("migration fixture reopen failed: {error}"),
            }
        };
        let persisted = reopened.credential_state(&wallet.id).await.unwrap();
        assert_eq!(
            quetzal_primitives::cbor_encode(&migrated.0).unwrap(),
            quetzal_primitives::cbor_encode(&persisted.0).unwrap()
        );
        assert_eq!(persisted.1.tip_slot, 99);
        assert_eq!(
            before.1.passphrase_last_updated_at,
            persisted.1.passphrase_last_updated_at
        );
        drop(reopened);
        std::fs::remove_dir_all(path).unwrap();
    }
}
