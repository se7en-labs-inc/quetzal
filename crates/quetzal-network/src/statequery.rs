use pallas_network::miniprotocols::localstate::queries_v16::{
    self, Coin, HardForkQuery, LedgerQuery, Request,
};
use pallas_network::miniprotocols::localstate::Client;

use crate::NetworkError;

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct EraTiming {
    pub start_slot: u64,
    pub start_epoch: u64,
    pub relative_millis: u64,
    pub slot_millis: u64,
    pub epoch_length: u64,
}

#[derive(Debug, Clone, Copy)]
pub struct NativeGenesis {
    pub security_parameter: u64,
    pub active_slot_coefficient: f64,
}

fn decode_era_timings(bytes: &[u8]) -> Result<Vec<EraTiming>, NetworkError> {
    use ciborium::Value;
    fn error() -> NetworkError {
        NetworkError::StateQuery("invalid native era interpreter".into())
    }
    fn array(v: &Value) -> Result<&[Value], NetworkError> {
        v.as_array().map(Vec::as_slice).ok_or_else(error)
    }
    fn number(v: &Value) -> Result<u128, NetworkError> {
        match v {
            Value::Integer(i) => u128::try_from(*i).map_err(|_| error()),
            Value::Tag(2, inner) => {
                inner
                    .as_bytes()
                    .ok_or_else(error)?
                    .iter()
                    .try_fold(0u128, |n, b| {
                        n.checked_mul(256)
                            .and_then(|n| n.checked_add(u128::from(*b)))
                            .ok_or_else(error)
                    })
            }
            _ => Err(error()),
        }
    }
    let raw: Value =
        ciborium::from_reader(bytes).map_err(|e| NetworkError::StateQuery(e.to_string()))?;
    let mut result = Vec::new();
    for summary in array(&raw)? {
        let fields = array(summary)?;
        let start = array(fields.first().ok_or_else(error)?)?;
        let parameters = array(fields.get(2).ok_or_else(error)?)?;
        if start.len() < 3 || parameters.len() < 2 {
            return Err(error());
        }
        let convert = |n: u128| u64::try_from(n).map_err(|_| error());
        // Bound RelativeTime uses Fixed Pico; SlotLength is integer milliseconds.
        let era = EraTiming {
            relative_millis: convert(number(&start[0])? / 1_000_000_000)?,
            start_slot: convert(number(&start[1])?)?,
            start_epoch: convert(number(&start[2])?)?,
            epoch_length: convert(number(&parameters[0])?)?,
            slot_millis: convert(number(&parameters[1])?)?,
        };
        if era.slot_millis == 0 || era.epoch_length == 0 {
            return Err(error());
        }
        if result.last().is_some_and(|last: &EraTiming| {
            last.start_slot > era.start_slot || last.relative_millis > era.relative_millis
        }) {
            return Err(error());
        }
        result.push(era);
    }
    if result.is_empty() {
        return Err(error());
    }
    Ok(result)
}

#[cfg(test)]
mod era_clock_tests {
    use super::*;
    use ciborium::Value;
    fn number(n: u128) -> Value {
        if let Ok(n) = u64::try_from(n) {
            Value::Integer(n.into())
        } else {
            Value::Tag(2, Box::new(Value::Bytes(n.to_be_bytes().to_vec())))
        }
    }
    #[test]
    fn native_interpreter_decodes_picosecond_bignums_and_millisecond_slot_lengths() {
        let summary = |time, slot, epoch, length, size| {
            Value::Array(vec![
                Value::Array(vec![number(time), number(slot), number(epoch)]),
                Value::Null,
                Value::Array(vec![number(size), number(length), Value::Null, number(0)]),
            ])
        };
        let raw = Value::Array(vec![
            summary(0, 0, 0, 20000, 21600),
            summary(89_856_000_000_000_000_000, 4_492_800, 208, 1000, 432000),
        ]);
        let mut bytes = Vec::new();
        ciborium::into_writer(&raw, &mut bytes).unwrap();
        let eras = decode_era_timings(&bytes).unwrap();
        assert_eq!(eras[0].slot_millis, 20000);
        assert_eq!(
            eras[1],
            EraTiming {
                start_slot: 4_492_800,
                start_epoch: 208,
                relative_millis: 89_856_000_000,
                slot_millis: 1000,
                epoch_length: 432000
            }
        );
        let invalid = Value::Array(vec![summary(0, 0, 0, 0, 120)]);
        bytes.clear();
        ciborium::into_writer(&invalid, &mut bytes).unwrap();
        assert!(decode_era_timings(&bytes).is_err());
    }
}

