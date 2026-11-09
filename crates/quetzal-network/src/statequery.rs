use pallas_network::miniprotocols::localstate::queries_v16::{
    self, Coin, HardForkQuery, LedgerQuery, Request,
};
use pallas_network::miniprotocols::localstate::Client;

use crate::NetworkError;

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
        let summaries: Vec<AnyCbor> = raw
            .into_decode()
            .map_err(|e| NetworkError::StateQuery(format!("era interpreter: {e}")))?;
        summaries
            .into_iter()
            .map(|summary| {
                let fields: Vec<AnyCbor> = summary
                    .into_decode()
                    .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
                let start = fields
                    .first()
                    .ok_or_else(|| NetworkError::StateQuery("missing era bound".into()))?;
                let bound: Vec<AnyCbor> = start
                    .clone()
                    .into_decode()
                    .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
                if bound.len() != 3 {
                    return Err(NetworkError::StateQuery("invalid era bound".into()));
                }
                let slot: u64 = bound[1]
                    .clone()
                    .into_decode()
                    .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
                let epoch: u64 = bound[2]
                    .clone()
                    .into_decode()
                    .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
                Ok((slot, epoch))
            })
            .collect()
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
