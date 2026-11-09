use pallas_addresses::Network;
use quetzal_primitives::WalletType;
use quetzal_wallet::{WalletError, WalletService};

#[tokio::test]
async fn identical_family_ids_have_independent_durable_credentials_and_lifecycle() {
    let path = std::env::temp_dir().join(format!("quetzal-family-{}", uuid::Uuid::new_v4()));
    let shelley = WalletService::new(path.to_str().unwrap()).unwrap();
    let byron = shelley.byron_service().unwrap();
    assert_eq!(shelley.pool_metadata_source().unwrap(), "none");
    shelley
        .set_pool_metadata_source("http://127.0.0.1:12345")
        .unwrap();
    let password = b"public-family-password";
    let first = shelley
        .create_wallet("shelley".into(), &[47; 20], password, 20, Network::Testnet)
        .await
        .unwrap();
    let second = byron
        .create_byron_wallet_style("icarus".into(), &[47; 20], password, Some(42), "icarus")
        .await
        .unwrap();
    assert_eq!(first.id, second.id);
    let id = first.id;
    assert_eq!(
        shelley.get_wallet(&id).await.unwrap().wallet_type,
        WalletType::Shelley
    );
    assert_eq!(
        byron.get_wallet(&id).await.unwrap().wallet_type,
        WalletType::Icarus
    );
    byron
        .update_passphrase(&id, password, b"new-family-password")
        .await
        .unwrap();
    shelley
        .update_passphrase(&id, password, b"shelley-family-password")
        .await
        .unwrap();
    byron
        .update_passphrase(&id, b"new-family-password", password)
        .await
        .unwrap();
    byron.delete_wallet(&id).await.unwrap();
    assert!(byron.get_wallet(&id).await.is_err());
    assert_eq!(shelley.get_wallet(&id).await.unwrap().name, "shelley");
    drop(byron);
    drop(shelley);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    let reopened = loop {
        match WalletService::new(path.to_str().unwrap()) {
            Ok(service) => break service,
            Err(WalletError::Db(error))
                if error.to_string().contains("could not acquire lock")
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            Err(error) => panic!("failed to reopen namespace fixture: {error}"),
        }
    };
    assert_eq!(reopened.get_wallet(&id).await.unwrap().name, "shelley");
    assert_eq!(
        reopened.pool_metadata_source().unwrap(),
        "http://127.0.0.1:12345"
    );
    assert!(reopened
        .byron_service()
        .unwrap()
        .get_wallet(&id)
        .await
        .is_err());
    drop(reopened);
    std::fs::remove_dir_all(path).unwrap();
}