/// Protocol parameters extracted from the node.
#[derive(Debug, Clone)]
pub struct ProtocolParams {
    pub min_fee_a: u64,
    pub min_fee_b: u64,
    pub max_tx_size: u64,
    pub ada_per_utxo_byte: u64,
    pub key_deposit: u64,
    pub pool_deposit: u64,
    pub desired_pool_number: u32,
    pub max_collateral_inputs: u32,
    pub collateral_percentage: u32,
    pub max_value_size: u32,
    pub memory_price: f64,
    pub step_price: f64,
    pub era: u16,
}

/// Convert a pallas `AnyUInt` coin value to a plain `u64`.
///
/// `AnyUInt` implements `From<&AnyUInt> for u64`, so we just use that.
fn coin_to_u64(c: &Coin) -> u64 {
    u64::from(c)
}

/// Queries the connected cardano-node for ledger state using the
/// local-state-query mini-protocol.
pub struct StateQuerier<'a> {
    pub(crate) client: &'a mut Client,
}

/// Ledger rewards and active pool delegation for a stake credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StakeAccount {
    pub registered: bool,
    pub deposit: Option<u64>,
    pub reward_balance: u64,
    pub pool_id: Option<Vec<u8>>,
}

impl<'a> StateQuerier<'a> {
    /// Shelley genesis parameters come from the selected node, not an adjacent file.
    pub async fn genesis_config(&mut self) -> Result<Option<NativeGenesis>, NetworkError> {
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result = async {
            let era = queries_v16::get_current_era(self.client).await?;
            if era == 0 {
                return Ok(None);
            }
            let values = queries_v16::get_genesis_config(self.client, era).await?;
            Ok::<_, pallas_network::miniprotocols::localstate::ClientError>(
                values.into_iter().next(),
            )
        }
        .await;
        let release = self.client.send_release().await;
        let value = result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        value
            .map(|genesis| {
                let fraction = genesis.active_slots_coefficient;
                if fraction.den == 0 || fraction.num == 0 || fraction.num > fraction.den {
                    return Err(NetworkError::StateQuery(
                        "invalid native active slot coefficient".into(),
                    ));
                }
                Ok(NativeGenesis {
                    security_parameter: u64::from(genesis.security_param),
                    active_slot_coefficient: fraction.num as f64 / fraction.den as f64,
                })
            })
            .transpose()
    }
    /// Read the point and height from the same acquired native chain snapshot.
    pub async fn chain_header(&mut self) -> Result<(u64, Vec<u8>, u64), NetworkError> {
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result = async {
            let point = queries_v16::get_chain_point(self.client).await?;
            let height = queries_v16::get_chain_block_no(self.client).await?;
            Ok::<_, pallas_network::miniprotocols::localstate::ClientError>((point, height))
        }
        .await;
        let release = self.client.send_release().await;
        let (point, height) = result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let (slot, hash) = match point {
            pallas_network::miniprotocols::Point::Origin => (0, vec![0; 32]),
            pallas_network::miniprotocols::Point::Specific(slot, hash) => (slot, hash),
        };
        Ok((slot, hash, u64::from(height.block_number)))
    }

    /// Query era without decoding any era-specific ledger payload.
    pub async fn current_era(&mut self) -> Result<u16, NetworkError> {
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let era = queries_v16::get_current_era(self.client).await;
        let release = self.client.send_release().await;
        let era = era.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        Ok(era)
    }

    /// Query key (0) or script (1) stake credentials at one acquired ledger tip.
    /// Results retain the order of the requested credentials. An absent reward
    /// account is unregistered and has no rewards or active delegation.
    pub async fn stake_accounts(
        &mut self,
        credentials: &[(u8, [u8; 28])],
    ) -> Result<Vec<StakeAccount>, NetworkError> {
        if credentials.iter().any(|(kind, _)| *kind > 1) {
            return Err(NetworkError::StateQuery(
                "invalid stake credential type".into(),
            ));
        }
        let addresses: Vec<queries_v16::StakeAddr> = credentials
            .iter()
            .map(|(kind, hash)| (*kind, hash.to_vec().into()).into())
            .collect();
        // Ledger Credential's Ord places ScriptHashObj before KeyHashObj,
        // although their CBOR discriminators are 1 and 0 respectively.
        let mut deposit_credentials = credentials.to_vec();
        deposit_credentials.sort_by_key(|(kind, hash)| (std::cmp::Reverse(*kind), *hash));
        deposit_credentials.dedup();
        let deposit_credentials: Vec<queries_v16::StakeAddr> = deposit_credentials
            .into_iter()
            .map(|(kind, hash)| (kind, hash.to_vec().into()).into())
            .collect();
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result = async {
            let era = queries_v16::get_current_era(self.client).await?;
            let accounts = queries_v16::get_filtered_delegations_rewards(
                self.client,
                era,
                addresses.iter().cloned().collect(),
            )
            .await?;
            let query = queries_v16::BlockQuery::GetStakeDelegDeposits(
                pallas_codec::utils::AnyCbor::from_encode(
                    pallas_codec::utils::TagWrap::<_, 258>::new(deposit_credentials),
                ),
            );
            let (deposits,): (std::collections::BTreeMap<queries_v16::StakeAddr, u64>,) = self
                .client
                .query(Request::LedgerQuery(LedgerQuery::BlockQuery(era, query)))
                .await?;
            Ok::<_, pallas_network::miniprotocols::localstate::ClientError>((accounts, deposits))
        }
        .await;
        let release = self.client.send_release().await;
        let (accounts, deposits) = result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        Ok(addresses
            .iter()
            .map(|address| StakeAccount {
                registered: accounts.rewards.iter().any(|(key, _)| key == address),
                deposit: deposits.get(address).copied(),
                reward_balance: accounts
                    .rewards
                    .iter()
                    .find(|(key, _)| key == address)
                    .map_or(0, |(_, value)| *value),
                pool_id: accounts
                    .delegs
                    .iter()
                    .find(|(key, _)| key == address)
                    .map(|(_, value)| value.to_vec()),
            })
            .collect())
    }
    /// Start slot and epoch for each hard-fork era, in consensus order.
    pub async fn era_history(&mut self) -> Result<Vec<(u64, u64)>, NetworkError> {
        Ok(self
            .era_timings()
            .await?
            .into_iter()
            .map(|era| (era.start_slot, era.start_epoch))
            .collect())
    }

