use pallas_addresses::Network;
use quetzal_primitives::WalletId;
use quetzal_wallet::WalletService;

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[tokio::test]
async fn watch_wallet_reproduces_addresses_without_private_derivation() {
    let path = temp_db_path();
    let service = WalletService::new(&path).unwrap();
    let root = service
        .create_wallet(
            "root".into(),
            &[42; 20],
            b"public-test-passphrase",
            20,
            Network::Testnet,
        )
        .await
        .unwrap();
    let public = service.get_account_xpub_bytes(&root.id).await.unwrap();
    let watch = service
        .create_watch_wallet("watch".into(), &public, 20, Network::Testnet)
        .await
        .unwrap();
    assert_ne!(root.id, watch.id);
    let root_addresses = service.get_addresses(&root.id).await.unwrap();
    let watch_addresses = service.get_addresses(&watch.id).await.unwrap();
    assert_eq!(
        root_addresses
            .iter()
            .map(|entry| entry.address.as_str())
            .collect::<Vec<_>>(),
        watch_addresses
            .iter()
            .map(|entry| entry.address.as_str())
            .collect::<Vec<_>>()
    );
    assert!(watch.passphrase_last_updated_at.is_none());
    assert!(matches!(
        service
            .derive_account_xpub_at(&watch.id, b"public-test-passphrase", 0)
            .await,
        Err(quetzal_wallet::WalletError::NoRootKey(_))
    ));
    service.delete_wallet(&watch.id).await.unwrap();
    service.delete_wallet(&root.id).await.unwrap();
    drop(service);
    std::fs::remove_dir_all(path).unwrap();
}

#[tokio::test]
async fn wallet_identity_matches_published_haskell_root_vector() {
    let path = temp_db_path();
    let service = WalletService::new(&path).unwrap();
    // The independent conformance corpus's 15-word zero-entropy mnemonic.
    let wallet = service
        .create_wallet(
            "identity-vector".into(),
            &[0u8; 20],
            b"public-fixture-passphrase",
            20,
            Network::Testnet,
        )
        .await
        .unwrap();
    assert_eq!(
        wallet.id.to_hex(),
        "2425e88933b3a065ba3b5787d86fba26ad450a9d"
    );
    service.delete_wallet(&wallet.id).await.unwrap();
    drop(service);
    std::fs::remove_dir_all(path).unwrap();
}

fn temp_db_path() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tid = std::thread::current().id();
    format!("/tmp/quetzal-test-{secs}-{n}-{tid:?}.sled")
}

#[tokio::test]
async fn create_and_list_wallet() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x01u8; 32];
    let passphrase = b"test-passphrase-123";

    let meta = svc
        .create_wallet(
            "My Wallet".to_string(),
            &entropy,
            passphrase,
            20,
            Network::Testnet,
        )
        .await
        .expect("create wallet");

    assert_eq!(meta.name, "My Wallet");
    assert_eq!(meta.address_pool_gap, 20);

    let wallets = svc.list_wallets().await.expect("list");
    assert_eq!(wallets.len(), 1);
    assert_eq!(wallets[0].id, meta.id);
}

#[tokio::test]
async fn get_wallet_not_found() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let fake_id = WalletId([0xFFu8; 20]);
    let result = svc.get_wallet(&fake_id).await;
    assert!(matches!(
        result,
        Err(quetzal_wallet::WalletError::NotFound(_))
    ));
}

#[tokio::test]
async fn delete_wallet() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x02u8; 32];
    let meta = svc
        .create_wallet("Temp".to_string(), &entropy, b"pass", 20, Network::Testnet)
        .await
        .expect("create");

    svc.delete_wallet(&meta.id).await.expect("delete");

    let wallets = svc.list_wallets().await.expect("list");
    assert!(wallets.is_empty());
}

#[tokio::test]
async fn update_passphrase_success() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x03u8; 32];
    let old = b"old-pass-word";
    let new = b"new-pass-word";

    let meta = svc
        .create_wallet("PassTest".to_string(), &entropy, old, 20, Network::Testnet)
        .await
        .expect("create");

    svc.update_passphrase(&meta.id, old, new)
        .await
        .expect("update pass");
}

#[tokio::test]
async fn update_passphrase_wrong_old() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x04u8; 32];
    let meta = svc
        .create_wallet(
            "PassTest".to_string(),
            &entropy,
            b"correct",
            20,
            Network::Testnet,
        )
        .await
        .expect("create");

    let result = svc.update_passphrase(&meta.id, b"wrong", b"new").await;
    assert!(matches!(
        result,
        Err(quetzal_wallet::WalletError::WrongPassphrase)
    ));
}

#[tokio::test]
async fn duplicate_wallet_rejected() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x05u8; 32];
    svc.create_wallet("W1".to_string(), &entropy, b"pass", 20, Network::Testnet)
        .await
        .expect("create first");
    let result = svc
        .create_wallet("W2".to_string(), &entropy, b"pass", 20, Network::Testnet)
        .await;
    assert!(matches!(
        result,
        Err(quetzal_wallet::WalletError::AlreadyExists(_))
    ));
}

#[tokio::test]
async fn get_addresses_returns_gap_limit_count() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x06u8; 32];
    let meta = svc
        .create_wallet(
            "AddrTest".to_string(),
            &entropy,
            b"pass",
            5,
            Network::Testnet,
        )
        .await
        .expect("create");

    let addrs = svc.get_addresses(&meta.id).await.expect("addresses");
    // 5 external + 5 internal = 10 addresses
    assert_eq!(addrs.len(), 10, "should have gap_limit*2 addresses");
}

