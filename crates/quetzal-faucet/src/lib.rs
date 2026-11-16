//! Development-only ADA funding over native Cardano node mini-protocols.
use anyhow::{bail, ensure, Context, Result};
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use pallas_addresses::{
    Address, Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart,
};
use pallas_codec::minicbor;
use pallas_crypto::{
    hash::Hasher,
    key::ed25519::{SecretKey, SecretKeyExtended},
};
use pallas_network::miniprotocols::localstate::queries_v16 as queries;
use pallas_primitives::{conway, Fragment, NonEmptySet};
use pallas_txbuilder::{BuildConway, Input, Output, StagingTransaction};
use quetzal_network::{statequery::ProtocolParams, NodeClient, StateQuerier, TxSubmitter};
use serde::{Deserialize, Serialize};
use std::{net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::Mutex;
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub node_socket: String,
    pub network_magic: u64,
    pub listen: SocketAddr,
    pub state_directory: PathBuf,
    pub max_lovelace: u64,
    pub sources: Vec<SourceConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    pub address: String,
    pub signing_key_file: PathBuf,
}

struct Source {
    address: Address,
    key: SigningKey,
}

// Pallas secret-key types scrub their storage on drop. Do not derive Debug.
enum SigningKey {
    Ordinary(SecretKey),
    Extended {
        secret: SecretKeyExtended,
        chain_code: Option<[u8; 32]>,
    },
}

impl SigningKey {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() == 32 {
            return Ok(Self::Ordinary(SecretKey::from(<[u8; 32]>::try_from(
                bytes,
            )?)));
        }
        ensure!(
            matches!(bytes.len(), 64 | 96 | 128),
            "unsupported signing key length"
        );
        let secret = SecretKeyExtended::from_bytes(bytes[..64].try_into()?)
            .context("invalid extended signing key scalar")?;
        if bytes.len() == 128 {
            let public: [u8; 32] = secret.public_key().into();
            ensure!(
                bytes[64..96] == public,
                "extended signing key public component mismatch"
            );
        }
        let chain_code = match bytes.len() {
            96 => Some(bytes[64..96].try_into()?),
            128 => Some(bytes[96..128].try_into()?),
            _ => None,
        };
        Ok(Self::Extended { secret, chain_code })
    }

    fn public_key(&self) -> [u8; 32] {
        match self {
            Self::Ordinary(key) => key.public_key().into(),
            Self::Extended { secret, .. } => secret.public_key().into(),
        }
    }

    fn sign(&self, message: &[u8]) -> [u8; 64] {
        let signature = match self {
            Self::Ordinary(key) => key.sign(message),
            Self::Extended { secret, .. } => secret.sign(message),
        };
        let mut bytes = [0; 64];
        bytes.copy_from_slice(signature.as_ref());
        bytes
    }

    fn chain_code(&self) -> Result<[u8; 32]> {
        match self {
            Self::Extended {
                chain_code: Some(code),
                ..
            } => Ok(*code),
            _ => bail!("Byron funding requires an extended key with chain code"),
        }
    }

    fn validate_source(&self, addr: &Address) -> Result<()> {
        use pallas_addresses::byron::{AddrType, AddressPayload, ByronAddress, SpendingData};
        let expected = match addr {
            Address::Shelley(_) => Address::Shelley(ShelleyAddress::new(
                Network::Testnet,
                ShelleyPaymentPart::key_hash(Hasher::<224>::hash(&self.public_key())),
                ShelleyDelegationPart::Null,
            )),
            Address::Byron(byron) => {
                let mut extended_public = self.public_key().to_vec();
                extended_public.extend_from_slice(&self.chain_code()?);
                let attributes = byron.decode()?.attributes;
                Address::Byron(ByronAddress::from_decoded(AddressPayload::new(
                    AddrType::PubKey,
                    SpendingData::PubKey(extended_public.into()),
                    attributes,
                )))
            }
            _ => bail!("unsupported funding source address"),
        };
        ensure!(
            addr.to_vec() == expected.to_vec(),
            "funding source address does not match signing key"
        );
        Ok(())
    }

    fn add_witness(
        &self,
        source: &Address,
        hash: &[u8],
        witnesses: &mut conway::WitnessSet,
    ) -> Result<()> {
        match source {
            Address::Byron(addr) => {
                witnesses.bootstrap_witness =
                    NonEmptySet::from_vec(vec![conway::BootstrapWitness {
                        public_key: self.public_key().to_vec().into(),
                        signature: self.sign(hash).to_vec().into(),
                        chain_code: self.chain_code()?.to_vec().into(),
                        attributes: minicbor::to_vec(addr.decode()?.attributes)?.into(),
                    }]);
            }
            Address::Shelley(_) => {
                witnesses.vkeywitness = NonEmptySet::from_vec(vec![conway::VKeyWitness {
                    vkey: self.public_key().to_vec().into(),
                    signature: self.sign(hash).to_vec().into(),
                }]);
            }
            _ => bail!("unsupported funding source address"),
        }
        Ok(())
    }
}

