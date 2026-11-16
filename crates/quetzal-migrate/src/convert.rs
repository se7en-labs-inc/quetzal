//! Map the pinned SQLite schema into the wallet's typed import boundary.
use crate::source::{Cell, Row, Tables};
use anyhow::{bail, ensure, Context, Result};
use quetzal_crypto::{passphrase::LegacyScheme, rootkey::RootKey, EncryptedKey};
use quetzal_primitives::{
    tx::{AssetAmount, BlockHeight, CoinBundle, TxBlock, TxInput, TxOutput, TxTime, Withdrawal},
    *,
};
use quetzal_wallet::{
    db::import::ImportedWallet,
    staking::{PoolChange, StakeState},
};
use serde_json::{json, Value as Json};
use std::collections::{HashMap, HashSet};

pub fn rows<'a>(t: &'a Tables, n: &str) -> &'a [Row] {
    t.get(n).map(Vec::as_slice).unwrap_or_default()
}
fn field<'a>(r: &'a Row, n: &str) -> Result<&'a Cell> {
    r.get(n)
        .with_context(|| format!("missing source field {n}"))
}
fn text<'a>(r: &'a Row, n: &str) -> Result<&'a str> {
    field(r, n)?.text()
}
fn optional<'a>(r: &'a Row, n: &str) -> Result<Option<&'a str>> {
    let Some(c) = r.get(n) else { return Ok(None) };
    if c.kind == "null" {
        return Ok(None);
    };
    Ok(Some(
        c.value.as_deref().context("missing source field value")?,
    ))
}
fn number(r: &Row, n: &str) -> Result<u64> {
    let c = field(r, n)?;
    ensure!(
        c.kind == "integer" || c.kind == "text",
        "noninteger source field {n}"
    );
    c.value
        .as_deref()
        .context("missing source number")?
        .parse()
        .with_context(|| format!("invalid nonnegative source field {n}"))
}
fn index(r: &Row, n: &str) -> Result<u32> {
    Ok(number(r, n)?.try_into()?)
}
fn ascii_blob(r: &Row, n: &str) -> Result<String> {
    String::from_utf8(field(r, n)?.bytes()?)
        .map_err(|_| anyhow::anyhow!("invalid binary text encoding in {n}"))
}
fn public(r: &Row, n: &str) -> Result<[u8; 64]> {
    let raw = hex::decode(ascii_blob(r, n)?)?;
    let p = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&raw)?;
    Ok(p.to_raw_bytes())
}
fn address_hex(s: &str) -> Result<Address> {
    let bytes = hex::decode(s)?;
    Ok(Address(
        pallas_addresses::Address::from_bytes(&bytes)?.to_string(),
    ))
}

