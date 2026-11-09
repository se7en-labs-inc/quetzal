//! JSON-line testing seam; expected answers remain in the independent harness.
use std::io::{self, BufRead, Read};

use bip39::Mnemonic;
use quetzal_crypto::{derive_icarus_master_key, Bip32PrivateKey, Bip32PublicKey};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct Request {
    protocol_version: u32,
    id: String,
    operation: String,
    input: Value,
}

fn bytes(input: &Value, field: &str) -> Result<Vec<u8>, &'static str> {
    hex::decode(input[field].as_str().ok_or("invalid_input")?).map_err(|_| "invalid_input")
}

fn text<'a>(input: &'a Value, field: &str) -> Result<&'a str, &'static str> {
    input[field].as_str().ok_or("invalid_input")
}

fn path(input: &Value) -> Result<Vec<u32>, &'static str> {
    input["path"]
        .as_array()
        .ok_or("invalid_input")?
        .iter()
        .map(|index| {
            index
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("invalid_input")
        })
        .collect()
}

fn execute(request: &Request) -> Result<Value, &'static str> {
    if request.protocol_version != 1 {
        return Err("unsupported_protocol");
    }
    let input = &request.input;
    if request.operation.starts_with("witness.") || request.operation == "wallet.certificates" {
        return quetzal_wallet::writer::execute(&request.operation, input);
    }
    if request.operation.starts_with("root-key.") {
        return execute_root(&request.operation, input);
    }
    if request.operation == "wallet.credentials" {
        return execute_credentials(input);
    }
    match request.operation.as_str() {
        "sha256" => Ok(
            json!({"hex": hex::encode(quetzal_crypto::hashing::sha256(&bytes(input, "message_hex")?))}),
        ),
        "sha512" => Ok(
            json!({"hex": hex::encode(quetzal_crypto::hashing::sha512(&bytes(input, "message_hex")?))}),
        ),
        "blake2b" => {
            let size = input["output_bytes"]
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("invalid_input")?;
            let digest = quetzal_crypto::hashing::blake2b(&bytes(input, "message_hex")?, size)
                .map_err(|_| "invalid_input")?;
            Ok(json!({"hex": hex::encode(digest)}))
        }
        "ed25519.sign" => {
            let seed = bytes(input, "seed_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let (public, signature) =
                quetzal_crypto::signing::sign_seed(&seed, &bytes(input, "message_hex")?);
            Ok(json!({"public_key_hex":hex::encode(public),"signature_hex":hex::encode(signature)}))
        }
        "keys.verify-extended" => {
            let public = Bip32PublicKey::from_raw_bytes(&bytes(input, "xpub_hex")?)
                .map_err(|_| "invalid_input")?;
            let signature = bytes(input, "signature_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            Ok(
                json!({"valid":quetzal_crypto::signing::verify_extended(&public.to_vk_bytes(), &bytes(input, "message_hex")?, &signature)}),
            )
        }
        "passphrase.hash-pbkdf2" => {
            let hash = quetzal_crypto::password::hash_pbkdf2(
                text(input, "passphrase_utf8")?.as_bytes(),
                &bytes(input, "salt_hex")?,
            )
            .map_err(|_| "invalid_input")?;
            Ok(json!({"hash_hex":hex::encode(hash)}))
        }
        "passphrase.check-pbkdf2" => Ok(
            json!({"valid":quetzal_crypto::password::check_pbkdf2(text(input,"passphrase_utf8")?.as_bytes(), &bytes(input,"hash_hex")?)}),
        ),
        "passphrase.check-scrypt" => Ok(
            json!({"valid":quetzal_crypto::password::check_scrypt(text(input,"passphrase_utf8")?.as_bytes(), &bytes(input,"hash_hex")?)}),
        ),
        "transaction.minimum-fee" => {
            let request: quetzal_wallet::ledger_fee::MinimumFeeRequest =
                serde_json::from_value(input.clone()).map_err(|_| "invalid_input")?;
            serde_json::to_value(request.calculate()?).map_err(|_| "invalid_input")
        }
        "transaction.size-estimate" => {
            let result = quetzal_wallet::txsize::estimate_signed(
                &bytes(input, "transaction_cbor_hex")?,
                input["vkey_witnesses"].as_u64().ok_or("invalid_input")?,
                input["bootstrap_witnesses"]
                    .as_u64()
                    .ok_or("invalid_input")?,
                input["maximum_transaction_bytes"]
                    .as_u64()
                    .ok_or("invalid_input")?,
                input["protocol_major"].as_u64().ok_or("invalid_input")?,
            )?;
            serde_json::to_value(result).map_err(|_| "invalid_input")
        }
        "transaction.input-witness-count" => {
            use pallas_primitives::Fragment;
            let encoded = bytes(input, "transaction_cbor_hex")?;
            let tx = pallas_primitives::conway::Tx::decode_fragment(&encoded)
                .map_err(|_| "invalid_input")?;
            let multi =
                pallas_traverse::MultiEraTx::decode(&encoded).map_err(|_| "invalid_input")?;
            let outputs = multi.outputs();
            let output = outputs.first().ok_or("invalid_input")?;
            let address = output.address().map_err(|_| "invalid_input")?.to_string();
            let utxos = tx
                .transaction_body
                .inputs
                .iter()
                .map(|input| quetzal_primitives::UtxoEntry {
                    id: quetzal_primitives::UtxoId::new(
                        quetzal_primitives::TxHash(*input.transaction_id),
                        input.index as u32,
                    ),
                    address: quetzal_primitives::Address(address.clone()),
                    value: quetzal_primitives::Value::lovelace(quetzal_primitives::Coin(
                        output.value().coin(),
                    )),
                })
                .collect::<Vec<_>>();
            serde_json::to_value(
                quetzal_wallet::txsize::input_witness_counts(&tx, &utxos)
                    .map_err(|_| "invalid_input")?,
            )
            .map_err(|_| "invalid_input")
        }
        "minimum-utxo" => {
            let result = quetzal_wallet::minimum_coin::minimum_coin(
                &bytes(input, "output_cbor_hex")?,
                input["coins_per_byte"].as_u64().ok_or("invalid_input")?,
            )?;
            serde_json::to_value(result).map_err(|_| "invalid_input")
        }
        "fee.components" => {
            let request: quetzal_wallet::fees::FeeRequest =
                serde_json::from_value(input.clone()).map_err(|_| "invalid_input")?;
            serde_json::to_value(request.calculate().map_err(|_| "invalid_input")?)
                .map_err(|_| "invalid_input")
        }
        "metadata.encode" => {
            let _version = input["ledger_version"].as_u64().ok_or("invalid_input")?;
            let entries =
                serde_json::from_value(input["entries"].clone()).map_err(|_| "invalid_input")?;
            quetzal_wallet::metadatum::encode_entries(entries).map_err(|_| "invalid_input")
        }
        "metadata.encrypt" | "metadata.decrypt" => {
            let salt = if request.operation == "metadata.encrypt" {
                Some(
                    bytes(input, "salt_hex")?
                        .try_into()
                        .map_err(|_| "invalid_input")?,
                )
            } else {
                None
            };
            quetzal_wallet::metadata_encryption::transform(
                &input["entries"],
                &bytes(input, "passphrase_hex")?,
                salt,
            )
        }
        "transaction.roundtrip" => {
            let (encoded, id) =
                quetzal_wallet::ledger::roundtrip(text(input, "era")?, &bytes(input, "cbor_hex")?)?;
            Ok(json!({"cbor_hex":hex::encode(encoded),"transaction_id":hex::encode(id)}))
        }
        "ledger.ed25519.verify" => {
            let public = bytes(input, "public_key_hex")?;
            let signature = bytes(input, "signature_hex")?;
            let message = bytes(input, "message_hex")?;
            let valid = match (public.try_into(), signature.try_into()) {
                (Ok(public), Ok(signature)) => {
                    quetzal_crypto::verify(&public, &message, &signature)
                }
                _ => false,
            };
            Ok(json!({"valid":valid}))
        }
        "time.slot-boundary" => {
            let request: quetzal_primitives::slotting::SlotBoundaryRequest =
                serde_json::from_value(input.clone()).map_err(|_| "invalid_input")?;
            serde_json::to_value(request.boundary().ok_or("invalid_input")?)
                .map_err(|_| "invalid_input")
        }
        "coin-selection.select" => {
            let request = serde_json::from_value(input.clone()).map_err(|_| "invalid_input")?;
            let selected =
                quetzal_wallet::selection::select(&request).map_err(|error| match error {
                    quetzal_wallet::WalletError::InsufficientFunds => "not_enough_money",
                    _ => "invalid_input",
                })?;
            serde_json::to_value(selected).map_err(|_| "invalid_input")
        }
        "byron.path-encrypt" => {
            let key = bytes(input, "passphrase_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let account = input["account_index"]
                .as_u64()
                .ok_or("invalid_input")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let index = input["address_index"]
                .as_u64()
                .ok_or("invalid_input")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            Ok(
                json!({"attributes_hex":hex::encode(quetzal_address::byron::encrypt_attributes(&key,account,index).map_err(|_|"invalid_input")?)}),
            )
        }
        "byron.path-decrypt" => {
            let key = bytes(input, "passphrase_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            Ok(
                json!({"path":quetzal_address::byron::decrypt_attributes(&key,&bytes(input,"attributes_hex")?).map_err(|_|"invalid_input")?}),
            )
        }
        "address.inspect-byron" => {
            let random = match text(input, "style")? {
                "Byron" => true,
                "Icarus" => false,
                _ => return Err("invalid_input"),
            };
            quetzal_address::byron::inspect(&bytes(input, "address_hex")?, random)
                .map_err(|_| "invalid_input")?;
            Ok(json!({"valid":true}))
        }
        "address.byron" => {
            let random = match text(input, "style")? {
                "Byron" => true,
                "Icarus" => false,
                _ => return Err("invalid_input"),
            };
            let entropy = Mnemonic::parse_in(bip39::Language::English, text(input, "mnemonic")?)
                .map_err(|_| "invalid_input")?
                .to_entropy();
            let root = if random {
                quetzal_crypto::derivation::derive_byron_master_key(&entropy)
            } else {
                derive_icarus_master_key(&entropy, b"")
            }
            .map_err(|_| "invalid_input")?;
            let account = input["account_index"]
                .as_u64()
                .ok_or("invalid_input")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let index = input["address_index"]
                .as_u64()
                .ok_or("invalid_input")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let magic = match text(input, "network")? {
                "mainnet" => None,
                "preprod" => Some(1),
                _ => return Err("invalid_input"),
            };
            let address = quetzal_address::byron::address(&root, random, account, index, magic)
                .map_err(|_| "invalid_input")?;
            Ok(json!({"address":address.to_base58(),"address_hex":address.to_hex()}))
        }
        "native.script" => {
            let script: quetzal_address::script::NativeScript =
                serde_json::from_value(input["script"].clone()).map_err(|_| "invalid_input")?;
            let encoded = script.serialize().map_err(|_| "invalid_input")?;
            let signers = input["signers"]
                .as_array()
                .ok_or("invalid_input")?
                .iter()
                .map(|signer| {
                    hex::decode(signer.as_str().ok_or("invalid_input")?)
                        .map_err(|_| "invalid_input")?
                        .try_into()
                        .map_err(|_| "invalid_input")
                })
                .collect::<Result<std::collections::BTreeSet<[u8; 28]>, _>>()?;
            Ok(
                json!({"cbor_hex":hex::encode(encoded),"hash_hex":hex::encode(script.hash().map_err(|_|"invalid_input")?),"valid":script.valid(),"satisfied":script.satisfied(&signers,input["invalid_before"].as_u64(),input["invalid_hereafter"].as_u64())}),
            )
        }
        "address.build" => {
            let kind: quetzal_address::codec::AddressKind =
                serde_json::from_value(input.clone()).map_err(|_| "invalid_input")?;
            let network: u8 = input["network_tag"]
                .as_u64()
                .ok_or("invalid_input")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let (address, raw) =
                quetzal_address::codec::address(&kind, network).map_err(|_| "invalid_input")?;
            Ok(json!({"address":address,"address_hex":hex::encode(raw)}))
        }
        "encoding.bech32" => match text(input, "action")? {
            "encode" => Ok(
                json!({"text":quetzal_address::codec::bech32_encode(text(input,"hrp")?,&bytes(input,"bytes_hex")?).map_err(|_|"invalid_input")?}),
            ),
            "decode" => {
                let (hrp, raw) = quetzal_address::codec::bech32_decode(text(input, "text")?)
                    .map_err(|_| "invalid_input")?;
                Ok(json!({"hrp":hrp,"bytes_hex":hex::encode(raw)}))
            }
            _ => Err("invalid_input"),
        },
        "encoding.base58" => match text(input, "action")? {
            "encode" => Ok(
                json!({"text":quetzal_address::codec::base58_encode(&bytes(input,"bytes_hex")?)}),
            ),
            "decode" => Ok(
                json!({"bytes_hex":hex::encode(quetzal_address::codec::base58_decode(text(input,"text")?).map_err(|_|"invalid_input")?)}),
            ),
            _ => Err("invalid_input"),
        },
        "key.hash" => {
            let public = bytes(input, "public_key_hex")?;
            if public.len() != 32 {
                return Err("invalid_input");
            }
            Ok(json!({"hash_hex":hex::encode(pallas_crypto::hash::Hasher::<224>::hash(&public))}))
        }
        "asset.fingerprint" => Ok(
            json!({"fingerprint":quetzal_address::codec::fingerprint(&bytes(input,"policy_hex")?,&bytes(input,"asset_name_hex")?).map_err(|_|"invalid_input")?}),
        ),
        "governance.identifier" => {
            let role = text(input, "role")?;
            if !["drep", "committee_hot", "committee_cold"].contains(&role) {
                return Err("invalid_input");
            }
            let script = match text(input, "kind")? {
                "key" => false,
                "script" => true,
                _ => return Err("invalid_input"),
            };
            match text(input, "action")? {
                "encode" => Ok(
                    json!({"text":quetzal_address::codec::governance_encode(role,script,&bytes(input,"hash_hex")?).map_err(|_|"invalid_input")?}),
                ),
                "decode" => Ok(
                    json!({"hash_hex":hex::encode(quetzal_address::codec::governance_decode(text(input,"text")?,script).map_err(|_|"invalid_input")?)}),
                ),
                _ => Err("invalid_input"),
            }
        }
        "address.enterprise" => {
            let vk = bytes(input, "public_key_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let network = match input["network_tag"].as_u64() {
                Some(0) => pallas_addresses::Network::Testnet,
                Some(1) => pallas_addresses::Network::Mainnet,
                _ => return Err("invalid_input"),
            };
            let address = quetzal_address::shelley::enterprise_from_public_key(&vk, network)
                .map_err(|_| "invalid_input")?;
            let raw = pallas_addresses::Address::from_bech32(&address.0)
                .map_err(|_| "invalid_input")?
                .to_vec();
            Ok(json!({"address": address.0, "address_hex": hex::encode(raw)}))
        }
        "keys.derive" => {
            let style = text(input, "style")?;
            match style {
                "Shelley" | "Icarus" | "Shared" | "Byron" => {}
                _ => return Err("invalid_input"),
            }
            let path = path(input)?;
            if !quetzal_crypto::derivation::valid_wallet_path(style, &path) {
                return Err("invalid_input");
            }
            let mnemonic =
                Mnemonic::parse(text(input, "mnemonic")?).map_err(|_| "invalid_input")?;
            let mut key = if style == "Byron" {
                quetzal_crypto::derivation::derive_byron_master_key(&mnemonic.to_entropy())
            } else {
                derive_icarus_master_key(
                    &mnemonic.to_entropy(),
                    &bytes(input, "second_factor_hex")?,
                )
            }
            .map_err(|_| "invalid_input")?;
            for index in path {
                key = if style == "Byron" {
                    key.derive_byron(index)
                } else {
                    key.derive(index)
                };
            }
            Ok(
                json!({"xprv_hex": hex::encode(key.to_raw_bytes()), "xpub_hex": hex::encode(key.public_key().to_raw_bytes())}),
            )
        }
        "keys.derive-public" => {
            let style = text(input, "style")?;
            match style {
                "Shelley" | "Icarus" | "Shared" | "Byron" => {}
                _ => return Err("invalid_input"),
            }
            let path = path(input)?;
            if path.len() > 5 {
                return Err("invalid_input");
            }
            let mut key = Bip32PublicKey::from_raw_bytes(&bytes(input, "parent_xpub_hex")?)
                .map_err(|_| "invalid_input")?;
            for index in path {
                key = if style == "Byron" {
                    key.derive_byron(index)
                } else {
                    key.derive(index)
                }
                .map_err(|_| "invalid_input")?;
            }
            Ok(json!({"xpub_hex": hex::encode(key.to_raw_bytes())}))
        }
        "keys.sign-extended" => {
            let key = Bip32PrivateKey::from_raw_bytes(&bytes(input, "xprv_hex")?)
                .map_err(|_| "invalid_input")?;
            let signature = quetzal_crypto::sign(&key, &bytes(input, "message_hex")?);
            Ok(
                json!({"public_key_hex": hex::encode(key.public_key().to_vk_bytes()), "signature_hex": hex::encode(signature)}),
            )
        }
        "ed25519.verify" => {
            let public_key = bytes(input, "public_key_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            let signature = bytes(input, "signature_hex")?
                .try_into()
                .map_err(|_| "invalid_input")?;
            Ok(
                json!({"valid": quetzal_crypto::verify(&public_key, &bytes(input, "message_hex")?, &signature)}),
            )
        }
        "bip39.entropy-to-mnemonic" => {
            let mnemonic = Mnemonic::from_entropy(&bytes(input, "entropy_hex")?)
                .map_err(|_| "invalid_input")?;
            Ok(json!({"mnemonic": mnemonic.to_string()}))
        }
        "bip39.to-seed" => {
            let mnemonic =
                Mnemonic::parse(text(input, "mnemonic")?).map_err(|_| "invalid_input")?;
            Ok(json!({"seed_hex": hex::encode(mnemonic.to_seed(text(input, "passphrase")?))}))
        }
        "bip39.wallet-seed" => {
            let mnemonic =
                Mnemonic::parse(text(input, "mnemonic")?).map_err(|_| "invalid_input")?;
            let seed = quetzal_crypto::derivation::raw_mnemonic_seed(
                &mnemonic.to_string(),
                text(input, "passphrase")?,
            )
            .map_err(|_| "invalid_input")?;
            Ok(json!({"seed_hex":hex::encode(seed.as_ref())}))
        }
        "byron.generate-root" => {
            let words = input["mnemonic"]
                .as_array()
                .ok_or("invalid_input")?
                .iter()
                .map(|w| w.as_str().ok_or("invalid_input"))
                .collect::<Result<Vec<_>, _>>()?;
            let mnemonic = Mnemonic::parse(words.join(" ")).map_err(|_| "invalid_input")?;
            let key = quetzal_crypto::derivation::derive_byron_master_key(&mnemonic.to_entropy())
                .map_err(|_| "invalid_input")?;
            Ok(
                json!({"xprv_hex":hex::encode(quetzal_crypto::password::legacy_serialization(&key, &bytes(input,"encryption_passphrase_hex")?).map_err(|_| "invalid_input")?)}),
            )
        }
        "keys.generate-root-profile" => {
            let mnemonic =
                Mnemonic::parse(text(input, "mnemonic")?).map_err(|_| "invalid_input")?;
            let style = text(input, "style")?;
            let factor = bytes(input, "second_factor_hex")?;
            let path = path(input)?;
            if path.len() > 5 {
                return Err("invalid_input");
            }
            if matches!(style, "Shelley" | "Shared")
                && !factor.is_empty()
                && ![16, 20, 24, 28, 32].contains(&factor.len())
            {
                return Err("invalid_input");
            }
            let mut key = match style {
                "Ledger" => {
                    if !factor.is_empty() {
                        return Err("unsupported_profile");
                    }
                    quetzal_crypto::derivation::derive_ledger_master_key(&mnemonic.to_string())
                }
                "Icarus" | "Trezor" | "Shelley" | "Shared" => {
                    derive_icarus_master_key(&mnemonic.to_entropy(), &factor)
                }
                _ => return Err("invalid_input"),
            }
            .map_err(|_| "invalid_input")?;
            let encrypted = quetzal_crypto::password::legacy_serialization(
                &key,
                text(input, "encryption_passphrase_utf8")?.as_bytes(),
            )
            .map_err(|_| "invalid_input")?;
            for index in path {
                key = key.derive(index);
            }
            let signature = quetzal_crypto::sign(&key, &bytes(input, "message_hex")?);
            Ok(
                json!({"xprv_hex":hex::encode(key.to_raw_bytes()),"xpub_hex":hex::encode(key.public_key().to_raw_bytes()),"encrypted_xprv_hex":hex::encode(encrypted),"signature_hex":hex::encode(signature)}),
            )
        }
        _ => Err("unsupported_operation"),
    }
}

fn execute_wallet_selection(input: &Value) -> Result<Value, Value> {
    use quetzal_wallet::{seeded_selection, selection, wallet_selection};
    #[derive(Deserialize)]
    struct Amount {
        coin: u64,
        asset_quantity: u64,
    }
    #[derive(Deserialize)]
    struct Entry {
        id: u32,
        coin: u64,
        asset_quantity: u64,
        inputs: bool,
        collateral: bool,
    }
    #[derive(Deserialize)]
    struct Parameters {
        utxos: Vec<Entry>,
        outputs: Vec<Amount>,
        minimum_fee: u64,
        minimum_change_coin: u64,
        maximum_collateral_inputs: usize,
        collateral_percentage: u64,
        collateral_required: bool,
        mint_quantity: u64,
        burn_quantity: u64,
        seed: u64,
        strategy: seeded_selection::Strategy,
    }
    let invalid = || json!({"code":"invalid_input"});
    let parameters: Parameters = serde_json::from_value(input.clone()).map_err(|_| invalid())?;
    if parameters.utxos.len() > 64
        || parameters.outputs.len() > 16
        || parameters.maximum_collateral_inputs > 64
    {
        return Err(invalid());
    }
    let ids = parameters
        .utxos
        .iter()
        .map(|u| u.id)
        .collect::<std::collections::BTreeSet<_>>();
    if ids.len() != parameters.utxos.len() {
        return Err(invalid());
    }
    let asset = format!("{}.00", "11".repeat(28));
    let bundle = |coin, quantity| selection::Bundle {
        coin,
        assets: if quantity == 0 {
            Default::default()
        } else {
            std::collections::BTreeMap::from([(asset.clone(), quantity)])
        },
    };
    let key = |id: u32| format!("{id:010}");
    let request = wallet_selection::Request {
        balance: seeded_selection::Request {
            utxos: parameters
                .utxos
                .iter()
                .map(|u| selection::Input {
                    id: key(u.id),
                    value: bundle(u.coin, u.asset_quantity),
                    eligible: u.inputs,
                })
                .collect(),
            outputs: parameters
                .outputs
                .iter()
                .map(|o| bundle(o.coin, o.asset_quantity))
                .collect(),
            seed: parameters.seed,
            minimum_fee: parameters.minimum_fee,
            minimum_change_coin: parameters.minimum_change_coin,
            strategy: parameters.strategy,
            maximum_output_coin: 45_000_000_000_000_000,
            maximum_output_token_quantity: i64::MAX as u64,
            maximum_assets_per_bundle: 32,
        },
        accounting: seeded_selection::Accounting {
            extra_in: seeded_selection::Bundle::from(&bundle(0, parameters.mint_quantity)),
            extra_out: seeded_selection::Bundle::from(&bundle(0, parameters.burn_quantity)),
        },
        collateral_required: parameters.collateral_required,
        maximum_collateral_inputs: parameters.maximum_collateral_inputs,
        collateral_percentage: parameters.collateral_percentage,
        collateral_candidates: parameters
            .utxos
            .iter()
            .filter(|u| u.collateral && u.asset_quantity == 0)
            .map(|u| (key(u.id), u128::from(u.coin)))
            .collect(),
    };
    let result = match wallet_selection::select(&request) {
        Ok(result) => result,
        Err(wallet_selection::Error::Balance(error)) => {
            return Err(json!({"code":match error {
                seeded_selection::Error::InvalidInput=>"invalid_input",seeded_selection::Error::EmptyUtxo=>"empty_utxo",seeded_selection::Error::BalanceInsufficient{..}=>"balance_insufficient",seeded_selection::Error::UnableToConstructChange{..}=>"unable_to_construct_change",
            }}))
        }
        Err(wallet_selection::Error::InsufficientCollateral {
            minimum_collateral,
            largest_available,
        }) => {
            let largest_available = largest_available
                .iter()
                .map(|id| id.parse::<u32>().map_err(|_| invalid()))
                .collect::<Result<Vec<_>, _>>()?;
            return Err(
                json!({"code":"insufficient_collateral","minimum_collateral":minimum_collateral,"largest_available":largest_available}),
            );
        }
    };
    let render = |value: &seeded_selection::Bundle| json!({"coin":value.coin,"asset_quantity":value.assets.values().sum::<u128>()});
    let inputs = result
        .balance
        .inputs
        .iter()
        .map(|id| id.parse::<u32>().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    let collateral = result
        .collateral
        .iter()
        .map(|id| id.parse::<u32>().map_err(|_| invalid()))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(
        json!({"inputs":inputs,"collateral":collateral,"change":result.balance.change.iter().map(render).collect::<Vec<_>>(),
        "outputs":request.balance.outputs.iter().map(|o|render(&seeded_selection::Bundle::from(o))).collect::<Vec<_>>(),"fee":result.balance.fee}),
    )
}

fn credential_snapshot(
    key: Option<&quetzal_crypto::EncryptedKey>,
    probes: &[Value],
) -> Result<Value, &'static str> {
    let Some(key) = key else {
        return Ok(
            json!({"format":"none","key_hex":null,"xpub_hex":null,"scheme":null,
            "password_hash_present":false,"password_checks":probes.iter().map(|_|false).collect::<Vec<_>>()}),
        );
    };
    let raw = key.serialized_root().ok_or("invalid_input")?;
    let checks = probes
        .iter()
        .map(|password| {
            Ok(key
                .decrypt(password.as_str().ok_or("invalid_input")?.as_bytes())
                .is_ok())
        })
        .collect::<Result<Vec<_>, &'static str>>()?;
    Ok(
        json!({"format":if raw.len()==128 {"v1"} else {"v2"},"key_hex":hex::encode(raw),
        "xpub_hex":hex::encode(key.public_key().map_err(|_|"invalid_input")?),"scheme":key.credential_scheme(),
        "password_hash_present":matches!(key, quetzal_crypto::EncryptedKey::Byron(_)),"password_checks":checks}),
    )
}

fn execute_credentials(input: &Value) -> Result<Value, &'static str> {
    use quetzal_crypto::{passphrase::LegacyScheme, EncryptedKey};
    use quetzal_wallet::{WalletError, WalletService};
    if text(input, "randomness")? != "counter-v1" {
        return Err("invalid_input");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "invalid_input")?;
    let directory = std::env::temp_dir().join(format!(
        "quetzal-credential-adapter-{}",
        uuid::Uuid::new_v4()
    ));
    let result = runtime.block_on(async {
        let service = WalletService::new(directory.to_str().ok_or("invalid_input")?).map_err(|_|"invalid_input")?;
        let mnemonic = input["mnemonic"].as_array().ok_or("invalid_input")?.iter()
            .map(|word| word.as_str().ok_or("invalid_input")).collect::<Result<Vec<_>,_>>()?.join(" ");
        let mnemonic = Mnemonic::parse(&mnemonic).map_err(|_|"invalid_input")?;
        let flavor = text(input,"flavor")?;
        let password = text(input,"passphrase_utf8")?.as_bytes();
        let entropy = mnemonic.to_entropy();
        let root = if flavor == "byron" { quetzal_crypto::derivation::derive_byron_master_key(&entropy) }
            else { derive_icarus_master_key(&entropy,b"") }.map_err(|_|"invalid_input")?;
        let wallet = match flavor {
            "shelley" => service.create_wallet("credential fixture".into(), &entropy, password, 20, pallas_addresses::Network::Testnet).await,
            "shared" => {
                let template=json!({"cosigners":{"cosigner#0":"self"},"template":{"all":["cosigner#0"]}});
                service.create_shared_wallet(quetzal_wallet::wallet::SharedWalletCreation {
                    name:"credential fixture".into(), entropy:&entropy, passphrase:password, account_ix:0,
                    cosigners:std::collections::HashMap::from([("cosigner#0".into(),None)]),
                    payment_template:&template, delegation_template:None, network:pallas_addresses::Network::Testnet,
                }).await
            }
            "byron" | "icarus" => service.create_byron_wallet_style("credential fixture".into(), &entropy, password, Some(42), if flavor=="byron" {"random"} else {"icarus"}).await,
            _ => return Err("invalid_input"),
        }.map_err(|_|"invalid_input")?;
        let mut random = CounterRandom::default();
        let modern = matches!(flavor,"shelley"|"shared");
        let initial = match text(input,"initial_format")? {
            "none" => None,
            "new" if modern => Some(EncryptedKey::V2(quetzal_crypto::rootkey::RootKey::encrypt(&root,password,&mut random).map_err(|_|"invalid_input")?)),
            "new" | "v1" => Some(EncryptedKey::encrypt_legacy(&root,password,match text(input,"scheme")? {
                "pbkdf2"=>LegacyScheme::Pbkdf2,"scrypt"=>LegacyScheme::Scrypt,_=>return Err("invalid_input"),
            }, &[0;32]).map_err(|_|"invalid_input")?),
            _ => return Err("invalid_input"),
        };
        service.import_credentials(&wallet.id, initial.as_ref()).await.map_err(|_|"invalid_input")?;
        let probes = input["password_probes"].as_array().ok_or("invalid_input")?;
        let message = bytes(input,"message_hex")?;
        let (mut previous_key, mut previous_meta) = service.credential_state(&wallet.id).await.map_err(|_|"invalid_input")?;
        let first = credential_snapshot(previous_key.as_ref(),probes)?;
        let mut steps = Vec::new();
        for action in input["actions"].as_array().ok_or("invalid_input")? {
            let mut callback=Value::Null;
            let mut consumed=0;
            let mut production=false;
            let mut warnings=0;
            let result = match text(action,"type")? {
                "change" => service.change_credentials(&wallet.id,text(action,"old_passphrase_utf8")?.as_bytes(),text(action,"new_passphrase_utf8")?.as_bytes(),&mut random).await,
                "unlock" => {
                    let unlocked = service.unlock_credentials_with_backend(&wallet.id,text(action,"passphrase_utf8")?.as_bytes(),&mut random, |key, password, rng| {
                        if action["fail_migration"] == true {
                            quetzal_crypto::rootkey::RootKey::encrypt_with_kdf(key, password, rng, |_, _, parameters| {
                                consumed += 1;
                                production = parameters == quetzal_crypto::rootkey::PRODUCTION_KDF_PARAMETERS;
                                Err(quetzal_crypto::rootkey::RootError::Internal)
                            })
                        } else { quetzal_crypto::rootkey::RootKey::encrypt(key,password,rng) }
                    }, |_| Ok(())).await;
                    match unlocked {
                        Ok(unlocked) => {
                            warnings=usize::from(unlocked.migration_warning);
                            callback=json!({"access":unlocked.access});
                            if action["callback_error"] == true { Err(WalletError::TxBuild("credential callback failed".into())) }
                            else {
                                callback["xpub_hex"]=json!(hex::encode(unlocked.key.public_key().to_raw_bytes()));
                                callback["signature_hex"]=json!(hex::encode(quetzal_crypto::sign(&unlocked.key,&message)));
                                Ok(())
                            }
                        }
                        Err(error)=>Err(error),
                    }
                }
                _=>return Err("invalid_input"),
            };
            let outcome = match result {
                Ok(())=>"success", Err(WalletError::NoRootKey(_))=>"no_root_key", Err(WalletError::WrongPassphrase)=>"wrong_passphrase",
                Err(WalletError::TxBuild(_)) if action["callback_error"] == true && !callback.is_null()=>"action_failed",
                Err(_)=>return Err("invalid_input"),
            };
            let (after_key,after_meta)=service.credential_state(&wallet.id).await.map_err(|_|"invalid_input")?;
            let unchanged = quetzal_primitives::cbor_encode(&previous_key).map_err(|_|"invalid_input")? == quetzal_primitives::cbor_encode(&after_key).map_err(|_|"invalid_input")?;
            let metadata_unchanged = previous_meta.passphrase_scheme==after_meta.passphrase_scheme && previous_meta.passphrase_last_updated_at==after_meta.passphrase_last_updated_at;
            let same_bytes = previous_key.as_ref().and_then(EncryptedKey::serialized_root)==after_key.as_ref().and_then(EncryptedKey::serialized_root);
            let mut step=json!({"outcome":outcome,"callback":callback,"credentials_unchanged":unchanged,
                "metadata_unchanged":metadata_unchanged,"timestamp_unchanged":previous_meta.passphrase_last_updated_at==after_meta.passphrase_last_updated_at,
                "key_bytes_unchanged":same_bytes,"after":credential_snapshot(after_key.as_ref(),probes)?});
            if action["fail_migration"] == true {
                step["migration_faults_consumed"]=json!(consumed);
                step["migration_warnings"]=json!(warnings);
                step["production_kdf_parameters"]=json!(production);
            }
            steps.push(step);
            previous_key=after_key;
            previous_meta=after_meta;
        }
        Ok(json!({"initial":first,"steps":steps}))
    });
    if directory.exists() {
        std::fs::remove_dir_all(directory).map_err(|_| "invalid_input")?;
    }
    result
}

#[derive(Default)]
struct CounterRandom(u64);
impl rand::RngCore for CounterRandom {
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        let counter = self.0.to_le_bytes();
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte = counter[index % 8];
        }
        self.0 += 1;
    }
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }
    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand::Error> {
        self.fill_bytes(bytes);
        Ok(())
    }
}