/// Conway testnet payment recipients, in lovelace; native assets are not minted.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Payment {
    pub address: String,
    pub lovelace: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FundRequest {
    pub payments: Vec<Payment>,
}

#[derive(Deserialize, Serialize)]
pub struct Receipt {
    pub transaction_id: String,
    pub fee: u64,
    pub lovelace: u64,
    pub status: String,
}

pub struct Faucet {
    config: Config,
    sources: Vec<Source>,
    reservations: sled::Db,
    serial: Mutex<()>,
}

/// Abort the Pallas mux even if a timed-out HTTP request drops its future.
struct Connection(Option<NodeClient>);
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(client) = self.0.take() {
            tokio::spawn(client.abort());
        }
    }
}

fn address(text: &str, magic: u64) -> Result<Address> {
    let parsed = if text.starts_with("addr1") || text.starts_with("addr_test1") {
        let (_, raw) = quetzal_address::codec::bech32_decode(text)?;
        Address::from_bytes(&raw)?
    } else {
        text.parse::<Address>()?
    };
    match &parsed {
        Address::Shelley(addr) => ensure!(addr.to_vec()[0] & 15 == 0, "recipient must be testnet"),
        Address::Byron(addr) => {
            let payload = addr.decode()?;
            ensure!(
                pallas_addresses::byron::ByronAddress::from_decoded(payload.clone()).to_vec()
                    == addr.to_vec(),
                "invalid Byron checksum"
            );
            let mut network = None;
            for attribute in payload.attributes.iter() {
                if let pallas_addresses::byron::AddrAttrProperty::NetworkTag(raw) = attribute {
                    ensure!(network.is_none(), "duplicate Byron network tag");
                    network = Some(minicbor::decode::<u32>(raw)?);
                }
            }
            ensure!(
                network.map(u64::from) == Some(magic),
                "Byron address network mismatch"
            );
        }
        _ => bail!("recipient must be a payment address"),
    }
    Ok(parsed)
}

fn read_key(path: &std::path::Path) -> Result<SigningKey> {
    let raw =
        Zeroizing::new(std::fs::read_to_string(path).context("cannot read signing key file")?);
    let bytes = if raw.trim_start().starts_with('{') {
        #[derive(Deserialize, zeroize::Zeroize, zeroize::ZeroizeOnDrop)]
        struct Envelope {
            #[serde(rename = "type")]
            kind: String,
            #[serde(rename = "cborHex")]
            cbor: String,
        }
        let envelope: Envelope =
            serde_json::from_str(&raw).context("invalid signing key envelope")?;
        let encoded =
            Zeroizing::new(hex::decode(&envelope.cbor).context("invalid signing key hex")?);
        let mut decoder = minicbor::Decoder::new(&encoded);
        let seed = Zeroizing::new(
            decoder
                .bytes()
                .context("invalid signing key CBOR")?
                .to_vec(),
        );
        ensure!(
            decoder.position() == encoded.len(),
            "trailing signing key bytes"
        );
        let valid_length = match envelope.kind.as_str() {
            "PaymentSigningKeyShelley_ed25519" => seed.len() == 32,
            "PaymentExtendedSigningKeyShelley_ed25519_bip32" => matches!(seed.len(), 96 | 128),
            "PaymentSigningKeyByron_ed25519_bip32" => seed.len() == 128,
            _ => bail!("unsupported signing key type"),
        };
        ensure!(valid_length, "signing key envelope length mismatch");
        seed
    } else {
        Zeroizing::new(hex::decode(raw.trim()).context("invalid seed hex")?)
    };
    SigningKey::from_bytes(&bytes)
}

