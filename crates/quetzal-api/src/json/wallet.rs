use quetzal_primitives::{Coin, DelegationStatus, PassphraseScheme, WalletMeta, WalletStatus};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Deserialize)]
pub struct PostWalletRequest {
    pub name: String,
    #[serde(default)]
    pub mnemonic_sentence: Vec<String>,
    #[serde(default)]
    pub passphrase: String,
    pub account_public_key: Option<String>,
    #[serde(default = "default_gap")]
    #[serde(deserialize_with = "quetzal_primitives::quantity::deserialize_u32")]
    pub address_pool_gap: u32,
    pub mnemonic_second_factor: Option<Vec<String>>,
}

fn default_gap() -> u32 {
    20
}

#[derive(Debug, Deserialize)]
pub struct PutPassphraseRequest {
    pub old_passphrase: Option<String>,
    pub new_passphrase: String,
    pub mnemonic_sentence: Option<Vec<String>>,
    pub mnemonic_second_factor: Option<Vec<String>>,
}

impl PutPassphraseRequest {
    pub fn parse(value: serde_json::Value, byron: bool) -> Result<Self, crate::error::ApiError> {
        use crate::error::ApiError;
        if !byron
            && value.get("old_passphrase").is_none()
            && value.get("mnemonic_sentence").is_none()
        {
            return Err(ApiError::bad_request(
                "bad_request",
                "Error in $: no variant selection criteria matching",
            ));
        }
        let typename = if byron {
            "ByronWalletPutPassphraseData"
        } else if value.get("old_passphrase").is_some() {
            "WalletPutPassphraseOldPassphraseData"
        } else {
            "WalletPutPassphraseMnemonicData"
        };
        if value.get("new_passphrase").is_none() {
            return Err(ApiError::bad_request("bad_request",format!("Error in $: parsing Cardano.Wallet.Api.Types.{typename}({typename}) failed, key 'new_passphrase' not found")));
        }
        serde_json::from_value(value)
            .map_err(|e| ApiError::bad_request("bad_request", format!("Error in $: {e}")))
    }
}