#[tokio::test]
async fn account_xpub_stored_at_creation() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x08u8; 32];
    let meta = svc
        .create_wallet(
            "XpubTest".to_string(),
            &entropy,
            b"pass",
            5,
            Network::Testnet,
        )
        .await
        .expect("create");

    let xpub = svc
        .get_account_xpub_bytes(&meta.id)
        .await
        .expect("get account xpub");
    assert_eq!(xpub.len(), 64, "account xpub must be 64 bytes");
}

#[tokio::test]
async fn derive_account_xpub_at_requires_correct_passphrase() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x09u8; 32];
    let meta = svc
        .create_wallet(
            "DeriveTest".to_string(),
            &entropy,
            b"secret",
            5,
            Network::Testnet,
        )
        .await
        .expect("create");

    // Correct passphrase: success
    svc.derive_account_xpub_at(&meta.id, b"secret", 0)
        .await
        .expect("derive with correct passphrase");

    // Wrong passphrase: WrongPassphrase error
    let err = svc.derive_account_xpub_at(&meta.id, b"wrong", 0).await;
    assert!(matches!(
        err,
        Err(quetzal_wallet::WalletError::WrongPassphrase)
    ));
}

#[tokio::test]
async fn initial_balance_is_zero() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0x07u8; 32];
    let meta = svc
        .create_wallet(
            "BalTest".to_string(),
            &entropy,
            b"pass",
            5,
            Network::Testnet,
        )
        .await
        .expect("create");

    let (available, total) = svc.get_balance(&meta.id).await.expect("balance");
    use quetzal_primitives::Coin;
    assert_eq!(available, Coin::ZERO);
    assert_eq!(total, Coin::ZERO);
}

#[tokio::test]
async fn create_shared_wallet_single_cosigner_self() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0xC1u8; 32];

    let payment_template = serde_json::json!({
        "cosigners": { "cosigner#0": "self" },
        "template": { "all": ["cosigner#0"] }
    });

    let mut cosigners = std::collections::HashMap::new();
    cosigners.insert("cosigner#0".to_string(), None); // "self"

    let meta = svc
        .create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
            name: "Shared Wallet".to_string(),
            entropy: &entropy,
            passphrase: b"passphrase123",
            account_ix: 0,
            cosigners,
            payment_template: &payment_template,
            delegation_template: None,
            network: Network::Testnet,
        })
        .await
        .expect("create shared wallet");

    assert_eq!(meta.name, "Shared Wallet");
    assert_eq!(meta.wallet_type, quetzal_primitives::WalletType::Shared);

    // Verify we can retrieve it
    let fetched = svc
        .get_shared_wallet(&meta.id)
        .await
        .expect("get shared wallet");
    assert_eq!(fetched.id, meta.id);

    // Addresses should be derived (20 external + 20 internal)
    let addrs = svc.get_addresses(&meta.id).await.expect("addresses");
    assert_eq!(addrs.len(), 40);

    // Addresses are script-based (addr1z... on testnet)
    for addr in &addrs {
        let s = addr.address.as_str();
        assert!(s.starts_with("addr"), "expected addr prefix, got: {s}");
    }

    // Payment script template is stored and retrievable
    let stored = svc
        .get_script_template(&meta.id, b"payment_script")
        .await
        .expect("script template");
    assert!(stored.is_some());
}

#[tokio::test]
async fn shared_wallet_deterministic_id() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0xD2u8; 32];

    let payment_template = serde_json::json!({
        "cosigners": { "cosigner#0": "self" },
        "template": { "all": ["cosigner#0"] }
    });
    let mut cosigners = std::collections::HashMap::new();
    cosigners.insert("cosigner#0".to_string(), None);

    let _meta1 = svc
        .create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
            name: "W1".to_string(),
            entropy: &entropy,
            passphrase: b"pass",
            account_ix: 0,
            cosigners: cosigners.clone(),
            payment_template: &payment_template,
            delegation_template: None,
            network: Network::Testnet,
        })
        .await
        .expect("create 1");

    // Same entropy + same template → AlreadyExists
    let result = svc
        .create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
            name: "W2".to_string(),
            entropy: &entropy,
            passphrase: b"pass",
            account_ix: 0,
            cosigners,
            payment_template: &payment_template,
            delegation_template: None,
            network: Network::Testnet,
        })
        .await;

    assert!(matches!(
        result,
        Err(quetzal_wallet::WalletError::AlreadyExists(_))
    ));
}

#[tokio::test]
async fn delete_shared_wallet() {
    let path = temp_db_path();
    let svc = WalletService::new(&path).expect("open db");
    let entropy = [0xE3u8; 32];

    let payment_template = serde_json::json!({
        "cosigners": { "cosigner#0": "self" },
        "template": { "all": ["cosigner#0"] }
    });
    let mut cosigners = std::collections::HashMap::new();
    cosigners.insert("cosigner#0".to_string(), None);

    let meta = svc
        .create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
            name: "Del Shared".to_string(),
            entropy: &entropy,
            passphrase: b"pass",
            account_ix: 0,
            cosigners,
            payment_template: &payment_template,
            delegation_template: None,
            network: Network::Testnet,
        })
        .await
        .expect("create");

    svc.delete_shared_wallet(&meta.id).await.expect("delete");
    let result = svc.get_shared_wallet(&meta.id).await;
    assert!(matches!(
        result,
        Err(quetzal_wallet::WalletError::NotFound(_))
    ));
}