impl Faucet {
    pub fn open(config: Config) -> Result<Arc<Self>> {
        ensure!(
            config.listen.ip().is_loopback(),
            "faucet must listen on loopback"
        );
        ensure!(
            config.network_magic != 764824073 && config.network_magic <= u64::from(u32::MAX),
            "testnet magic required"
        );
        ensure!(
            config.max_lovelace > 0 && !config.sources.is_empty(),
            "funding sources and positive limit required"
        );
        let mut sources = Vec::new();
        for source in &config.sources {
            let key = read_key(&source.signing_key_file)?;
            let addr = address(&source.address, config.network_magic)?;
            key.validate_source(&addr)?;
            sources.push(Source { address: addr, key });
        }
        let reservations = sled::open(&config.state_directory)?;
        Ok(Arc::new(Self {
            config,
            sources,
            reservations,
            serial: Mutex::new(()),
        }))
    }

    pub fn listen(&self) -> SocketAddr {
        self.config.listen
    }

    fn validate(&self, request: &FundRequest) -> Result<Vec<(Address, u64)>> {
        ensure!(
            !request.payments.is_empty() && request.payments.len() <= 32,
            "provide 1 to 32 payments"
        );
        let mut total = 0u64;
        let mut payments = Vec::new();
        for payment in &request.payments {
            ensure!(payment.lovelace > 0, "payment must be positive");
            total = total
                .checked_add(payment.lovelace)
                .context("payment amount overflow")?;
            payments.push((
                address(&payment.address, self.config.network_magic)?,
                payment.lovelace,
            ));
        }
        ensure!(
            total <= self.config.max_lovelace,
            "request exceeds configured limit"
        );
        Ok(payments)
    }

    async fn fund(&self, payments: Vec<(Address, u64)>) -> Result<Receipt> {
        let _serial = self.serial.lock().await;
        let mut connection = Connection(Some(
            NodeClient::connect(&self.config.node_socket, self.config.network_magic).await?,
        ));
        let client = connection.0.as_mut().context("missing node connection")?;
        let parameters = StateQuerier::new(client.statequery())
            .protocol_params()
            .await?;
        ensure!(
            parameters.era == 6,
            "faucet currently requires Conway query era 6"
        );
        ensure!(
            parameters.ada_per_utxo_byte > 0 && parameters.max_tx_size > 0,
            "incomplete node parameters"
        );
        let (slot, _, _) = StateQuerier::new(client.statequery())
            .chain_header()
            .await?;
        let expiry = slot.checked_add(3600).context("slot overflow")?;
        for source in &self.sources {
            let query = client.statequery();
            query.acquire(None).await?;
            let found = queries::get_utxo_by_address(
                query,
                parameters.era,
                vec![source.address.to_vec().into()],
            )
            .await;
            query.send_release().await?;
            for (input, output) in found?.utxo.iter() {
                let (addr, amount) = match output {
                    queries::TransactionOutput::Current(o) => (&o.address, &o.amount),
                    queries::TransactionOutput::Legacy(o) => (&o.address, &o.amount),
                };
                ensure!(
                    addr.as_slice() == source.address.to_vec(),
                    "node returned unexpected source address"
                );
                let queries::Value::Coin(coin) = amount else {
                    continue;
                };
                let index = u64::from(&input.index);
                let key = format!("{}#{index}", input.transaction_id);
                if self.reservations.contains_key(key.as_bytes())? {
                    continue;
                }
                let Some((cbor, fee, id)) = build_payment(
                    *input.transaction_id,
                    index,
                    u64::from(coin),
                    &source.address,
                    &source.key,
                    &payments,
                    &parameters,
                    expiry,
                )?
                else {
                    continue;
                };
                reserve(&self.reservations, &key, &cbor)?;
                let result = TxSubmitter::new(client.submission())
                    .submit_era(parameters.era, cbor)
                    .await;
                if matches!(&result, Err(quetzal_network::NetworkError::TxRejected(_))) {
                    self.reservations.remove(key.as_bytes())?;
                    self.reservations.flush()?;
                }
                result?;
                return Ok(Receipt {
                    transaction_id: id,
                    fee,
                    lovelace: payments.iter().map(|(_, coin)| coin).sum(),
                    status: "accepted".into(),
                });
            }
        }
        bail!("no unreserved ADA-only funding UTxO large enough for payment and change")
    }
}

