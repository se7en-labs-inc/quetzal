use quetzal_primitives::*;

#[test]
fn coin_json_roundtrip() {
    let c = Coin(12345678);
    let s = serde_json::to_string(&c).unwrap();
    assert_eq!(s, "\"12345678\"");
    let back: Coin = serde_json::from_str(&s).unwrap();
    assert_eq!(back, c);
}

#[test]
fn coin_checked_add_overflow() {
    let max = Coin(u64::MAX);
    assert!(max.checked_add(Coin(1)).is_none());
}

#[test]
fn coin_checked_sub_underflow() {
    let small = Coin(5);
    assert!(small.checked_sub(Coin(10)).is_none());
}

#[test]
fn wallet_id_hex_roundtrip() {
    let wid = WalletId([42u8; 20]);
    let hex = wid.to_hex();
    let back = WalletId::from_hex(&hex).unwrap();
    assert_eq!(wid, back);
}

#[test]
fn tx_hash_hex_roundtrip() {
    let h = TxHash([0xABu8; 32]);
    let hex = h.to_hex();
    let back = TxHash::from_hex(&hex).unwrap();
    assert_eq!(h, back);
}

#[test]
fn checkpoint_default_is_origin() {
    let cp = Checkpoint::origin();
    assert_eq!(cp.slot, 0);
    assert_eq!(cp.block_hash, [0u8; 32]);
}

#[test]
fn slot_key_ordering() {
    let k1 = slot_key(100);
    let k2 = slot_key(200);
    assert!(k1 < k2, "slot keys must be lexicographically ordered");
}

#[test]
fn tx_history_key_ordering() {
    let hash = TxHash([1u8; 32]);
    let k_early = tx_history_key(100, &hash);
    let k_late = tx_history_key(200, &hash);
    assert!(k_early < k_late);
}

#[test]
fn utxo_key_different_outputs() {
    let hash = TxHash([5u8; 32]);
    let k0 = utxo_key(&hash, 0);
    let k1 = utxo_key(&hash, 1);
    assert_ne!(k0, k1);
}

#[test]
fn value_pure_ada() {
    let v = Value::lovelace(Coin(1_000_000));
    assert!(v.is_pure_ada());
}

#[test]
fn cbor_roundtrip_wallet_meta() {
    let meta = WalletMeta {
        id: WalletId([0u8; 20]),
        name: "Test Wallet".to_string(),
        account_index: 0,
        address_pool_gap: 20,
        status: WalletStatus::Ready,
        passphrase_scheme: PassphraseScheme::Argon2idV2,
        passphrase_last_updated_at: Some("2026-01-01T00:00:00Z".to_string()),
        tip_slot: 12345,
        tip_height: 100,
        created_at: "2026-01-01T00:00:00Z".to_string(),
        balance_total: Coin(5_000_000),
        balance_available: Coin(4_000_000),
        balance_reward: Coin(0),
        assets_available: vec![],
        assets_total: vec![],
        delegation_status: DelegationStatus::NotDelegating,
        stake_pool_id: None,
        reward_account_balance: Coin(0),
        wallet_type: WalletType::Shelley,
    };
    let bytes = cbor_encode(&meta).unwrap();
    let back: WalletMeta = cbor_decode(&bytes).unwrap();
    assert_eq!(back.name, meta.name);
    assert_eq!(back.tip_slot, meta.tip_slot);
    assert_eq!(back.balance_total, meta.balance_total);
}

mod prop {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn coin_add_sub_inverse(a in 0u64..u64::MAX/2, b in 0u64..u64::MAX/2) {
            let ca = Coin(a);
            let cb = Coin(b);
            if let Some(sum) = ca.checked_add(cb) {
                let back = sum.checked_sub(cb).unwrap();
                prop_assert_eq!(back, ca);
            }
        }
    }
}
