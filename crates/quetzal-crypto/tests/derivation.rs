use quetzal_crypto::{
    derive_icarus_master_key, shared_account_key, shelley_account_key, shelley_address_key,
};

#[test]
fn icarus_master_key_deterministic() {
    let entropy = [0x01u8; 32];
    let k1 = derive_icarus_master_key(&entropy, b"").expect("key1");
    let k2 = derive_icarus_master_key(&entropy, b"").expect("key2");
    assert_eq!(
        k1.to_raw_bytes(),
        k2.to_raw_bytes(),
        "derivation must be deterministic"
    );
}

#[test]
fn different_entropy_different_key() {
    let e1 = [0x01u8; 32];
    let e2 = [0x02u8; 32];
    let k1 = derive_icarus_master_key(&e1, b"").expect("k1");
    let k2 = derive_icarus_master_key(&e2, b"").expect("k2");
    assert_ne!(k1.to_raw_bytes(), k2.to_raw_bytes());
}

#[test]
fn account_key_from_root() {
    let entropy = [0xABu8; 32];
    let root = derive_icarus_master_key(&entropy, b"").expect("root");
    let acct0 = shelley_account_key(&root, 0);
    let acct1 = shelley_account_key(&root, 1);
    // Different account indices produce different keys
    assert_ne!(acct0.to_raw_bytes(), acct1.to_raw_bytes());
}

#[test]
fn child_key_derivation() {
    let entropy = [0x77u8; 32];
    let root = derive_icarus_master_key(&entropy, b"").expect("root");
    let account = shelley_account_key(&root, 0);
    // External chain: role=0
    let k0 = shelley_address_key(&account, 0, 0);
    let k1 = shelley_address_key(&account, 0, 1);
    assert_ne!(
        k0.to_raw_bytes(),
        k1.to_raw_bytes(),
        "different indices → different keys"
    );
}

#[test]
fn public_key_length() {
    let entropy = [0x33u8; 32];
    let root = derive_icarus_master_key(&entropy, b"").expect("root");
    let pub_key = root.public_key();
    assert_eq!(
        pub_key.to_raw_bytes().len(),
        64,
        "extended public key is 64 bytes"
    );
    assert_eq!(
        pub_key.to_vk_bytes().len(),
        32,
        "verification key is 32 bytes"
    );
}

#[test]
fn cip1854_shared_key_differs_from_cip1852() {
    let entropy = [0x55u8; 32];
    let root = derive_icarus_master_key(&entropy, b"").expect("root");
    let shelley_acct = shelley_account_key(&root, 0);
    let shared_acct = shared_account_key(&root, 0);
    assert_ne!(
        shelley_acct.public_key().to_raw_bytes(),
        shared_acct.public_key().to_raw_bytes(),
        "CIP-1854 and CIP-1852 account keys must differ",
    );
}

#[test]
fn cip1854_account_index_deterministic() {
    let entropy = [0x42u8; 32];
    let root = derive_icarus_master_key(&entropy, b"").expect("root");
    let k1 = shared_account_key(&root, 0);
    let k2 = shared_account_key(&root, 0);
    assert_eq!(
        k1.public_key().to_raw_bytes(),
        k2.public_key().to_raw_bytes()
    );
}