fn reserve(db: &sled::Db, input: &str, cbor: &[u8]) -> Result<()> {
    ensure!(
        db.compare_and_swap(input.as_bytes(), None as Option<&[u8]>, Some(cbor))?
            .is_ok(),
        "input already reserved"
    );
    db.flush()?;
    Ok(())
}

type BuiltPayment = (Vec<u8>, u64, String);
#[allow(clippy::too_many_arguments)]
fn build_payment(
    hash: [u8; 32],
    index: u64,
    balance: u64,
    change: &Address,
    key: &SigningKey,
    payments: &[(Address, u64)],
    params: &ProtocolParams,
    expiry: u64,
) -> Result<Option<BuiltPayment>> {
    let total = payments.iter().try_fold(0u64, |sum, (_, coin)| {
        sum.checked_add(*coin).context("payment overflow")
    })?;
    let mut fee = params.min_fee_b;
    for _ in 0..16 {
        let Some(remainder) = balance.checked_sub(total).and_then(|c| c.checked_sub(fee)) else {
            return Ok(None);
        };
        let mut staging = StagingTransaction::new()
            .network_id(0)
            .fee(fee)
            .invalid_from_slot(expiry)
            .input(Input::new(hash.into(), index));
        for (address, coin) in payments {
            staging = staging.output(Output::new(address.clone(), *coin));
        }
        staging = staging.output(Output::new(change.clone(), remainder));
        let encoded = staging
            .build_conway_raw()
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .tx_bytes
            .0;
        let mut tx = conway::Tx::decode_fragment(&encoded).map_err(|e| anyhow::anyhow!("{e}"))?;
        for (index, output) in tx.transaction_body.outputs.iter().enumerate() {
            let minimum = (160u64 + minicbor::to_vec(output)?.len() as u64)
                .checked_mul(params.ada_per_utxo_byte)
                .context("minimum ADA overflow")?;
            let amount = if index < payments.len() {
                payments[index].1
            } else {
                remainder
            };
            if index == payments.len() && amount < minimum {
                return Ok(None);
            }
            ensure!(
                amount >= minimum,
                "payment below live minimum ADA: {minimum}"
            );
        }
        let body = minicbor::to_vec(&tx.transaction_body)?;
        let id = Hasher::<256>::hash(&body);
        key.add_witness(change, id.as_ref(), &mut tx.transaction_witness_set)?;
        let signed = tx.encode_fragment().map_err(|e| anyhow::anyhow!("{e}"))?;
        ensure!(
            signed.len() as u64 <= params.max_tx_size,
            "transaction exceeds native size limit"
        );
        let minimum_fee = params
            .min_fee_a
            .checked_mul(signed.len() as u64)
            .and_then(|n| n.checked_add(params.min_fee_b))
            .context("fee overflow")?;
        if fee >= minimum_fee {
            return Ok(Some((signed, fee, id.to_string())));
        }
        fee = minimum_fee;
    }
    bail!("transaction fee did not converge")
}

pub fn router(faucet: Arc<Faucet>) -> Router {
    Router::new()
        .route(
            "/health",
            get(|| async { Json(serde_json::json!({"status":"ready"})) }),
        )
        .route("/fund", post(fund))
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024))
        .with_state(faucet)
}