fn root_error(error: quetzal_crypto::rootkey::RootError) -> &'static str {
    use quetzal_crypto::rootkey::RootError;
    match error {
        RootError::Decode => "XPrvDecodeError",
        RootError::Authentication => "XPrvAuthenticationFailed",
        RootError::InvalidSecret => "XPrvInvalidSecretKey",
        RootError::PublicMismatch => "XPrvPublicKeyMismatch",
        RootError::Internal => "XPrvInternalError",
    }
}

fn root_result(key: quetzal_crypto::rootkey::RootKey) -> Result<Value, &'static str> {
    Ok(
        json!({"format": key.format(), "key_hex": hex::encode(key.serialized()), "xpub_hex":hex::encode(key.public_key().map_err(root_error)?)}),
    )
}

fn execute_root(operation: &str, input: &Value) -> Result<Value, &'static str> {
    use quetzal_crypto::rootkey::RootKey;
    let mut random = CounterRandom::default();
    if matches!(
        operation,
        "root-key.create-v2" | "root-key.change-v2" | "root-key.derive-v2"
    ) && text(input, "randomness")? != "counter-v1"
    {
        return Err("invalid_input");
    }
    if operation == "root-key.create-v2" {
        return root_result(
            RootKey::create(
                &bytes(input, "master_key_hex")?,
                &bytes(input, "passphrase_hex")?,
                &mut random,
            )
            .map_err(root_error)?,
        );
    }
    let key = RootKey::parse(&bytes(input, "key_hex")?).map_err(root_error)?;
    match operation {
        "root-key.parse" => Ok(
            json!({"format":key.format(),"xpub_hex":hex::encode(key.public_key().map_err(root_error)?)}),
        ),
        "root-key.check-v2" => {
            key.decrypt(&bytes(input, "passphrase_hex")?)
                .map_err(root_error)?;
            Ok(json!({"valid":true}))
        }
        "root-key.sign-v2" => {
            let private = key
                .decrypt(&bytes(input, "passphrase_hex")?)
                .map_err(root_error)?;
            Ok(
                json!({"signature_hex":hex::encode(quetzal_crypto::sign(&private,&bytes(input,"message_hex")?)),"xpub_hex":hex::encode(key.public_key().map_err(root_error)?)}),
            )
        }
        "root-key.change-v2" => root_result(
            key.change_passphrase(
                &bytes(input, "old_passphrase_hex")?,
                &bytes(input, "new_passphrase_hex")?,
                &mut random,
            )
            .map_err(root_error)?,
        ),
        "root-key.derive-v2" => root_result(
            key.derive(&bytes(input, "passphrase_hex")?, &path(input)?, &mut random)
                .map_err(root_error)?,
        ),
        _ => Err("unsupported_operation"),
    }
}

