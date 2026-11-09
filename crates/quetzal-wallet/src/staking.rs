//! Confirmed stake certificates and their reversible local ledger state.
use crate::{WalletError, WalletService};
use pallas_primitives::{conway, StakeCredential};
use quetzal_primitives::{Coin, TxRecord, WalletId};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StakeState {
    pub deposit: Option<u64>,
    pub vote: Option<String>,
    pub pool: Option<String>,
    #[serde(default)]
    pub pool_changes: Vec<PoolChange>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PoolChange {
    pub epoch: u64,
    pub target: String,
}

#[derive(Default)]
pub struct Operation {
    pub certificates: Vec<conway::Certificate>,
    pub deposit_taken: u64,
    pub deposit_returned: u64,
    pub withdrawals: std::collections::BTreeMap<Vec<u8>, u64>,
}

/// Pinned Shelley wallet policy: require twice the conservative withdrawal
/// cost. SizeEstimation uses 41 withdrawal bytes and 105 vkey-witness bytes.
pub fn check_withdrawal_reward(reward: u64, fee_per_byte: u64) -> Result<(), WalletError> {
    let minimum = fee_per_byte
        .checked_mul(2 * (41 + 105))
        .ok_or_else(|| WalletError::TxBuild("withdrawal cost overflow".into()))?;
    if reward == 0 || reward < minimum {
        return Err(WalletError::WithdrawalNotBeneficial);
    }
    Ok(())
}

pub fn stake_hash(public: &[u8; 64]) -> Result<[u8; 28], WalletError> {
    let public = quetzal_crypto::Bip32PublicKey::from_raw_bytes(public)?
        .derive(2)?
        .derive(0)?;
    Ok(*pallas_crypto::hash::Hasher::<224>::hash(
        &public.to_vk_bytes(),
    ))
}

pub fn drep_text(drep: &conway::DRep) -> Result<String, WalletError> {
    Ok(match drep {
        conway::DRep::Abstain => "abstain".into(),
        conway::DRep::NoConfidence => "no_confidence".into(),
        conway::DRep::Key(hash) => {
            let mut bytes = vec![0x22];
            bytes.extend_from_slice(hash.as_ref());
            quetzal_address::codec::bech32_encode("drep", &bytes)?
        }
        conway::DRep::Script(hash) => {
            let mut bytes = vec![0x23];
            bytes.extend_from_slice(hash.as_ref());
            quetzal_address::codec::bech32_encode("drep", &bytes)?
        }
    })
}

pub fn drep(text: &str) -> Result<conway::DRep, WalletError> {
    Ok(match text {
        "abstain" => conway::DRep::Abstain,
        "no_confidence" => conway::DRep::NoConfidence,
        _ => {
            let (hrp, bytes) = quetzal_address::codec::bech32_decode(text)?;
            let (script, bytes) = match (hrp.as_str(), bytes.as_slice()) {
                ("drep_vkh", bytes) if bytes.len() == 28 => (false, bytes),
                ("drep", bytes) if bytes.len() == 28 => (false, bytes),
                ("drep_script", bytes) if bytes.len() == 28 => (true, bytes),
                ("drep", [0x22, bytes @ ..]) if bytes.len() == 28 => (false, bytes),
                ("drep", [0x23, bytes @ ..]) if bytes.len() == 28 => (true, bytes),
                _ => return Err(WalletError::TxBuild("invalid DRep identifier".into())),
            };
            let hash: [u8; 28] = bytes
                .try_into()
                .map_err(|_| WalletError::TxBuild("invalid DRep hash".into()))?;
            if script {
                conway::DRep::Script(hash.into())
            } else {
                conway::DRep::Key(hash.into())
            }
        }
    })
}

fn credential(certificate: &conway::Certificate) -> Option<&StakeCredential> {
    use conway::Certificate as C;
    match certificate {
        C::StakeRegistration(key)
        | C::StakeDeregistration(key)
        | C::StakeDelegation(key, _)
        | C::Reg(key, _)
        | C::UnReg(key, _)
        | C::VoteDeleg(key, _)
        | C::StakeVoteDeleg(key, _, _)
        | C::StakeRegDeleg(key, _, _)
        | C::VoteRegDeleg(key, _, _)
        | C::StakeVoteRegDeleg(key, _, _, _) => Some(key),
        _ => None,
    }
}

pub fn owned_certificates(
    tx: &pallas_traverse::MultiEraTx<'_>,
    key: &[u8; 28],
) -> Result<Operation, WalletError> {
    let mut operation = Operation::default();
    for cert in tx.certs() {
        let Some(cert) = cert.as_conway() else {
            continue;
        };
        if !matches!(credential(cert), Some(StakeCredential::AddrKeyhash(hash)) if **hash == *key) {
            continue;
        }
        use conway::Certificate as C;
        match cert {
            C::Reg(_, deposit)
            | C::StakeRegDeleg(_, _, deposit)
            | C::VoteRegDeleg(_, _, deposit)
            | C::StakeVoteRegDeleg(_, _, _, deposit) => {
                operation.deposit_taken = operation
                    .deposit_taken
                    .checked_add(*deposit)
                    .ok_or_else(|| WalletError::TxBuild("stake deposit overflow".into()))?
            }
            C::UnReg(_, refund) => {
                operation.deposit_returned = operation
                    .deposit_returned
                    .checked_add(*refund)
                    .ok_or_else(|| WalletError::TxBuild("stake refund overflow".into()))?
            }
            _ => {}
        }
        operation.certificates.push(cert.clone());
    }
    Ok(operation)
}

impl Operation {
    pub fn encoded(&self) -> Result<Vec<Vec<u8>>, WalletError> {
        self.certificates
            .iter()
            .map(|cert| {
                pallas_codec::minicbor::to_vec(cert)
                    .map_err(|e| WalletError::TxBuild(e.to_string()))
            })
            .collect()
    }
}

impl StakeState {
    fn set_pool(
        &mut self,
        pool: pallas_crypto::hash::Hash<28>,
        record: &TxRecord,
    ) -> Result<(), WalletError> {
        let target = quetzal_address::codec::bech32_encode("pool", pool.as_ref())?;
        let epoch = u64::from(record.inserted_at.block.epoch_no) + 2;
        self.pool = Some(target.clone());
        self.pool_changes.retain(|change| change.epoch != epoch);
        self.pool_changes.push(PoolChange { epoch, target });
        Ok(())
    }

    pub fn active_pool(&self, epoch: u64) -> Option<&str> {
        if self.pool_changes.is_empty() {
            return self.pool.as_deref();
        }
        self.pool_changes
            .iter()
            .rev()
            .find(|change| change.epoch <= epoch)
            .map(|change| change.target.as_str())
    }

    pub fn apply(&mut self, record: &TxRecord) -> Result<(), WalletError> {
        use conway::Certificate as C;
        for encoded in &record.certificates {
            let cert: C = pallas_codec::minicbor::decode(encoded)
                .map_err(|e| WalletError::Cbor(e.to_string()))?;
            match cert {
                C::Reg(_, deposit) => self.deposit = Some(deposit),
                C::VoteDeleg(_, vote) => self.vote = Some(drep_text(&vote)?),
                C::StakeDelegation(_, pool) => {
                    self.set_pool(pool, record)?;
                }
                C::UnReg(_, _) | C::StakeDeregistration(_) => *self = Self::default(),
                C::VoteRegDeleg(_, vote, deposit) => {
                    self.vote = Some(drep_text(&vote)?);
                    self.deposit = Some(deposit);
                }
                C::StakeRegDeleg(_, pool, deposit) => {
                    self.set_pool(pool, record)?;
                    self.deposit = Some(deposit);
                }
                C::StakeVoteDeleg(_, pool, vote) => {
                    self.set_pool(pool, record)?;
                    self.vote = Some(drep_text(&vote)?);
                }
                C::StakeVoteRegDeleg(_, pool, vote, deposit) => {
                    self.set_pool(pool, record)?;
                    self.vote = Some(drep_text(&vote)?);
                    self.deposit = Some(deposit);
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl WalletService {
    pub async fn staking_state(&self, wallet: &WalletId) -> Result<StakeState, WalletError> {
        self.get_wallet(wallet).await?;
        self.db.staking_state(wallet)
    }
}

pub fn deposit_coins(operation: &Operation) -> (Coin, Coin) {
    (
        Coin(operation.deposit_taken),
        Coin(operation.deposit_returned),
    )
}

pub fn api_certificates(record: &TxRecord) -> Result<Vec<serde_json::Value>, WalletError> {
    if record.certificates.is_empty() {
        return Ok(vec![]);
    }
    let path = record
        .stake_derivation_path
        .as_ref()
        .ok_or_else(|| WalletError::Cbor("missing stake certificate derivation path".into()))?;
    api_certificate_list(&record.certificates, path)
}

pub fn api_certificate_list(
    certificates: &[Vec<u8>],
    path: &quetzal_primitives::DerivationPath,
) -> Result<Vec<serde_json::Value>, WalletError> {
    let path = serde_json::json!([
        "1852H",
        "1815H",
        format!("{}H", path.account_ix),
        "2",
        path.address_ix.to_string()
    ]);
    let mut result = Vec::new();
    for encoded in certificates {
        use conway::Certificate as C;
        let cert: C = pallas_codec::minicbor::decode(encoded)
            .map_err(|e| WalletError::Cbor(e.to_string()))?;
        let (registering, pool, vote, quitting) = match cert {
            C::Reg(_, _) | C::StakeRegistration(_) => (true, None, None, false),
            C::UnReg(_, _) | C::StakeDeregistration(_) => (false, None, None, true),
            C::VoteDeleg(_, vote) => (false, None, Some(vote), false),
            C::StakeDelegation(_, pool) => (false, Some(pool), None, false),
            C::StakeVoteDeleg(_, pool, vote) => (false, Some(pool), Some(vote), false),
            C::StakeRegDeleg(_, pool, _) => (true, Some(pool), None, false),
            C::VoteRegDeleg(_, vote, _) => (true, None, Some(vote), false),
            C::StakeVoteRegDeleg(_, pool, vote, _) => (true, Some(pool), Some(vote), false),
            _ => {
                return Err(WalletError::Cbor(
                    "unexpected owned stake certificate".into(),
                ))
            }
        };
        if registering {
            result.push(serde_json::json!({"certificate_type":"register_reward_account", "reward_account_path":path}));
        }
        if quitting {
            result.push(
                serde_json::json!({"certificate_type":"quit_pool", "reward_account_path":path}),
            );
        }
        if let Some(pool) = pool {
            result.push(serde_json::json!({"certificate_type":"join_pool", "reward_account_path":path, "pool": quetzal_address::codec::bech32_encode("pool", pool.as_ref())?}));
        }
        if let Some(vote) = vote {
            result.push(serde_json::json!({"certificate_type":"cast_vote", "reward_account_path":path, "vote":drep_text(&vote)?}));
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn withdrawal_benefit_uses_the_pinned_conservative_cost_boundary() {
        let minimum = 2 * (41 + 105) * 44;
        assert!(matches!(
            check_withdrawal_reward(0, 44),
            Err(WalletError::WithdrawalNotBeneficial)
        ));
        assert!(matches!(
            check_withdrawal_reward(minimum - 1, 44),
            Err(WalletError::WithdrawalNotBeneficial)
        ));
        assert!(check_withdrawal_reward(minimum, 44).is_ok());
        assert!(check_withdrawal_reward(1000000, u64::MAX).is_err());
    }
}