async fn fund(
    State(faucet): State<Arc<Faucet>>,
    Json(request): Json<FundRequest>,
) -> axum::response::Response {
    let payments = match faucet.validate(&request) {
        Ok(payments) => payments,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({"error":error.to_string()})),
            )
                .into_response()
        }
    };
    match tokio::time::timeout(Duration::from_secs(30), faucet.fund(payments)).await {
        Ok(Ok(receipt)) => (StatusCode::ACCEPTED, Json(receipt)).into_response(),
        Ok(Err(error)) => {
            eprintln!("faucet funding failed: {error}");
            (StatusCode::SERVICE_UNAVAILABLE, Json(serde_json::json!({"error":"funding failed; inspect faucet logs before retrying"}))).into_response()
        }
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(
                serde_json::json!({"error":"node operation timed out; input may remain reserved"}),
            ),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn sender() -> Address {
        let public: [u8; 32] = SecretKey::from([7; 32]).public_key().into();
        Address::Shelley(ShelleyAddress::new(
            Network::Testnet,
            ShelleyPaymentPart::key_hash(Hasher::<224>::hash(&public)),
            ShelleyDelegationPart::Null,
        ))
    }

    fn config(root: &std::path::Path) -> Config {
        let key = root.join("seed");
        std::fs::write(&key, hex::encode([7; 32])).unwrap();
        Config {
            node_socket: root.join("absent.socket").to_string_lossy().into_owned(),
            network_magic: 42,
            listen: "127.0.0.1:0".parse().unwrap(),
            state_directory: root.join("reservations"),
            max_lovelace: 100_000_000,
            sources: vec![SourceConfig {
                address: sender().to_bech32().unwrap(),
                signing_key_file: key,
            }],
        }
    }

    fn params() -> ProtocolParams {
        ProtocolParams {
            min_fee_a: 44,
            min_fee_b: 155381,
            max_tx_size: 16384,
            ada_per_utxo_byte: 4310,
            key_deposit: 0,
            pool_deposit: 0,
            desired_pool_number: 0,
            max_collateral_inputs: 0,
            collateral_percentage: 0,
            max_value_size: 5000,
            memory_price: 0.0,
            step_price: 0.0,
            era: 6,
        }
    }

    fn byron() -> Address {
        use pallas_addresses::byron::{
            AddrAttrProperty, AddrAttrs, AddrType, AddressPayload, ByronAddress, SpendingData,
        };
        let attributes: AddrAttrs = vec![AddrAttrProperty::NetworkTag(
            minicbor::to_vec(42u32).unwrap().into(),
        )]
        .into();
        Address::Byron(ByronAddress::from_decoded(AddressPayload::new(
            AddrType::PubKey,
            SpendingData::PubKey(vec![5; 64].into()),
            attributes,
        )))
    }

    #[test]
    fn byron_encodings_preserve_bytes_and_reject_wrong_network() {
        let original = byron();
        let base58 = quetzal_address::codec::base58_encode(&original.to_vec());
        let wrapped =
            quetzal_address::codec::bech32_encode("addr_test", &original.to_vec()).unwrap();
        assert_eq!(address(&base58, 42).unwrap().to_vec(), original.to_vec());
        assert_eq!(address(&wrapped, 42).unwrap().to_vec(), original.to_vec());
        assert!(address(&base58, 43).is_err());
        let mut corrupt = original.to_vec();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(address(&quetzal_address::codec::base58_encode(&corrupt), 42).is_err());
    }

    #[test]
    fn signed_payment_conserves_coin_pays_fee_and_has_valid_signature() {
        for destination in [sender(), byron()] {
            let (bytes, fee, id) = build_payment(
                [1; 32],
                3,
                100_000_000,
                &sender(),
                &SigningKey::Ordinary(SecretKey::from([7; 32])),
                &[(destination.clone(), 30_000_000)],
                &params(),
                999,
            )
            .unwrap()
            .unwrap();
            let tx = conway::Tx::decode_fragment(&bytes).unwrap();
            assert_eq!(tx.transaction_body.ttl, Some(999));
            assert_eq!(tx.transaction_body.inputs.iter().next().unwrap().index, 3);
            assert!(fee >= params().min_fee_a * bytes.len() as u64 + params().min_fee_b);
            let mut total = fee;
            for (index, output) in tx.transaction_body.outputs.iter().enumerate() {
                let conway::TransactionOutput::PostAlonzo(output) = output else {
                    panic!("unexpected output")
                };
                let conway::Value::Coin(coin) = output.value else {
                    panic!("unexpected assets")
                };
                total += coin;
                if index == 0 {
                    assert_eq!(output.address.as_slice(), destination.to_vec());
                    assert_eq!(coin, 30_000_000);
                }
            }
            assert_eq!(total, 100_000_000);
            let hash = Hasher::<256>::hash(&minicbor::to_vec(&tx.transaction_body).unwrap());
            assert_eq!(hash.to_string(), id);
            let witnesses = tx.transaction_witness_set.vkeywitness.unwrap().to_vec();
            let key: [u8; 32] = witnesses[0].vkey.as_slice().try_into().unwrap();
            let signature: [u8; 64] = witnesses[0].signature.as_slice().try_into().unwrap();
            assert!(quetzal_crypto::signing::verify(
                &key,
                hash.as_ref(),
                &signature
            ));
        }
        assert!(build_payment(
            [1; 32],
            0,
            100,
            &sender(),
            &SigningKey::Ordinary(SecretKey::from([7; 32])),
            &[(sender(), 30_000_000)],
            &params(),
            999
        )
        .unwrap()
        .is_none());
        assert!(build_payment(
            [1; 32],
            0,
            100_000_000,
            &sender(),
            &SigningKey::Ordinary(SecretKey::from([7; 32])),
            &[(sender(), 1)],
            &params(),
            999
        )
        .is_err());
    }

    #[test]
    fn reservations_survive_restart_and_cannot_be_overwritten() {
        let root = tempfile::tempdir().unwrap();
        {
            let db = sled::open(root.path()).unwrap();
            reserve(&db, "input#0", b"signed transaction").unwrap();
            assert!(reserve(&db, "input#0", b"different transaction").is_err());
        }
        let db = sled::open(root.path()).unwrap();
        assert_eq!(
            db.get("input#0").unwrap().unwrap().as_ref(),
            b"signed transaction"
        );
    }

    fn extended_layouts() -> Vec<Vec<u8>> {
        let mut secret = vec![4; 64];
        secret[0] &= 248;
        secret[31] = (secret[31] & 63) | 64;
        let key = SigningKey::from_bytes(&secret).unwrap();
        let mut with_chain = secret.clone();
        with_chain.extend_from_slice(&[9; 32]);
        let mut legacy = secret.clone();
        legacy.extend_from_slice(&key.public_key());
        legacy.extend_from_slice(&[9; 32]);
        vec![secret, with_chain, legacy]
    }

    #[test]
    fn extended_layouts_sign_payments_and_validate_embedded_public_keys() {
        for bytes in extended_layouts() {
            let key = SigningKey::from_bytes(&bytes).unwrap();
            let source = Address::Shelley(ShelleyAddress::new(
                Network::Testnet,
                ShelleyPaymentPart::key_hash(Hasher::<224>::hash(&key.public_key())),
                ShelleyDelegationPart::Null,
            ));
            let (signed, _, id) = build_payment(
                [2; 32],
                0,
                100_000_000,
                &source,
                &key,
                &[(byron(), 30_000_000)],
                &params(),
                999,
            )
            .unwrap()
            .unwrap();
            let tx = conway::Tx::decode_fragment(&signed).unwrap();
            let hash = Hasher::<256>::hash(&minicbor::to_vec(&tx.transaction_body).unwrap());
            assert_eq!(hash.to_string(), id);
            let witness = tx
                .transaction_witness_set
                .vkeywitness
                .unwrap()
                .to_vec()
                .remove(0);
            assert_eq!(witness.vkey.as_slice(), key.public_key());
            assert!(quetzal_crypto::signing::verify(
                &key.public_key(),
                hash.as_ref(),
                &witness.signature.as_slice().try_into().unwrap()
            ));
        }
        let mut wrong_public = extended_layouts().pop().unwrap();
        wrong_public[64] ^= 1;
        assert!(SigningKey::from_bytes(&wrong_public).is_err());
        assert!(SigningKey::from_bytes(&[0; 64]).is_err());
        assert!(SigningKey::from_bytes(&[0; 65]).is_err());
    }

    #[test]
    fn extended_envelopes_validate_type_length_cbor_and_source_identity() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("extended.key");
        let layouts = extended_layouts();
        for (kind, bytes) in [
            ("PaymentSigningKeyByron_ed25519_bip32", &layouts[2]),
            (
                "PaymentExtendedSigningKeyShelley_ed25519_bip32",
                &layouts[1],
            ),
            (
                "PaymentExtendedSigningKeyShelley_ed25519_bip32",
                &layouts[2],
            ),
        ] {
            let encoded =
                minicbor::to_vec(pallas_codec::minicbor::bytes::ByteVec::from(bytes.to_vec()))
                    .unwrap();
            std::fs::write(
                &path,
                serde_json::json!({"type":kind,"cborHex":hex::encode(&encoded)}).to_string(),
            )
            .unwrap();
            let key = read_key(&path).unwrap();
            assert_eq!(
                key.public_key(),
                SigningKey::from_bytes(bytes).unwrap().public_key()
            );
            let mut c = config(root.path());
            c.sources[0].signing_key_file = path.clone();
            assert!(Faucet::open(c).is_err());
            let mut c = config(root.path());
            c.sources[0].signing_key_file = path.clone();
            c.sources[0].address = ShelleyAddress::new(
                Network::Testnet,
                ShelleyPaymentPart::key_hash(Hasher::<224>::hash(&key.public_key())),
                ShelleyDelegationPart::Null,
            )
            .to_bech32()
            .unwrap();
            drop(Faucet::open(c).unwrap());
            let mut trailing = encoded;
            trailing.push(0);
            std::fs::write(
                &path,
                serde_json::json!({"type":kind,"cborHex":hex::encode(trailing)}).to_string(),
            )
            .unwrap();
            assert!(read_key(&path).is_err());
        }
        for kind in ["PaymentSigningKeyShelley_ed25519", "UnknownSigningKey"] {
            let encoded = minicbor::to_vec(pallas_codec::minicbor::bytes::ByteVec::from(
                layouts[2].clone(),
            ))
            .unwrap();
            std::fs::write(
                &path,
                serde_json::json!({"type":kind,"cborHex":hex::encode(encoded)}).to_string(),
            )
            .unwrap();
            assert!(read_key(&path).is_err());
        }
    }

    #[test]
    fn byron_funding_validates_root_and_preserves_bootstrap_attributes() {
        use pallas_addresses::byron::{
            AddrAttrProperty, AddrAttrs, AddrType, AddressPayload, ByronAddress, SpendingData,
        };
        let bytes = extended_layouts().pop().unwrap();
        let key = SigningKey::from_bytes(&bytes).unwrap();
        let attributes: AddrAttrs = vec![
            AddrAttrProperty::NetworkTag(minicbor::to_vec(42u32).unwrap().into()),
            AddrAttrProperty::DerivationPath(vec![5; 32].into()),
        ]
        .into();
        let mut public = key.public_key().to_vec();
        public.extend_from_slice(&key.chain_code().unwrap());
        let source = Address::Byron(ByronAddress::from_decoded(AddressPayload::new(
            AddrType::PubKey,
            SpendingData::PubKey(public.into()),
            attributes.clone(),
        )));
        key.validate_source(&source).unwrap();
        assert!(key.validate_source(&byron()).is_err());
        let mut wrong_chain = bytes.clone();
        wrong_chain[96] ^= 1;
        assert!(SigningKey::from_bytes(&wrong_chain)
            .unwrap()
            .validate_source(&source)
            .is_err());
        assert!(SigningKey::from_bytes(&bytes[..64])
            .unwrap()
            .validate_source(&source)
            .is_err());
        let (signed, fee, _) = build_payment(
            [3; 32],
            0,
            100_000_000,
            &source,
            &key,
            &[(sender(), 30_000_000)],
            &params(),
            999,
        )
        .unwrap()
        .unwrap();
        assert!(fee >= params().min_fee_a * signed.len() as u64 + params().min_fee_b);
        let tx = conway::Tx::decode_fragment(&signed).unwrap();
        let hash = Hasher::<256>::hash(&minicbor::to_vec(&tx.transaction_body).unwrap());
        assert!(tx.transaction_witness_set.vkeywitness.is_none());
        let witness = tx
            .transaction_witness_set
            .bootstrap_witness
            .unwrap()
            .to_vec()
            .remove(0);
        assert_eq!(witness.public_key.as_slice(), key.public_key());
        assert_eq!(witness.chain_code.as_slice(), key.chain_code().unwrap());
        assert_eq!(
            witness.attributes.as_slice(),
            minicbor::to_vec(attributes).unwrap()
        );
        assert!(quetzal_crypto::signing::verify(
            &key.public_key(),
            hash.as_ref(),
            &witness.signature.as_slice().try_into().unwrap()
        ));
    }

    #[test]
    #[ignore = "requires explicit private native funding-source config"]
    fn real_extended_fixture_sources_match_their_funding_addresses() {
        let path = std::env::var_os("QUETZAL_FAUCET_SOURCE_CONFIG")
            .expect("set QUETZAL_FAUCET_SOURCE_CONFIG");
        let config: Config = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(!config.sources.is_empty());
        let count = config.sources.len();
        let faucet = Faucet::open(config).unwrap();
        assert_eq!(faucet.sources.len(), count);
        assert!(faucet
            .sources
            .iter()
            .all(|source| matches!(source.key, SigningKey::Extended { .. })));
    }

    #[test]
    fn concurrent_reservations_have_only_one_winner() {
        let root = tempfile::tempdir().unwrap();
        let db = sled::open(root.path()).unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let attempts: Vec<_> = (0..8)
            .map(|_| {
                let db = db.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    reserve(&db, "same-input#0", b"signed").is_ok()
                })
            })
            .collect();
        assert_eq!(
            attempts
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[tokio::test]
    async fn cancelled_submission_keeps_durable_input_reservation() {
        let root = tempfile::tempdir().unwrap();
        let db = sled::open(root.path()).unwrap();
        let (ready, reserved) = tokio::sync::oneshot::channel();
        let task_db = db.clone();
        let task = tokio::spawn(async move {
            reserve(&task_db, "possibly-submitted#0", b"signed transaction").unwrap();
            ready.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        reserved.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(db);
        let reopened = sled::open(root.path()).unwrap();
        assert_eq!(
            reopened
                .get("possibly-submitted#0")
                .unwrap()
                .unwrap()
                .as_ref(),
            b"signed transaction"
        );
        assert!(reserve(&reopened, "possibly-submitted#0", b"retry").is_err());
    }

    #[test]
    fn config_rejects_mainnet_exposure_and_mismatched_key() {
        let root = tempfile::tempdir().unwrap();
        let mut c = config(root.path());
        c.listen = "0.0.0.0:19999".parse().unwrap();
        assert!(Faucet::open(c).is_err());
        let mut c = config(root.path());
        c.network_magic = 764824073;
        assert!(Faucet::open(c).is_err());
        let c = config(root.path());
        std::fs::write(&c.sources[0].signing_key_file, hex::encode([8; 32])).unwrap();
        assert!(Faucet::open(c).is_err());
    }

    #[tokio::test]
    async fn http_validates_before_connecting_and_never_claims_acceptance_offline() {
        let root = tempfile::tempdir().unwrap();
        let faucet = Faucet::open(config(root.path())).unwrap();
        let app = router(faucet);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        for request in [
            serde_json::json!({"payments":[]}),
            serde_json::json!({"payments":[{"address":"invalid","lovelace":1}]}),
            serde_json::json!({"payments":[{"address":sender().to_bech32().unwrap(),"lovelace":100_000_001}]}),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/fund")
                        .header("content-type", "application/json")
                        .body(Body::from(request.to_string()))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let request = serde_json::json!({"payments":[{"address":sender().to_bech32().unwrap(),"lovelace":2_000_000}]});
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/fund")
                    .header("content-type", "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
