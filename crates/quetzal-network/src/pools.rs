//! Ledger pool parameters, scheduled changes, stake ratios and member rewards.
//! Query encodings are pinned to ouroboros-consensus 4.2.1.0.
use crate::{NetworkError, StateQuerier};
use pallas_codec::utils::{AnyCbor, Bytes, TagWrap};
use pallas_network::miniprotocols::localstate::{
    queries_v16::{self as q, BlockQuery, LedgerQuery, Request},
    ClientError,
};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct PoolLedgerState {
    pub parameters: q::PoolParams,
    pub retirement: Option<u64>,
    pub relative_stake: (u64, u64),
    pub member_rewards: u64,
}

#[derive(Debug)]
pub struct PoolSnapshot {
    pub epoch: u64,
    pub optimum_pool_count: u32,
    pub pools: BTreeMap<Vec<u8>, PoolLedgerState>,
}

type PoolParamsMap = BTreeMap<Bytes, q::PoolParams>;
type PoolCoinMap = BTreeMap<Bytes, u64>;
type PoolStateResult = (PoolParamsMap, PoolParamsMap, PoolCoinMap, PoolCoinMap);
type MemberRewards = BTreeMap<(u8, u64), PoolCoinMap>;

impl StateQuerier<'_> {
    /// Read every component against one acquired tip. Future registrations
    /// override current parameters, matching the wallet's catalogue semantics.
    pub async fn pools(&mut self, stake: u64) -> Result<PoolSnapshot, NetworkError> {
        self.client
            .acquire(None)
            .await
            .map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let result = async {
            let era = q::get_current_era(self.client).await?;
            let (epoch,): (u64,) = self
                .client
                .query(Request::LedgerQuery(LedgerQuery::BlockQuery(
                    era,
                    BlockQuery::GetEpochNo,
                )))
                .await?;
            // Nothing requests all pools; Maybe uses [] / [set].
            let query = BlockQuery::GetPoolState(AnyCbor::from_encode(Vec::<u8>::new()));
            let ((mut current, future, retirements, _deposits),): (PoolStateResult,) = self
                .client
                .query(Request::LedgerQuery(LedgerQuery::BlockQuery(era, query)))
                .await?;
            current.extend(future);
            let distribution = q::get_stake_distribution(self.client, era).await?;
            let parameters = q::get_current_pparams(self.client, era).await?;
            let query =
                BlockQuery::GetNonMyopicMemberRewards(AnyCbor::from_encode(
                    TagWrap::<_, 258>::new(vec![(0u8, stake)]),
                ));
            let (rewards,): (MemberRewards,) = self
                .client
                .query(Request::LedgerQuery(LedgerQuery::BlockQuery(era, query)))
                .await?;
            Ok::<_, ClientError>((
                epoch,
                current,
                retirements,
                distribution,
                rewards,
                parameters,
            ))
        }
        .await;
        let release = self.client.send_release().await;
        let (epoch, current, retirements, distribution, mut rewards, parameters) =
            result.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        release.map_err(|e| NetworkError::StateQuery(e.to_string()))?;
        let optimum_pool_count = parameters
            .first()
            .and_then(|parameters| parameters.desired_number_of_stake_pools)
            .ok_or_else(|| {
                NetworkError::StateQuery("missing desired number of stake pools".into())
            })?;
        let rewards = rewards.remove(&(0, stake)).ok_or_else(|| {
            NetworkError::StateQuery("requested member rewards are absent".into())
        })?;
        // The pinned wallet gives newly registered pools without LSQ metrics
        // the rounded average reward of the best nOpt pools.
        let mut live_rewards: BTreeMap<_, _> = distribution
            .pools
            .iter()
            .map(|(pool, _)| (pool.clone(), 0u64))
            .collect();
        live_rewards.extend(rewards.iter().map(|(pool, reward)| (pool.clone(), *reward)));
        let mut ranked: Vec<_> = live_rewards.values().copied().collect();
        ranked.sort_unstable_by(|a, b| b.cmp(a));
        ranked.truncate(optimum_pool_count as usize);
        let freshman_reward = if ranked.is_empty() {
            0
        } else {
            let sum: u128 = ranked.iter().map(|value| u128::from(*value)).sum();
            let average = (sum as f64 / ranked.len() as f64).round_ties_even();
            if !average.is_finite() || average < 0.0 || average >= u64::MAX as f64 {
                return Err(NetworkError::StateQuery(
                    "pool reward average overflow".into(),
                ));
            }
            average as u64
        };
        let mut pools = BTreeMap::new();
        for (id, parameters) in current {
            let ratio = distribution
                .pools
                .iter()
                .find(|(pool, _)| pool == &id)
                .map(|(_, pool)| (pool.stakes.numerator, pool.stakes.denominator))
                .unwrap_or((0, 1));
            if ratio.1 == 0 {
                return Err(NetworkError::StateQuery(
                    "zero pool stake denominator".into(),
                ));
            }
            // Absence from these ledger maps means no stake/reward/retirement,
            // rather than a failed or missing query response.
            pools.insert(
                id.to_vec(),
                PoolLedgerState {
                    retirement: retirements.get(&id).copied(),
                    relative_stake: ratio,
                    member_rewards: live_rewards.get(&id).copied().unwrap_or(freshman_reward),
                    parameters,
                },
            );
        }
        Ok(PoolSnapshot {
            epoch,
            optimum_pool_count,
            pools,
        })
    }
}