    /// Native hard-fork clock; no genesis filename convention is involved.
    pub async fn era_timings(&mut self) -> Result<Vec<EraTiming>, NetworkError> {
        use pallas_codec::utils::AnyCbor;
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result: Result<AnyCbor, _> = self
            .client
            .query(Request::LedgerQuery(LedgerQuery::HardForkQuery(
                HardForkQuery::GetInterpreter,
            )))
            .await;
        let _ = self.client.send_release().await;
        let raw = result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let bytes = pallas_codec::minicbor::to_vec(raw)
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        decode_era_timings(&bytes)
    }

    /// Create a querier that borrows a mutable reference to a pallas
    /// local-state-query client.
    pub fn new(client: &'a mut Client) -> Self {
        Self { client }
    }

    /// Fetch the current protocol parameters from the node.
    pub async fn protocol_params(&mut self) -> Result<ProtocolParams, NetworkError> {
        // Acquire volatile tip.
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;

        // Determine current era.
        let era: u16 = self
            .client
            .query(Request::LedgerQuery(LedgerQuery::HardForkQuery(
                HardForkQuery::GetCurrentEra,
            )))
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;

        // Fetch protocol parameters for that era.
        let pparams = queries_v16::get_current_pparams(self.client, era)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()));

        // Release state before propagating errors.
        let _ = self.client.send_release().await;

        let pparams = pparams?;

        let p = pparams
            .into_iter()
            .next()
            .ok_or_else(|| NetworkError::StateQuery("empty protocol params response".into()))?;

        let params = ProtocolParams {
            min_fee_a: p.minfee_a.unwrap_or(0) as u64,
            min_fee_b: p.minfee_b.unwrap_or(0) as u64,
            max_tx_size: p.max_transaction_size.unwrap_or(0) as u64,
            ada_per_utxo_byte: p.ada_per_utxo_byte.as_ref().map(coin_to_u64).unwrap_or(0),
            key_deposit: p.key_deposit.as_ref().map(coin_to_u64).unwrap_or(0),
            pool_deposit: p.pool_deposit.as_ref().map(coin_to_u64).unwrap_or(0),
            desired_pool_number: p.desired_number_of_stake_pools.unwrap_or(0),
            max_collateral_inputs: p.max_collateral_inputs.unwrap_or(0),
            collateral_percentage: p.collateral_percentage.unwrap_or(0),
            max_value_size: p.max_value_size.unwrap_or(0),
            memory_price: p
                .execution_costs
                .as_ref()
                .map(|p| p.mem_price.numerator as f64 / p.mem_price.denominator as f64)
                .unwrap_or(0.0),
            step_price: p
                .execution_costs
                .as_ref()
                .map(|p| p.step_price.numerator as f64 / p.step_price.denominator as f64)
                .unwrap_or(0.0),
            era,
        };

        Ok(params)
    }

    /// Return the current epoch number.
    pub async fn current_epoch(&mut self) -> Result<u32, NetworkError> {
        // Acquire volatile tip.
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;

        // Determine current era.
        let era: u16 = self
            .client
            .query(Request::LedgerQuery(LedgerQuery::HardForkQuery(
                HardForkQuery::GetCurrentEra,
            )))
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;

        let epoch = queries_v16::get_block_epoch_number(self.client, era)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()));

        let _ = self.client.send_release().await;

        epoch
    }
}
