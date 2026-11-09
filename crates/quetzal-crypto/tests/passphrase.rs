use quetzal_crypto::EncryptedKey;

#[test]
fn byron_protection_survives_storage_and_authenticates_the_password() {
    let key = quetzal_crypto::derivation::derive_byron_master_key(&[0; 16]).unwrap();
    let protected =
        EncryptedKey::encrypt_byron(&key.to_raw_bytes(), b"public-test-passphrase").unwrap();
    let serialized = quetzal_primitives::cbor_encode(&protected).unwrap();
    let restored: EncryptedKey = quetzal_primitives::cbor_decode(&serialized).unwrap();
    assert!(matches!(restored, EncryptedKey::Byron(_)));
    assert_eq!(
        restored
            .decrypt(b"public-test-passphrase")
            .unwrap()
            .as_slice(),
        key.to_raw_bytes().as_slice()
    );
    assert!(matches!(
        restored.decrypt(b"wrong-password"),
        Err(quetzal_crypto::CryptoError::InvalidPassphrase)
    ));
}

#[test]
fn encrypt_decrypt_roundtrip() {
    let key_bytes = quetzal_crypto::derive_icarus_master_key(&[0xAB; 20], b"")
        .unwrap()
        .to_raw_bytes();
    let passphrase = b"correct-horse-battery-staple";

    let enc = EncryptedKey::encrypt(&key_bytes, passphrase).expect("encrypt");
    let dec = enc.decrypt(passphrase).expect("decrypt");
    assert_eq!(dec.as_slice(), &key_bytes);
}

#[test]
fn wrong_passphrase_returns_error() {
    let key_bytes = quetzal_crypto::derive_icarus_master_key(&[0x42; 20], b"")
        .unwrap()
        .to_raw_bytes();
    let enc = EncryptedKey::encrypt(&key_bytes, b"right-pass").expect("encrypt");
    let result = enc.decrypt(b"wrong-pass");
    assert!(result.is_err(), "wrong passphrase must fail");
    assert!(matches!(
        result.unwrap_err(),
        quetzal_crypto::CryptoError::InvalidPassphrase
    ));
}

#[test]
fn fresh_salt_nonce_each_time() {
    let key_bytes = quetzal_crypto::derive_icarus_master_key(&[0x11; 20], b"")
        .unwrap()
        .to_raw_bytes();
    let pass = b"same-passphrase";
    let enc1 = EncryptedKey::encrypt(&key_bytes, pass).expect("enc1");
    let enc2 = EncryptedKey::encrypt(&key_bytes, pass).expect("enc2");
    // Different salt and nonce every call
    let mut first = pallas_codec::minicbor::Decoder::new(enc1.envelope().unwrap());
    let mut second = pallas_codec::minicbor::Decoder::new(enc2.envelope().unwrap());
    assert_eq!(first.array().unwrap(), Some(5));
    assert_eq!(second.array().unwrap(), Some(5));
    assert_ne!(
        first.bytes().unwrap(),
        second.bytes().unwrap(),
        "salt must be fresh"
    );
    assert_ne!(
        first.bytes().unwrap(),
        second.bytes().unwrap(),
        "nonce must be fresh"
    );
    // But both decrypt to the same key
    let dec1 = enc1.decrypt(pass).unwrap();
    let dec2 = enc2.decrypt(pass).unwrap();
    assert_eq!(dec1.as_slice(), dec2.as_slice());
}
