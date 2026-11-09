//! Conway registry queries pinned to ouroboros-consensus 4.2.1.0 and ledger
//! f4a3026c9bae (DRepState's expiry, anchor, deposit and delegator set).
use crate::{NetworkError, StateQuerier};
use pallas_codec::utils::{AnyCbor, Bytes, KeyValuePairs, TagWrap};
use pallas_network::miniprotocols::localstate::{queries_v16 as q, ClientError};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct DRepRegistration {
    pub kind: u8,
    pub hash: [u8; 28],
    pub expiry_epoch: u64,
    pub deposit: u64,
    pub voting_power: u64,
    pub anchor: Option<(String, [u8; 32])>,
}

#[derive(Debug)]
pub struct DRepSnapshot {
    pub epoch: u64,
    pub registrations: Vec<DRepRegistration>,
}

type Credential = (u8, Bytes);
type Registry = BTreeMap<Credential, (u64, Vec<(String, Bytes)>, u64, AnyCbor)>;

impl StateQuerier<'_> {
    pub async fn dreps(&mut self) -> Result<DRepSnapshot, NetworkError> {
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result = async {
            let era = q::get_current_era(self.client).await?;
            if era < 6 {
                return Ok::<_, ClientError>((0, BTreeMap::new(), KeyValuePairs::from(Vec::new())));
            }
            let (epoch,): (u64,) = self
                .client
                .query(q::Request::LedgerQuery(q::LedgerQuery::BlockQuery(
                    era,
                    q::BlockQuery::GetEpochNo,
                )))
                .await?;
            // Request -> LedgerQuery -> BlockQuery envelope, as in Pallas v16.
            let request = |tag: u8| {
                AnyCbor::from_encode((
                    0u8,
                    (
                        0u8,
                        (era, (tag, TagWrap::<_, 258>::new(Vec::<Credential>::new()))),
                    ),
                ))
            };
            let (states,): (Registry,) = self.client.query(request(25)).await?;
            let (distribution,): (KeyValuePairs<AnyCbor, u64>,) =
                self.client.query(request(26)).await?;
            Ok((epoch, states, distribution))
        }
        .await;
        let release = self.client.send_release().await;
        let (epoch, states, distribution) =
            result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let invalid = |message: &str| NetworkError::StateQuery(message.into());
        let mut powers = BTreeMap::new();
        for (key, coin) in distribution.iter() {
            let mut decoder = pallas_codec::minicbor::Decoder::new(key.raw_bytes());
            let length = decoder.array().map_err(|e| invalid(&e.to_string()))?;
            let kind = decoder.u8().map_err(|e| invalid(&e.to_string()))?;
            match (length, kind) {
                (Some(2), 0 | 1) => {
                    let hash = decoder.bytes().map_err(|e| invalid(&e.to_string()))?;
                    if hash.len() != 28 {
                        return Err(invalid("invalid DRep distribution hash"));
                    }
                    powers.insert((kind, Bytes::from(hash.to_vec())), *coin);
                }
                (Some(1), 2 | 3) => {}
                _ => return Err(invalid("invalid DRep distribution credential")),
            }
        }
        let mut registrations = Vec::with_capacity(states.len());
        for ((kind, hash), (expiry_epoch, anchor, deposit, _delegators)) in states {
            if kind > 1 || hash.len() != 28 || anchor.len() > 1 {
                return Err(invalid("invalid DRep registration"));
            }
            let voting_power = powers.remove(&(kind, hash.clone())).unwrap_or(0);
            let anchor = anchor
                .into_iter()
                .next()
                .map(|(url, hash)| {
                    let hash = hash
                        .as_slice()
                        .try_into()
                        .map_err(|_| invalid("invalid DRep anchor hash"))?;
                    Ok::<_, NetworkError>((url, hash))
                })
                .transpose()?;
            registrations.push(DRepRegistration {
                kind,
                hash: hash
                    .as_slice()
                    .try_into()
                    .map_err(|_| invalid("invalid DRep hash"))?,
                expiry_epoch,
                deposit,
                voting_power,
                anchor,
            });
        }
        Ok(DRepSnapshot {
            epoch,
            registrations,
        })
    }
}