/// SQLite pools store payment fingerprints as enterprise addresses. Lift only
/// after proving that the derived address has the same payment credential.
fn lift_pool_address(stored: &Address, complete: Address) -> Result<Address> {
    let source = stored.as_str().parse::<pallas_addresses::Address>()?;
    let expected = complete.as_str().parse::<pallas_addresses::Address>()?;
    let (pallas_addresses::Address::Shelley(source), pallas_addresses::Address::Shelley(expected)) =
        (source, expected)
    else {
        bail!("invalid Shelley/shared discovery address");
    };
    let source = source.to_vec();
    let expected = expected.to_vec();
    ensure!(
        source[0] & 0x1f == expected[0] & 0x1f && source[1..29] == expected[1..29],
        "source discovery payment credential/network differs from derivation"
    );
    ensure!(
        source.len() == 29 || source == expected,
        "source discovery delegation credential differs from derivation"
    );
    Ok(complete)
}
fn timestamp(s: &str) -> Result<String> {
    let s = s.replace(' ', "T");
    let s = if s.ends_with('Z') || s.get(10..).is_some_and(|v| v.contains('+')) {
        s
    } else {
        format!("{s}Z")
    };
    Ok(quetzal_primitives::slotting::compact_utc_fraction(
        chrono::DateTime::parse_from_rfc3339(&s)?
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true),
    ))
}
#[derive(Clone)]
pub struct Clock {
    pub genesis_start: u64,
    pub eras: Vec<quetzal_network::statequery::EraTiming>,
}
impl Clock {
    pub fn reference(&self, slot: u64) -> Result<(u64, u64, String)> {
        let era = self
            .eras
            .iter()
            .rev()
            .find(|e| e.start_slot <= slot)
            .context("native era history does not cover source slot")?;
        ensure!(
            era.epoch_length > 0 && era.slot_millis > 0,
            "invalid native era timing"
        );
        let delta = slot - era.start_slot;
        let millis = self
            .genesis_start
            .checked_mul(1000)
            .and_then(|n| n.checked_add(era.relative_millis))
            .and_then(|n| {
                delta
                    .checked_mul(era.slot_millis)
                    .and_then(|d| n.checked_add(d))
            })
            .context("slot time overflow")?;
        let time = chrono::DateTime::from_timestamp_millis(i64::try_from(millis)?)
            .context("invalid slot time")?
            .to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true);
        Ok((
            era.start_epoch + delta / era.epoch_length,
            delta % era.epoch_length,
            quetzal_primitives::slotting::compact_utc_fraction(time),
        ))
    }
    fn tx_time(&self, slot: u64, height: u64) -> Result<TxTime> {
        let (epoch, _, time) = self.reference(slot)?;
        Ok(TxTime {
            time,
            block: TxBlock {
                slot_no: slot,
                epoch_no: epoch.try_into()?,
                height: BlockHeight::new(height),
            },
        })
    }
}
fn root_key(
    t: &Tables,
    id: &WalletId,
    kind: &WalletType,
    scheme: &str,
) -> Result<(Option<EncryptedKey>, Option<[u8; 64]>)> {
    let rows = rows(t, "private_key");
    ensure!(
        rows.len() <= 1,
        "multiple private keys in one source wallet"
    );
    let Some(r) = rows.first() else {
        return Ok((None, None));
    };
    ensure!(
        text(r, "wallet_id")? == id.to_hex(),
        "private key wallet ID mismatch"
    );
    let serialized = ascii_blob(r, "root")?;
    let mut segments = serialized.split(':').collect::<Vec<_>>();
    let v2 = segments.first() == Some(&"V2");
    if v2 {
        segments.remove(0);
        if segments.first().is_some_and(|s| s.len() == 256) {
            segments.remove(0);
        }
    }
    let random = *kind == WalletType::Byron;
    ensure!(
        segments.len() == if random { 2 } else { 1 },
        "invalid persisted credential fields"
    );
    if random {
        let payload = hex::decode(segments[1])?;
        ensure!(
            payload.len() == 32,
            "invalid random-wallet discovery payload"
        );
    }
    let raw = hex::decode(segments[0])?;
    let parsed =
        RootKey::parse(&raw).map_err(|_| anyhow::anyhow!("invalid encrypted root envelope"))?;
    let public = parsed
        .public_key()
        .map_err(|_| anyhow::anyhow!("invalid encrypted root public key"))?;
    if random {
        ensure!(
            hex::decode(segments[1])? == quetzal_crypto::byron::payload_key(&public)?,
            "random-wallet discovery payload differs from root public key"
        );
    }
    let key = if v2 {
        ensure!(parsed.format() == "v2", "V2 marker contains legacy root");
        EncryptedKey::V2(parsed)
    } else {
        let hash = hex::decode(ascii_blob(r, "hash")?)?;
        let scheme = match scheme {
            "EncryptWithPBKDF2" | "pbkdf2_hmac_sha512" | "pbkdf2-hmac-sha512" => {
                LegacyScheme::Pbkdf2
            }
            "EncryptWithScrypt" | "scrypt" => LegacyScheme::Scrypt,
            _ => bail!("unsupported legacy credential scheme"),
        };
        EncryptedKey::from_legacy_parts(raw, hash, scheme)?
    };
    if *kind != WalletType::Shared {
        ensure!(
            *pallas_crypto::hash::Hasher::<160>::hash(&public) == id.0,
            "root public key does not match wallet identity"
        );
    }
    Ok((Some(key), Some(public)))
}
fn prefix(s: &str) -> Result<(u32, u32)> {
    let p = s
        .split('/')
        .map(str::parse::<u32>)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    ensure!(
        p.len() == 3 && p.iter().all(|n| *n >= 0x80000000) && p[1] == 0x80000717,
        "unsupported wallet derivation prefix"
    );
    Ok((p[0] - 0x80000000, p[2] - 0x80000000))
}
fn role(s: &str) -> Result<AddressRole> {
    match s {
        "utxo_external" => Ok(AddressRole::External),
        "utxo_internal" => Ok(AddressRole::Internal),
        "mutable_account" => Ok(AddressRole::Stake),
        _ => bail!("unsupported source address role"),
    }
}
fn state(s: &str) -> Result<AddressState> {
    match s {
        "used" => Ok(AddressState::Used),
        "unused" => Ok(AddressState::Unused),
        _ => bail!("unsupported source address status"),
    }
}
fn template(t: &Tables, s: &Row, name: &str, credential: &str) -> Result<Option<Json>> {
    let Some(script) = optional(s, name)? else {
        return Ok(None);
    };
    let script: Json = serde_json::from_str(script)?;
    let mut cosigners = serde_json::Map::new();
    for r in rows(t, "cosigner_key") {
        if text(r, "credential")? != credential {
            continue;
        }
        let key = public(r, "account_xpub")?;
        cosigners.insert(
            format!("cosigner#{}", number(r, "cosigner_index")?),
            json!(quetzal_address::codec::bech32_encode(
                "acct_shared_xvk",
                &key
            )?),
        );
    }
    Ok(Some(json!({"cosigners":cosigners,"template":script})))
}
fn asset(r: &Row) -> Result<AssetAmount> {
    let policy = text(r, "token_policy_id")?.to_ascii_lowercase();
    ensure!(hex::decode(&policy)?.len() == 28, "invalid asset policy");
    let name = text(r, "token_name")?.to_ascii_lowercase();
    ensure!(hex::decode(&name)?.len() <= 32, "invalid asset name");
    Ok(AssetAmount {
        policy_id: policy,
        asset_name: name,
        quantity: number(r, "token_quantity")?,
    })
}