fn main() -> anyhow::Result<()> {
    let mut line = String::new();
    io::stdin()
        .lock()
        .take(1024 * 1024 + 1)
        .read_line(&mut line)?;
    anyhow::ensure!(line.len() <= 1024 * 1024, "request too large");
    let request: Request = serde_json::from_str(&line)?;
    let mut response = json!({"protocol_version": 1, "id": request.id});
    if request.operation == "coin-selection.haskell" && request.protocol_version == 1 {
        let result =
            serde_json::from_value::<quetzal_wallet::seeded_selection::Request>(request.input)
                .map_err(|_| quetzal_wallet::seeded_selection::Error::InvalidInput)
                .and_then(|request| quetzal_wallet::seeded_selection::select(&request));
        match result {
            Ok(result) => response["result"] = serde_json::to_value(result)?,
            Err(error) => response["error"] = serde_json::to_value(error)?,
        }
        println!("{}", serde_json::to_string(&response)?);
        return Ok(());
    }
    if request.operation == "selection.wallet" && request.protocol_version == 1 {
        match execute_wallet_selection(&request.input) {
            Ok(result) => response["result"] = result,
            Err(error) => response["error"] = error,
        }
        println!("{}", serde_json::to_string(&response)?);
        return Ok(());
    }
    match execute(&request) {
        Ok(result) => response["result"] = result,
        Err("metadata_payload_exhausted") => {
            response["error"] = json!({"code":"haskell_error_call","class":"ErrorCall","message":"we have checked already in composePayload that there is enough elements in decrypedList"})
        }
        Err(code) if code.starts_with("haskell:") => {
            response["error"] = json!({"code":"haskell_error_call","class":"ErrorCall","message":code.trim_start_matches("haskell:")})
        }
        Err(code) => response["error"] = json!({"code": code}),
    }
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}