#[derive(Debug, Deserialize)]
pub struct PutWalletRequest {
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WalletResponse {
    pub id: String,
    pub name: String,
    pub address_pool_gap: u32,
    pub balance: BalanceResponse,
    pub state: StateResponse,
    pub delegation: DelegationResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<PassphraseResponse>,
    pub tip: TipResponse,
    pub assets: AssetsResponse,
}

#[derive(Debug, Serialize)]
pub struct BalanceResponse {
    pub available: AmountResponse,
    pub total: AmountResponse,
    pub reward: AmountResponse,
}

#[derive(Debug, Serialize)]
pub struct AmountResponse {
    pub quantity: u64,
    pub unit: String,
}

impl AmountResponse {
    pub fn lovelace(coin: Coin) -> Self {
        AmountResponse {
            quantity: coin.as_lovelace(),
            unit: "lovelace".to_string(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct StateResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressResponse>,
}

#[derive(Debug, Serialize)]
pub struct ProgressResponse {
    pub quantity: f64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct DelegationResponse {
    pub active: ActiveDelegation,
    pub next: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ActiveDelegation {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voting: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PassphraseResponse {
    pub last_updated_at: Option<String>,
    pub encryption_method: String,
}

#[derive(Debug, Serialize)]
pub struct TipResponse {
    pub height: BlockHeightResponse,
    pub time: Option<String>,
    pub epoch_number: Option<u64>,
    pub slot_number: Option<u64>,
    pub absolute_slot_number: Option<u64>,
}

#[derive(Debug, Serialize)]
pub struct BlockHeightResponse {
    pub quantity: u64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct AssetsResponse {
    pub available: Vec<serde_json::Value>,
    pub total: Vec<serde_json::Value>,
}

impl WalletResponse {
    pub fn apply_staking(
        &mut self,
        state: &quetzal_wallet::staking::StakeState,
        config: &crate::config::Config,
        epoch: u64,
    ) -> Result<(), crate::error::ApiError> {
        let render = |pool: Option<&str>| ActiveDelegation {
            status: match (pool.is_some(), state.vote.is_some()) {
                (true, true) => "voting_and_delegating",
                (true, false) => "delegating",
                (false, true) => "voting",
                (false, false) => "not_delegating",
            }
            .into(),
            target: pool.map(|pool| json!(pool)),
            voting: state.vote.clone(),
        };
        self.delegation.active = render(state.active_pool(epoch));
        self.delegation.next = state.pool_changes.iter().filter(|change| change.epoch > epoch).map(|change| {
            let mut next = serde_json::to_value(render(Some(&change.target))).map_err(|e|crate::error::ApiError::internal(e.to_string()))?;
            let slot = config.epoch_first_slot(change.epoch).ok_or_else(||crate::error::ApiError::internal("delegation epoch overflow"))?;
            next["changes_at"] = json!({"epoch_number":change.epoch, "epoch_start_time":config.slot_reference(slot).2});
            Ok(next)
        }).collect::<Result<_, crate::error::ApiError>>()?;
        Ok(())
    }
    pub fn from_meta(meta: &WalletMeta, config: &crate::config::Config) -> Self {
        let (epoch, slot, time) = config.slot_reference(meta.tip_slot);
        let (status, progress) = if meta.status == WalletStatus::NotResponding {
            ("not_responding", None)
        } else if let Some(quantity) =
            config.wallet_sync_progress_at(meta.tip_slot, std::time::SystemTime::now())
        {
            (
                "syncing",
                Some(ProgressResponse {
                    quantity,
                    unit: "percent".into(),
                }),
            )
        } else {
            ("ready", None)
        };
        let encryption_method = match meta.passphrase_scheme {
            PassphraseScheme::Argon2idV2 => "argon2id-v2",
            PassphraseScheme::Pbkdf2HmacSha512 => "pbkdf2-hmac-sha512",
            PassphraseScheme::Scrypt => "scrypt",
        };
        WalletResponse {
            id: meta.id.to_hex(),
            name: meta.name.clone(),
            address_pool_gap: meta.address_pool_gap,
            balance: BalanceResponse {
                available: AmountResponse::lovelace(meta.balance_available),
                total: AmountResponse::lovelace(meta.balance_total),
                reward: AmountResponse::lovelace(meta.balance_reward),
            },
            state: StateResponse {
                status: status.to_string(),
                progress,
            },
            delegation: {
                let (deleg_status, deleg_target) = match meta.delegation_status {
                    DelegationStatus::NotDelegating => ("not_delegating", None),
                    DelegationStatus::Delegating => (
                        "delegating",
                        meta.stake_pool_id
                            .as_ref()
                            .map(|pool_id| json!({ "id": pool_id })),
                    ),
                    DelegationStatus::Retiring => (
                        "retiring",
                        meta.stake_pool_id
                            .as_ref()
                            .map(|pool_id| json!({ "id": pool_id })),
                    ),
                };
                DelegationResponse {
                    active: ActiveDelegation {
                        status: deleg_status.to_string(),
                        target: deleg_target,
                        voting: None,
                    },
                    next: vec![],
                }
            },
            passphrase: meta
                .passphrase_last_updated_at
                .as_ref()
                .map(|_| PassphraseResponse {
                    last_updated_at: meta.passphrase_last_updated_at.clone(),
                    encryption_method: encryption_method.to_string(),
                }),
            tip: TipResponse {
                height: BlockHeightResponse {
                    quantity: meta.tip_height,
                    unit: "block".to_string(),
                },
                time: Some(time),
                epoch_number: Some(epoch),
                slot_number: Some(slot),
                absolute_slot_number: Some(meta.tip_slot),
            },
            assets: AssetsResponse {
                available:meta.assets_available.iter().map(|asset|json!({"policy_id":asset.policy_id,"asset_name":asset.asset_name,"quantity":asset.quantity})).collect(),
                total:meta.assets_total.iter().map(|asset|json!({"policy_id":asset.policy_id,"asset_name":asset.asset_name,"quantity":asset.quantity})).collect(),
            },
        }
    }
}