fn value_from_delta(bytes: &[u8]) -> Result<(Address, quetzal_primitives::Value)> {
    use pallas_codec::minicbor::Decoder;
    let mut d = Decoder::new(bytes);
    ensure!(d.array()? == Some(3), "invalid delta UTxO output");
    let address = address_hex(&hex::encode(d.bytes()?))?;
    let coin = d.u64()?;
    let count = d
        .array()?
        .context("indefinite delta token list unsupported")?;
    let mut value = quetzal_primitives::Value::lovelace(Coin(coin));
    for _ in 0..count {
        ensure!(d.array()? == Some(3), "invalid delta token tuple");
        let policy = d.bytes()?;
        ensure!(policy.len() == 28, "invalid delta policy");
        let policy = hex::encode(policy);
        let name = d.bytes()?;
        ensure!(name.len() <= 32, "invalid delta asset name");
        let name = hex::encode(name);
        let quantity = d.u64()?;
        ensure!(
            value
                .assets
                .entry(policy)
                .or_default()
                .insert(name, quantity)
                .is_none(),
            "duplicate delta asset"
        );
    }
    ensure!(d.position() == bytes.len(), "trailing delta output bytes");
    Ok((address, value))
}
fn utxos(t: &Tables, checkpoint: &Checkpoint) -> Result<Vec<UtxoEntry>> {
    let delta = rows(t, "delta_u_tx_o_value");
    let mut result = Vec::new();
    if !delta.is_empty() {
        let slots = rows(t, "delta_u_tx_o_slots");
        ensure!(
            slots.len() == 1 && number(&slots[0], "tip")? == checkpoint.slot,
            "delta UTxO tip differs from checkpoint"
        );
        for r in delta {
            if number(r, "boot")? != 0 || optional(r, "spent")?.is_some() {
                continue;
            }
            ensure!(
                number(r, "slot")? <= checkpoint.slot,
                "UTxO created beyond checkpoint"
            );
            let (address, value) = value_from_delta(&field(r, "tx_out")?.bytes()?)?;
            result.push(UtxoEntry {
                id: UtxoId::new(
                    TxHash::from_hex(text(r, "tx_in_tx")?)?,
                    index(r, "tx_in_ix")?,
                ),
                address,
                value,
            });
        }
    } else {
        let mut tokens = HashMap::<_, Vec<_>>::new();
        for row in rows(t, "utxo_token") {
            if number(row, "slot")? == checkpoint.slot {
                tokens
                    .entry((text(row, "tx_id")?, number(row, "tx_index")?))
                    .or_default()
                    .push(asset(row)?);
            }
        }
        for r in rows(t, "utxo") {
            if number(r, "slot")? != checkpoint.slot {
                continue;
            }
            let mut value = quetzal_primitives::Value::lovelace(Coin(number(r, "output_coin")?));
            let tx = text(r, "input_tx_id")?;
            let ix = number(r, "input_index")?;
            if let Some(tokens) = tokens.get(&(tx, ix)) {
                for a in tokens {
                    ensure!(
                        value
                            .assets
                            .entry(a.policy_id.clone())
                            .or_default()
                            .insert(a.asset_name.clone(), a.quantity)
                            .is_none(),
                        "duplicate UTxO token"
                    );
                }
            }
            result.push(UtxoEntry {
                id: UtxoId::new(TxHash::from_hex(tx)?, ix.try_into()?),
                address: address_hex(text(r, "output_address")?)?,
                value,
            });
        }
    }
    Ok(result)
}
fn tx_status(s: &str) -> Result<TxStatus> {
    match s {
        "in_ledger" => Ok(TxStatus::InLedger),
        "pending" => Ok(TxStatus::Pending),
        "expired" => Ok(TxStatus::Expired),
        _ => bail!("unsupported transaction status"),
    }
}
fn tx_direction(r: &Row) -> Result<TxDirection> {
    match number(r, "direction")? {
        1 => Ok(TxDirection::Incoming),
        0 => Ok(TxDirection::Outgoing),
        _ => bail!("invalid transaction direction"),
    }
}
fn grouped<'a>(t: &'a Tables, table: &str) -> Result<HashMap<&'a str, Vec<&'a Row>>> {
    let mut result = HashMap::<_, Vec<_>>::new();
    for row in rows(t, table) {
        result.entry(text(row, "tx_id")?).or_default().push(row);
    }
    Ok(result)
}
fn history(t: &Tables, clock: &Clock, network: pallas_addresses::Network) -> Result<Vec<TxRecord>> {
    let inputs_by_id = grouped(t, "tx_in")?;
    let outputs_by_id = grouped(t, "tx_out")?;
    let withdrawals_by_id = grouped(t, "tx_withdrawal")?;
    let collateral_by_id = grouped(t, "tx_collateral")?;
    let collateral_outputs_by_id = grouped(t, "tx_collateral_out")?;
    let collateral_tokens_by_id = grouped(t, "tx_collateral_out_token")?;
    let mut output_index = HashMap::new();
    for row in rows(t, "tx_out") {
        ensure!(
            output_index
                .insert((text(row, "tx_id")?, number(row, "index")?), row)
                .is_none(),
            "duplicate transaction output"
        );
    }
    let mut assets_by_output = HashMap::<_, Vec<_>>::new();
    for row in rows(t, "tx_out_token") {
        assets_by_output
            .entry((text(row, "tx_id")?, number(row, "tx_index")?))
            .or_default()
            .push(asset(row)?);
    }
    let mut records = Vec::new();
    for r in rows(t, "tx_meta") {
        let id = text(r, "tx_id")?;
        let read_inputs = |by_id: &HashMap<&str, Vec<&Row>>| -> Result<Vec<TxInput>> {
            let mut inputs = Vec::new();
            let mut ins = by_id
                .get(id)
                .into_iter()
                .flatten()
                .map(|r| Ok((number(r, "order")?, *r)))
                .collect::<Result<Vec<_>>>()?;
            ins.sort_by_key(|(order, _)| *order);
            for (_, i) in ins {
                let source = text(i, "source_tx_id")?;
                let ix = index(i, "source_index")?;
                let output = output_index.get(&(source, u64::from(ix))).copied();
                inputs.push(TxInput {
                    id: TxHash::from_hex(source)?,
                    index: ix,
                    address: output
                        .map(|o| address_hex(text(o, "address")?))
                        .transpose()?,
                    amount: output
                        .map(|o| -> Result<_> {
                            Ok(CoinBundle::lovelace(Coin(number(o, "amount")?)))
                        })
                        .transpose()?,
                    assets: assets_by_output
                        .get(&(source, u64::from(ix)))
                        .cloned()
                        .unwrap_or_default(),
                });
            }
            Ok(inputs)
        };
        let inputs = read_inputs(&inputs_by_id)?;
        let collateral = read_inputs(&collateral_by_id)?;
        let mut collateral_outputs = Vec::new();
        for row in collateral_outputs_by_id.get(id).into_iter().flatten() {
            collateral_outputs.push(TxOutput {
                address: address_hex(text(row, "address")?)?,
                amount: CoinBundle::lovelace(Coin(number(row, "amount")?)),
                assets: collateral_tokens_by_id
                    .get(id)
                    .into_iter()
                    .flatten()
                    .map(|row| asset(row))
                    .collect::<Result<Vec<_>>>()?,
            });
        }
        ensure!(
            collateral_outputs.len() <= 1,
            "multiple collateral return outputs"
        );
        let script_validity = optional(r, "script_validity")?
            .map(|value| match value {
                "0" => Ok(false),
                "1" => Ok(true),
                _ => bail!("invalid source script validity"),
            })
            .transpose()?;
        let mut outs = outputs_by_id
            .get(id)
            .into_iter()
            .flatten()
            .map(|r| Ok((number(r, "index")?, *r)))
            .collect::<Result<Vec<_>>>()?;
        outs.sort_by_key(|(index, _)| *index);
        let mut outputs = Vec::new();
        for (index, o) in outs {
            outputs.push(TxOutput {
                address: address_hex(text(o, "address")?)?,
                amount: CoinBundle::lovelace(Coin(number(o, "amount")?)),
                assets: assets_by_output
                    .get(&(id, index))
                    .cloned()
                    .unwrap_or_default(),
            });
        }
        let withdrawals = withdrawals_by_id
            .get(id)
            .into_iter()
            .flatten()
            .map(|w| -> Result<_> {
                let account = text(w, "account")?;
                let (header, account) = match account.strip_prefix('s') {
                    Some(account) => (0xf0, account),
                    None => (0xe0, account),
                };
                let account = hex::decode(account)?;
                ensure!(account.len() == 28, "invalid withdrawal reward account");
                let mut bytes = vec![
                    header
                        | if network == pallas_addresses::Network::Mainnet {
                            1
                        } else {
                            0
                        },
                ];
                bytes.extend(account);
                Ok(Withdrawal {
                    stake_address: Address(
                        pallas_addresses::Address::from_bytes(&bytes)?.to_string(),
                    ),
                    amount: CoinBundle::lovelace(Coin(number(w, "amount")?)),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let slot = number(r, "slot")?;
        let height = number(r, "block_height")?;
        records.push(TxRecord {
            ledger_details: quetzal_primitives::tx::TxLedgerDetails {
                collateral,
                collateral_outputs,
                script_validity,
                raw_cbor: None,
                validity_interval: None,
                extra_signatures: vec![],
            },
            id: TxHash::from_hex(id)?,
            slot,
            block_hash: [0; 32],
            block_height: height,
            direction: tx_direction(r)?,
            amount: Coin(number(r, "amount")?),
            fee: optional(r, "fee")?.map(str::parse).transpose()?.map(Coin),
            inputs,
            outputs,
            status: tx_status(text(r, "status")?)?,
            inserted_at: clock.tx_time(slot, height)?,
            metadata: optional(r, "data")?.map(serde_json::from_str).transpose()?,
            withdrawals,
            expires_at_slot: optional(r, "slot_expires")?.map(str::parse).transpose()?,
            certificates: vec![],
            deposit_taken: Coin::ZERO,
            deposit_returned: Coin::ZERO,
            stake_derivation_path: None,
        });
    }
    Ok(records)
}

pub fn wallet(
    t: &Tables,
    filename: &str,
    clock: &Clock,
    genesis_hash: &str,
    network: pallas_addresses::Network,
) -> Result<ImportedWallet> {
    let metadata = rows(t, "wallet");
    ensure!(metadata.len() == 1, "expected one wallet per source file");
    let m = &metadata[0];
    let id = WalletId::from_hex(text(m, "wallet_id")?)?;
    ensure!(
        text(m, "genesis_hash")? == genesis_hash,
        "source genesis hash differs from selected network"
    );
    let start =
        chrono::DateTime::parse_from_rfc3339(&timestamp(text(m, "genesis_start")?)?)?.timestamp();
    ensure!(
        start == i64::try_from(clock.genesis_start)?,
        "source genesis start differs from selected network"
    );
    let seq = rows(t, "seq_state");
    let random = rows(t, "rnd_state");
    let shared = rows(t, "shared_state");
    ensure!(
        seq.len() + random.len() + shared.len() == 1,
        "source must have exactly one discovery state"
    );
    let (kind, account, gap) = if let Some(s) = seq.first() {
        let (purpose, account) = prefix(text(s, "derivation_prefix")?)?;
        ensure!(
            number(s, "external_gap")? == number(s, "internal_gap")?,
            "different internal/external gaps are not supported"
        );
        let kind = match purpose {
            1852 => WalletType::Shelley,
            44 => WalletType::Icarus,
            _ => bail!("unsupported sequential derivation purpose"),
        };
        (kind, account, index(s, "external_gap")?)
    } else if let Some(s) = shared.first() {
        let (purpose, account) = prefix(text(s, "derivation_prefix")?)?;
        ensure!(purpose == 1854, "invalid shared derivation purpose");
        (WalletType::Shared, account, index(s, "pool_gap")?)
    } else {
        (WalletType::Byron, 0, 0)
    };
    let expected_prefix = match kind {
        WalletType::Shelley => "she",
        WalletType::Icarus => "ica",
        WalletType::Byron => "rnd",
        WalletType::Shared => "sha",
    };
    if let Some((prefix, suffix)) = filename.split_once('.') {
        if ["she", "ica", "rnd", "sha"].contains(&prefix) {
            ensure!(
                prefix == expected_prefix && suffix == format!("{id}.sqlite"),
                "source filename/family/identity mismatch"
            );
        }
    }
    let scheme = optional(m, "passphrase_scheme")?.unwrap_or("EncryptWithPBKDF2");
    let (credentials, root_public) = root_key(t, &id, &kind, scheme)?;
    let passphrase_scheme = if credentials.as_ref().is_some_and(|k| k.envelope().is_some()) {
        PassphraseScheme::Argon2idV2
    } else {
        match scheme {
            "EncryptWithPBKDF2" | "pbkdf2_hmac_sha512" | "pbkdf2-hmac-sha512" => {
                PassphraseScheme::Pbkdf2HmacSha512
            }
            "EncryptWithScrypt" | "scrypt" => PassphraseScheme::Scrypt,
            "EncryptWithArgon2idV2" => PassphraseScheme::Argon2idV2,
            _ => bail!("unsupported passphrase scheme"),
        }
    };
    let account_public_key = if let Some(s) = seq.first() {
        public(s, "account_xpub")?
    } else if let Some(s) = shared.first() {
        public(s, "account_xpub")?
    } else {
        root_public.context("random wallet without root credentials is unsupported")?
    };
    let account_public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(&account_public_key)?;
    if kind == WalletType::Shelley {
        ensure!(
            public(&seq[0], "reward_xpub")? == account_public.derive(2)?.derive(0)?.to_raw_bytes(),
            "source reward key differs from supported account delegation path"
        );
    }
    if credentials.is_none() && kind != WalletType::Shared {
        ensure!(
            *pallas_crypto::hash::Hasher::<160>::hash(&account_public_key) == id.0,
            "watch-only identity differs from account public key"
        );
    }
    let policy_public_key = seq
        .first()
        .map(|s| -> Result<_> {
            if optional(s, "policy_xpub")?.is_some() {
                Ok(Some(public(s, "policy_xpub")?))
            } else {
                Ok(None)
            }
        })
        .transpose()?
        .flatten();
    for s in seq.iter().chain(shared) {
        if let Some(mode) = optional(s, "change_addr_mode")? {
            ensure!(
                mode == "increasing",
                "unsupported change-address allocation mode"
            );
        }
    }
    let payment_script = shared
        .first()
        .map(|s| template(t, s, "payment_script", "payment"))
        .transpose()?
        .flatten();
    let delegation_script = shared
        .first()
        .map(|s| template(t, s, "delegation_script", "delegation"))
        .transpose()?
        .flatten();
    if kind == WalletType::Shared {
        let payment = payment_script
            .as_ref()
            .context("shared payment template missing")?;
        let mut identity = account_public_key.to_vec();
        identity
            .extend(quetzal_wallet::wallet::script_template_text(&payment["template"])?.as_bytes());
        if let Some(script) = &delegation_script {
            identity.extend(
                quetzal_wallet::wallet::script_template_text(&script["template"])?.as_bytes(),
            );
        }
        ensure!(
            *pallas_crypto::hash::Hasher::<160>::hash(&identity) == id.0,
            "shared wallet identity differs from templates/account"
        );
    }
    let mut checkpoints = rows(t, "checkpoint")
        .iter()
        .map(|r| -> Result<_> {
            let bytes = hex::decode(text(r, "header_hash")?)?;
            Ok(Checkpoint {
                slot: number(r, "slot")?,
                block_hash: bytes
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid checkpoint hash"))?,
                block_height: number(r, "block_height")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    checkpoints.sort_by_key(|c| c.slot);
    let checkpoint = checkpoints.pop().context("source checkpoint missing")?;
    for table in [
        "wallet",
        "private_key",
        "seq_state",
        "rnd_state",
        "shared_state",
        "checkpoint",
        "utxo",
        "utxo_token",
        "tx_meta",
        "seq_state_address",
        "rnd_state_address",
        "rnd_state_pending_address",
        "seq_state_pending",
        "shared_state_pending",
        "cosigner_key",
        "submissions",
        "submissions_slots",
        "delta_u_tx_o_value",
        "delta_u_tx_o_slots",
    ] {
        for row in rows(t, table) {
            ensure!(
                text(row, "wallet_id")? == id.to_hex(),
                "cross-wallet row in {table}"
            );
        }
    }
    let mut address_map = HashMap::<String, AddressEntry>::new();
    for table in [
        "seq_state_address",
        "rnd_state_address",
        "rnd_state_pending_address",
    ] {
        for r in rows(t, table) {
            if table != "rnd_state_pending_address" && number(r, "slot")? != checkpoint.slot {
                continue;
            }
            let address = address_hex(text(r, "address")?)?;
            let mut entry = AddressEntry {
                address: address.clone(),
                path: DerivationPath {
                    account_ix: if kind == WalletType::Byron {
                        index(r, "account_ix")?
                    } else {
                        account
                    },
                    role: if kind == WalletType::Byron {
                        AddressRole::External
                    } else {
                        role(text(r, "role")?)?
                    },
                    address_ix: index(r, "address_ix")?,
                },
                state: if table == "rnd_state_pending_address" {
                    AddressState::Unused
                } else {
                    state(text(r, "status")?)?
                },
            };
            entry.address = match kind {
                WalletType::Shelley => lift_pool_address(
                    &entry.address,
                    quetzal_address::shelley::base_address_for_role(
                        &account_public,
                        entry.path.role.index(),
                        entry.path.address_ix,
                        0,
                        network,
                    )?,
                )?,
                WalletType::Shared => lift_pool_address(
                    &entry.address,
                    quetzal_wallet::shared::address_for_path(
                        payment_script
                            .as_ref()
                            .context("shared payment template missing")?,
                        delegation_script.as_ref(),
                        entry.path.role,
                        entry.path.address_ix,
                        network,
                    )?,
                )?,
                _ => entry.address,
            };
            if let Some(existing) = address_map.get(entry.address.as_str()) {
                ensure!(
                    existing.path == entry.path,
                    "conflicting source address paths"
                );
            } else {
                address_map.insert(entry.address.0.clone(), entry);
            }
        }
    }
    let mut addresses = address_map.into_values().collect::<Vec<_>>();
    addresses.sort_by(|a, b| a.address.0.cmp(&b.address.0));
    let mut allocated_change = Vec::new();
    for table in ["seq_state_pending", "shared_state_pending"] {
        for row in rows(t, table) {
            let ix = index(row, "pending_ix")?;
            let entry = addresses
                .iter()
                .find(|a| a.path.role == AddressRole::Internal && a.path.address_ix == ix)
                .context("allocated change address missing from source pool")?;
            allocated_change.push(entry.address.clone());
        }
    }
    let utxos = utxos(t, &checkpoint)?;
    let mut balance = quetzal_primitives::Value::default();
    for u in &utxos {
        balance = balance
            .checked_add(&u.value)
            .context("imported balance overflow")?;
    }
    let mut records = history(t, clock, network)?;
    let stake_hash = if kind == WalletType::Shelley {
        Some(quetzal_wallet::staking::stake_hash(&account_public_key)?)
    } else {
        None
    };
    let mut key_deposit = None;
    for p in rows(t, "protocol_parameters") {
        key_deposit = Some(number(p, "key_deposit")?);
    }
    let mut encoded_txs = HashMap::new();
    for row in rows(t, "c_b_o_r") {
        ensure!(
            encoded_txs.insert(text(row, "tx_id")?, row).is_none(),
            "duplicate historical CBOR"
        );
    }
    for record in &mut records {
        if let Some(row) = encoded_txs.get(record.id.to_hex().as_str()) {
            let raw = field(row, "tx_cbor")?.bytes()?;
            let tx = pallas_traverse::MultiEraTx::decode(&raw)
                .map_err(|_| anyhow::anyhow!("invalid historical transaction CBOR"))?;
            ensure!(
                *tx.hash() == record.id.0,
                "historical transaction CBOR hash mismatch"
            );
            record.ledger_details.extra_signatures = quetzal_wallet::ledger::extra_signatures(&tx);
            record.ledger_details.validity_interval =
                (!matches!(tx, pallas_traverse::MultiEraTx::Byron(_))).then(|| {
                    (
                        tx.validity_start().unwrap_or(0),
                        tx.ttl().unwrap_or(u64::MAX),
                    )
                });
            if let Some(expiry) = tx.ttl() {
                ensure!(
                    record.expires_at_slot.is_none_or(|stored| stored == expiry),
                    "historical transaction expiry differs from CBOR"
                );
                record.expires_at_slot = Some(expiry);
            }
            if let Some(hash) = stake_hash {
                let operation = quetzal_wallet::staking::owned_certificates(&tx, &hash)?;
                record.certificates = operation.encoded()?;
                record.deposit_taken = Coin(operation.deposit_taken);
                record.deposit_returned = Coin(operation.deposit_returned);
                if !record.certificates.is_empty() {
                    record.stake_derivation_path = Some(DerivationPath {
                        account_ix: account,
                        role: AddressRole::Stake,
                        address_ix: 0,
                    });
                }
            }
            record.ledger_details.raw_cbor = Some(raw);
        }
    }
    let mut staking = StakeState::default();
    let mut delegations = rows(t, "delegations").iter().collect::<Vec<_>>();
    delegations.sort_by_key(|r| number(r, "slot").unwrap_or(u64::MAX));
    for r in delegations {
        let status = text(r, "status")?;
        ensure!(
            [
                "inactive",
                "registered",
                "active",
                "voted",
                "active_and_voted"
            ]
            .contains(&status),
            "unsupported delegation status"
        );
        if status == "inactive" {
            staking = StakeState::default();
            continue;
        }
        staking.deposit = key_deposit;
        staking.vote = optional(r, "vote")?.map(str::to_owned);
        staking.pool = optional(r, "pool")?
            .map(|s| -> Result<_> {
                Ok(quetzal_address::codec::bech32_encode(
                    "pool",
                    &hex::decode(s)?,
                )?)
            })
            .transpose()?;
        if let Some(pool) = &staking.pool {
            staking.pool_changes.push(PoolChange {
                epoch: clock.reference(number(r, "slot")?)?.0 + 2,
                target: pool.clone(),
            });
        }
    }
    let reward = rows(t, "delegation_reward")
        .first()
        .map(|r| number(r, "account_balance"))
        .transpose()?
        .unwrap_or(0);
    let mut submissions = Vec::new();
    let mut pending = Vec::new();
    let pending_ids = rows(t, "tx_meta")
        .iter()
        .filter_map(|m| match text(m, "status") {
            Ok("pending") => Some(text(m, "tx_id")),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<HashSet<_>>>()?;
    for r in rows(t, "submissions") {
        let id = TxHash::from_hex(text(r, "tx_id")?)?;
        let cbor = field(r, "tx")?.bytes()?;
        ensure!(
            quetzal_wallet::txsize::body_hash(&cbor)
                .map_err(|_| anyhow::anyhow!("invalid submitted signed bytes"))?
                == id.0,
            "submitted transaction hash mismatch"
        );
        let slot = number(r, "slot")?;
        let tx = PendingTx {
            id: id.clone(),
            cbor,
            submitted_at: clock.reference(slot)?.2,
            expiry_slot: Some(number(r, "expiration")?),
            submitted_slot: slot,
            submitted_height: number(r, "block_height")?,
            latest_submission_slot: Some(number(r, "resubmitted")?),
        };
        let status = number(r, "status")?;
        ensure!(status <= 2, "unsupported local submission status");
        if pending_ids.contains(id.to_hex().as_str())
            || status == 0 && optional(r, "acceptance")?.is_none()
        {
            pending.push(tx.clone());
        }
        submissions.push(tx);
    }
    let submitted_ids = submissions
        .iter()
        .map(|s| s.id.to_hex())
        .collect::<HashSet<_>>();
    for r in rows(t, "tx_meta") {
        if text(r, "status")? == "pending" {
            ensure!(
                submitted_ids.contains(text(r, "tx_id")?),
                "pending history has no original signed submission bytes"
            );
        }
    }
    let mut available = balance.clone();
    let mut reserved = HashSet::new();
    for tx in &pending {
        let tx = pallas_traverse::MultiEraTx::decode(&tx.cbor)
            .map_err(|_| anyhow::anyhow!("invalid pending transaction"))?;
        for input in tx.inputs() {
            reserved.insert((input.hash().to_string(), input.index()));
        }
    }
    for u in &utxos {
        if reserved.contains(&(u.id.tx_hash.to_hex(), u64::from(u.id.output_index))) {
            available.coin = Coin(
                available
                    .coin
                    .0
                    .checked_sub(u.value.coin.0)
                    .context("reserved balance underflow")?,
            );
            for (policy, names) in &u.value.assets {
                for (name, quantity) in names {
                    let v = available
                        .assets
                        .get_mut(policy)
                        .and_then(|m| m.get_mut(name))
                        .context("reserved token missing")?;
                    *v = v
                        .checked_sub(*quantity)
                        .context("reserved token underflow")?;
                }
            }
        }
    }
    let assets = |value: &quetzal_primitives::Value| {
        value
            .assets
            .iter()
            .flat_map(|(p, n)| {
                n.iter().filter(|(_, q)| **q > 0).map(|(n, q)| AssetAmount {
                    policy_id: p.clone(),
                    asset_name: n.clone(),
                    quantity: *q,
                })
            })
            .collect()
    };
    let meta = WalletMeta {
        id,
        name: text(m, "name")?.into(),
        account_index: account,
        address_pool_gap: gap,
        status: WalletStatus::Syncing,
        passphrase_scheme,
        passphrase_last_updated_at: optional(m, "passphrase_last_updated_at")?
            .map(timestamp)
            .transpose()?,
        tip_slot: checkpoint.slot,
        tip_height: checkpoint.block_height,
        created_at: timestamp(text(m, "creation_time")?)?,
        balance_total: balance.coin,
        balance_available: available.coin,
        balance_reward: Coin(reward),
        assets_available: assets(&available),
        assets_total: assets(&balance),
        delegation_status: if staking.pool.is_some() {
            DelegationStatus::Delegating
        } else {
            DelegationStatus::NotDelegating
        },
        stake_pool_id: staking.pool.clone(),
        reward_account_balance: Coin(reward),
        wallet_type: kind,
    };
    Ok(ImportedWallet {
        metadata: meta,
        account_public_key,
        policy_public_key,
        credentials,
        payment_script,
        delegation_script,
        addresses,
        allocated_change,
        checkpoint,
        utxos,
        history: records,
        pending,
        submissions,
        staking,
    })
}
#[cfg(test)]
mod native_source_tests {
    #[test]
    fn source_timestamps_keep_exact_fractional_precision() -> anyhow::Result<()> {
        for (source, expected) in [
            (
                "2026-11-16 20:51:17.143004410",
                "2026-11-16T20:51:17.14300441Z",
            ),
            (
                "2026-11-16T20:51:17.14300441Z",
                "2026-11-16T20:51:17.14300441Z",
            ),
            ("2026-11-16 20:51:17.000001", "2026-11-16T20:51:17.000001Z"),
            ("2026-11-16 20:51:17.800", "2026-11-16T20:51:17.8Z"),
            ("2026-11-16 20:51:17.000", "2026-11-16T20:51:17Z"),
        ] {
            assert_eq!(super::timestamp(source)?, expected);
        }
        assert!(super::timestamp("invalid").is_err());
        Ok(())
    }
    use super::*;

    fn cell(kind: &str, value: String) -> Cell {
        Cell {
            kind: kind.into(),
            value: Some(value),
        }
    }
    #[test]
    fn pool_fingerprints_lift_to_full_addresses_without_changing_payment_ownership() -> Result<()> {
        use pallas_addresses::{
            Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart,
        };
        for script in [false, true] {
            for network in [Network::Mainnet, Network::Testnet] {
                let payment = if script {
                    ShelleyPaymentPart::script_hash([31; 28].into())
                } else {
                    ShelleyPaymentPart::key_hash([31; 28].into())
                };
                let stake = if script {
                    ShelleyDelegationPart::Script([32; 28].into())
                } else {
                    ShelleyDelegationPart::key_hash([32; 28].into())
                };
                let stored = Address(
                    ShelleyAddress::new(network, payment.clone(), ShelleyDelegationPart::Null)
                        .to_bech32()?,
                );
                let complete =
                    Address(ShelleyAddress::new(network, payment.clone(), stake).to_bech32()?);
                assert_eq!(lift_pool_address(&stored, complete.clone())?, complete);
                assert_eq!(lift_pool_address(&complete, complete.clone())?, complete);
                let wrong_key = Address(
                    ShelleyAddress::new(
                        network,
                        ShelleyPaymentPart::key_hash([33; 28].into()),
                        ShelleyDelegationPart::Null,
                    )
                    .to_bech32()?,
                );
                assert!(lift_pool_address(&wrong_key, complete.clone()).is_err());
                let wrong_stake = Address(
                    ShelleyAddress::new(
                        network,
                        payment.clone(),
                        ShelleyDelegationPart::key_hash([34; 28].into()),
                    )
                    .to_bech32()?,
                );
                assert!(lift_pool_address(&wrong_stake, complete.clone()).is_err());
                let other_network = if network == Network::Mainnet {
                    Network::Testnet
                } else {
                    Network::Mainnet
                };
                let wrong_network = Address(
                    ShelleyAddress::new(other_network, payment, ShelleyDelegationPart::Null)
                        .to_bech32()?,
                );
                assert!(lift_pool_address(&wrong_network, complete.clone()).is_err());
                if script {
                    let wrong_kind = Address(
                        ShelleyAddress::new(
                            network,
                            ShelleyPaymentPart::key_hash([31; 28].into()),
                            ShelleyDelegationPart::Null,
                        )
                        .to_bech32()?,
                    );
                    assert!(lift_pool_address(&wrong_kind, complete).is_err());
                }
            }
        }
        Ok(())
    }
    #[test]
    fn history_retains_collateral_invalid_scripts_and_script_withdrawal_network() -> Result<()> {
        let tx = hex::encode([41; 32]);
        let source = hex::encode([42; 32]);
        let address = hex::encode([&[0x61][..], &[43; 28]].concat());
        let account = format!("s{}", hex::encode([44; 28]));
        let row = |fields: &[(&str, &str)]| -> Row {
            fields
                .iter()
                .map(|(key, value)| (key.to_string(), cell("text", value.to_string())))
                .collect()
        };
        let tables = Tables::from([
            (
                "tx_meta".into(),
                vec![row(&[
                    ("tx_id", &tx),
                    ("slot", "240"),
                    ("block_height", "77"),
                    ("direction", "0"),
                    ("amount", "100"),
                    ("status", "in_ledger"),
                    ("script_validity", "0"),
                ])],
            ),
            (
                "tx_collateral".into(),
                vec![row(&[
                    ("tx_id", &tx),
                    ("order", "0"),
                    ("source_tx_id", &source),
                    ("source_index", "2"),
                    ("source_amount", "123"),
                ])],
            ),
            (
                "tx_out".into(),
                vec![row(&[
                    ("tx_id", &source),
                    ("index", "2"),
                    ("address", &address),
                    ("amount", "123"),
                ])],
            ),
            (
                "tx_collateral_out".into(),
                vec![row(&[
                    ("tx_id", &tx),
                    ("address", &address),
                    ("amount", "23"),
                ])],
            ),
            (
                "tx_collateral_out_token".into(),
                vec![row(&[
                    ("tx_id", &tx),
                    ("token_policy_id", &hex::encode([45; 28])),
                    ("token_name", "00"),
                    ("token_quantity", "18446744073709551615"),
                ])],
            ),
            (
                "tx_withdrawal".into(),
                vec![row(&[
                    ("tx_id", &tx),
                    ("account", &account),
                    ("amount", "456"),
                ])],
            ),
        ]);
        let clock = Clock {
            genesis_start: 1_600_000_000,
            eras: vec![quetzal_network::statequery::EraTiming {
                start_slot: 0,
                start_epoch: 0,
                relative_millis: 0,
                slot_millis: 500,
                epoch_length: 120,
            }],
        };
        let records = history(&tables, &clock, pallas_addresses::Network::Mainnet)?;
        let record = &records[0];
        ensure!(
            record.ledger_details.script_validity == Some(false),
            "script validity lost"
        );
        ensure!(
            record.ledger_details.collateral[0]
                .amount
                .as_ref()
                .context("collateral amount")?
                .quantity
                == Coin(123),
            "collateral amount lost"
        );
        ensure!(
            record.ledger_details.collateral[0].address.is_some(),
            "collateral resolution lost"
        );
        ensure!(
            record.ledger_details.collateral_outputs[0].assets[0].quantity == u64::MAX,
            "collateral token rounded"
        );
        let reward: pallas_addresses::Address = record.withdrawals[0].stake_address.0.parse()?;
        ensure!(
            reward.to_vec()[0] == 0xf1,
            "withdrawal script credential/network changed"
        );
        ensure!(
            record.inserted_at.block.epoch_no == 2 && record.inserted_at.block.slot_no == 240,
            "historical native timing changed"
        );
        Ok(())
    }
    #[test]
    fn persisted_legacy_and_prefixed_v2_credentials_authenticate_without_reencryption() -> Result<()>
    {
        let private = quetzal_crypto::derive_icarus_master_key(&[31; 20], b"")?;
        let password = b"public-import-format-password";
        let public = private.public_key().to_raw_bytes();
        let id = WalletId(*pallas_crypto::hash::Hasher::<160>::hash(&public));
        for scheme in [LegacyScheme::Pbkdf2, LegacyScheme::Scrypt] {
            let original = EncryptedKey::encrypt_legacy(&private, password, scheme, &[19; 32])?;
            let serialized = hex::encode(original.serialized_root().context("legacy root")?);
            let hash = match scheme {
                LegacyScheme::Pbkdf2 => quetzal_crypto::password::hash_pbkdf2(password, &[19; 32])?,
                LegacyScheme::Scrypt => quetzal_crypto::password::hash_scrypt(password, &[19; 32])?,
            };
            for kind in [WalletType::Shelley, WalletType::Byron] {
                let suffix = if kind == WalletType::Byron {
                    format!(
                        ":{}",
                        hex::encode(quetzal_crypto::byron::payload_key(&public)?)
                    )
                } else {
                    String::new()
                };
                let row = Row::from([
                    ("wallet_id".into(), cell("text", id.to_hex())),
                    (
                        "root".into(),
                        cell("blob", hex::encode(format!("{serialized}{suffix}"))),
                    ),
                    ("hash".into(), cell("blob", hex::encode(hex::encode(&hash)))),
                ]);
                let tables = Tables::from([("private_key".into(), vec![row])]);
                let (parsed, _) = root_key(
                    &tables,
                    &id,
                    &kind,
                    match scheme {
                        LegacyScheme::Pbkdf2 => "EncryptWithPBKDF2",
                        LegacyScheme::Scrypt => "EncryptWithScrypt",
                    },
                )?;
                let parsed = parsed.context("imported legacy key missing")?;
                ensure!(
                    parsed.serialized_root() == original.serialized_root(),
                    "legacy root was altered"
                );
                ensure!(
                    *parsed.decrypt(password)? == private.to_raw_bytes(),
                    "legacy unlock changed key"
                );
                ensure!(
                    parsed.decrypt(b"wrong-password").is_err(),
                    "wrong legacy password accepted"
                );
            }
        }
        let v2 = EncryptedKey::encrypt(&private.to_raw_bytes(), password)?;
        let old = quetzal_crypto::password::legacy_serialization(&private, password)?;
        let serialized = format!(
            "V2:{}:{}:{}",
            hex::encode(old),
            hex::encode(v2.envelope().context("V2 envelope")?),
            hex::encode(quetzal_crypto::byron::payload_key(&public)?)
        );
        let tables = Tables::from([(
            "private_key".into(),
            vec![Row::from([
                ("wallet_id".into(), cell("text", id.to_hex())),
                ("root".into(), cell("blob", hex::encode(serialized))),
            ])],
        )]);
        let (parsed, _) = root_key(&tables, &id, &WalletType::Byron, "EncryptWithArgon2idV2")?;
        let parsed = parsed.context("imported V2 key missing")?;
        ensure!(
            parsed.envelope() == v2.envelope(),
            "V2 envelope was altered"
        );
        ensure!(
            *parsed.decrypt(password)? == private.to_raw_bytes(),
            "V2 unlock changed key"
        );
        Ok(())
    }

    /// Explicitly selected owned reference fixture; never reads a user's wallet.
    #[test]
    #[ignore = "requires QUETZAL_OWNED_MIGRATION_FIXTURE with stopped native fixture"]
    fn stopped_reference_files_convert_and_authenticate() -> Result<()> {
        let root = std::path::PathBuf::from(std::env::var("QUETZAL_OWNED_MIGRATION_FIXTURE")?);
        let genesis: Json = serde_json::from_slice(&std::fs::read(
            root.join("runtime/cluster/byron-genesis.json"),
        )?)?;
        let shelley: Json = serde_json::from_slice(&std::fs::read(
            root.join("runtime/cluster/shelley-genesis.json"),
        )?)?;
        let clock = Clock {
            genesis_start: genesis["startTime"].as_u64().context("fixture start")?,
            eras: vec![quetzal_network::statequery::EraTiming {
                start_slot: 0,
                start_epoch: 0,
                relative_millis: 0,
                slot_millis: (shelley["slotLength"].as_f64().context("fixture slot")? * 1000.0)
                    as u64,
                epoch_length: shelley["epochLength"].as_u64().context("fixture epoch")?,
            }],
        };
        let genesis_hash = hex::encode(pallas_crypto::hash::Hasher::<256>::hash(
            &serde_json::to_vec(&genesis)?,
        ));
        let paths = crate::source::inventory(&root.join("reference/database"))?;
        ensure!(paths.len() == 5, "expected all five owned fixture families");
        let sqlite = std::path::PathBuf::from(option_env!("QUETZAL_SQLITE3").unwrap_or("sqlite3"));
        for path in paths {
            let snapshot = crate::source::Snapshot::copy(&path)?;
            let tables = snapshot.load(&sqlite)?;
            crate::source::version(&tables)?;
            let input = wallet(
                &tables,
                path.file_name()
                    .and_then(|s| s.to_str())
                    .context("fixture filename")?,
                &clock,
                &genesis_hash,
                pallas_addresses::Network::Testnet,
            )
            .with_context(|| format!("mapping fixture {}", path.display()))?;
            let discovered: HashSet<_> = input.addresses.iter().map(|a| &a.address).collect();
            let unique: HashSet<_> = input.utxos.iter().map(|u| &u.id).collect();
            let unowned = input
                .utxos
                .iter()
                .filter(|u| !discovered.contains(&u.address))
                .count();
            eprintln!(
                "fixture {:?}: {} discovery addresses, {} UTxOs, {} unowned, {} duplicates",
                input.metadata.wallet_type,
                input.addresses.len(),
                input.utxos.len(),
                unowned,
                input.utxos.len() - unique.len()
            );
            if let Some(key) = &input.credentials {
                let root = key.decrypt(b"public-native-migration-password")?;
                ensure!(
                    root.len() == 96 || root.len() == 128,
                    "unexpected unlocked root size"
                );
                ensure!(
                    key.decrypt(b"incorrect-migration-password").is_err(),
                    "wrong password authenticated"
                );
            }
            let target = tempfile::tempdir()?;
            {
                let db = quetzal_wallet::db::WalletDb::open(
                    target.path().to_str().context("fixture path")?,
                )?;
                db.import_wallet(&input).with_context(|| {
                    format!(
                        "importing fixture {} ({:?})",
                        path.display(),
                        input.metadata.wallet_type
                    )
                })?;
                db.flush()?;
            }
            let db = quetzal_wallet::db::WalletDb::open(
                target.path().to_str().context("fixture path")?,
            )?;
            crate::verify(&db, &input)?;
        }
        Ok(())
    }
}
