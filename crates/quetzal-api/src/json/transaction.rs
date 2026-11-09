use quetzal_primitives::{Coin, TxDirection, TxRecord, TxStatus};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PostTransactionRequest {
    pub passphrase: String,
    pub payments: Vec<PaymentTarget>,
    pub withdrawal: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub time_to_live: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct PaymentTarget {
    pub address: String,
    pub amount: AmountInput,
    pub assets: Option<Vec<quetzal_primitives::tx::AssetAmount>>,
}

#[derive(Debug, Deserialize)]
pub struct AmountInput {
    #[serde(deserialize_with = "quetzal_primitives::quantity::deserialize_coin")]
    pub quantity: u64,
    pub unit: String,
}

#[derive(Debug, Deserialize)]
pub struct PostTransactionSubmitRequest {
    pub transaction: String, // base64-encoded CBOR
}

#[derive(Debug, Serialize)]
pub struct TransactionResponse {
    pub certificates: Vec<serde_json::Value>,
    pub id: String,
    pub amount: AmountResponse,
    pub fee: AmountResponse,
    pub deposit_taken: AmountResponse,
    pub deposit_returned: AmountResponse,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inserted_at: Option<TimeResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<TimeResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_since: Option<TimeResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<DepthResponse>,
    pub direction: String,
    pub inputs: Vec<InputResponse>,
    pub outputs: Vec<OutputResponse>,
    pub collateral: Vec<serde_json::Value>,
    pub collateral_outputs: Vec<serde_json::Value>,
    pub withdrawals: Vec<serde_json::Value>,
    pub mint: MintBurnResponse,
    pub burn: MintBurnResponse,
    pub status: String,
    pub metadata: Option<serde_json::Value>,
    pub script_validity: String,
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

#[derive(Debug, Clone, Serialize)]
pub struct TimeResponse {
    pub time: String,
    pub slot_number: u64,
    pub epoch_number: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BlockHeight>,
    pub absolute_slot_number: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct BlockHeight {
    pub quantity: u64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct DepthResponse {
    pub quantity: u64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct InputResponse {
    pub id: String,
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub amount: Option<AmountResponse>,
    pub assets: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct OutputResponse {
    pub address: String,
    pub amount: AmountResponse,
    pub assets: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derivation_path: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct MintBurnResponse {
    pub tokens: Vec<serde_json::Value>,
}

pub fn withdrawal_responses(
    withdrawals: &[quetzal_primitives::tx::Withdrawal],
) -> Vec<serde_json::Value> {
    withdrawals.iter().map(|withdrawal| serde_json::json!({"stake_address":withdrawal.stake_address.as_str(), "amount":{"quantity":withdrawal.amount.quantity.0,"unit":"lovelace"}})).collect()
}

impl TransactionResponse {
    pub fn from_record(
        tx: &TxRecord,
        config: &crate::config::Config,
        current_height: u64,
    ) -> Result<Self, crate::error::ApiError> {
        let status = match tx.status {
            TxStatus::InLedger => "in_ledger",
            TxStatus::Pending => "pending",
            TxStatus::Expired => "expired",
        };
        let direction = match tx.direction {
            TxDirection::Incoming => "incoming",
            TxDirection::Outgoing => "outgoing",
        };
        let (epoch_number, slot_number, time) = config.slot_reference(tx.slot);
        let time_reference = TimeResponse {
            time,
            slot_number,
            epoch_number,
            height: Some(BlockHeight {
                quantity: tx.inserted_at.block.height.quantity,
                unit: "block".to_string(),
            }),
            absolute_slot_number: tx.inserted_at.block.slot_no,
        };
        let in_ledger = tx.status == TxStatus::InLedger;
        let inserted_at = if in_ledger {
            Some(time_reference.clone())
        } else {
            None
        };
        let pending_since = if in_ledger {
            None
        } else {
            Some(time_reference)
        };
        let expires_at = tx.expires_at_slot.map(|slot| {
            let (epoch_number, slot_number, time) = config.slot_reference(slot);
            TimeResponse {
                time,
                slot_number,
                epoch_number,
                height: None,
                absolute_slot_number: slot,
            }
        });
        Ok(TransactionResponse {
            certificates: quetzal_wallet::staking::api_certificates(tx).map_err(crate::error::ApiError::from)?,
            id: tx.id.to_hex(),
            amount: AmountResponse::lovelace(tx.amount),
            fee: AmountResponse::lovelace(tx.fee.unwrap_or(Coin::ZERO)),
            deposit_taken: AmountResponse::lovelace(tx.deposit_taken),
            deposit_returned: AmountResponse::lovelace(tx.deposit_returned),
            inserted_at,
            expires_at,
            pending_since,
            depth: if in_ledger{Some(DepthResponse {
                quantity: current_height.saturating_sub(tx.block_height),
                unit: "block".to_string(),
            })}else{None},
            direction: direction.to_string(),
            inputs: tx
                .inputs
                .iter()
                .map(|i| InputResponse {
                    id: i.id.to_hex(),
                    index: i.index,
                    address: i.address.as_ref().map(|a| a.0.clone()),
                    amount: i
                        .amount
                        .as_ref()
                        .map(|b| AmountResponse::lovelace(b.quantity)),
                    assets:i.assets.iter().map(|asset|serde_json::json!({"policy_id":asset.policy_id,"asset_name":asset.asset_name,"quantity":asset.quantity})).collect(),
                })
                .collect(),
            outputs: tx
                .outputs
                .iter()
                .map(|o| OutputResponse {
                    address: o.address.0.clone(),
                    amount: AmountResponse::lovelace(o.amount.quantity),
                    assets:o.assets.iter().map(|asset|serde_json::json!({"policy_id":asset.policy_id,"asset_name":asset.asset_name,"quantity":asset.quantity})).collect(),
                    derivation_path: None,
                })
                .collect(),
            collateral: vec![],
            collateral_outputs: vec![],
            withdrawals: withdrawal_responses(&tx.withdrawals),
            mint: MintBurnResponse { tokens: vec![] },
            burn: MintBurnResponse { tokens: vec![] },
            status: status.to_string(),
            metadata: tx.metadata.clone(),
            script_validity: "valid".to_string(),
        })
    }
}
pub(crate) fn derivation_path(
    kind: &quetzal_primitives::WalletType,
    path: &quetzal_primitives::DerivationPath,
) -> serde_json::Value {
    use quetzal_primitives::WalletType;
    if *kind == WalletType::Byron {
        return serde_json::json!([
            format!("{}H", path.account_ix & 0x7fffffff),
            format!("{}H", path.address_ix & 0x7fffffff)
        ]);
    }
    let purpose = match kind {
        WalletType::Icarus => "44H",
        WalletType::Shared => "1854H",
        _ => "1852H",
    };
    serde_json::json!([
        purpose,
        "1815H",
        format!("{}H", path.account_ix),
        path.role.index().to_string(),
        path.address_ix.to_string()
    ])
}
