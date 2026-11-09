use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct NetworkInfoResponse {
    pub node_tip: NodeTipInfo,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_tip: Option<NodeTipInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_epoch: Option<EpochInfo>,
    pub node_era: String,
    pub sync_progress: SyncProgress,
    pub network_info: NetworkIdentity,
    pub wallet_mode: &'static str,
}

#[derive(Debug, Serialize)]
pub struct NetworkIdentity {
    pub protocol_magic: u64,
    pub network_id: &'static str,
}

#[derive(Debug, Serialize)]
pub struct NodeTipInfo {
    pub absolute_slot_number: Option<u64>,
    pub slot_number: Option<u64>,
    pub epoch_number: Option<u64>,
    pub time: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<BlockHeight>,
}

#[derive(Debug, Serialize)]
pub struct BlockHeight {
    pub quantity: u64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct EpochInfo {
    pub epoch_number: u64,
    pub epoch_start_time: String,
}

#[derive(Debug, Serialize)]
pub struct SyncProgress {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<ProgressValue>,
}

#[derive(Debug, Serialize)]
pub struct ProgressValue {
    #[serde(serialize_with = "quetzal_primitives::quantity::serialize_float")]
    pub quantity: f64,
    pub unit: String,
}

#[derive(Debug, Serialize)]
pub struct NetworkParamsResponse {
    pub genesis_block_hash: String,
    pub blockchain_start_time: String,
    pub slot_length: SlotLength,
    pub epoch_length: EpochLength,
    pub security_parameter: BlockHeight,
    pub active_slot_coefficient: ProgressValue,
    pub decentralization_level: ProgressValue,
    pub desired_pool_number: u32,
    pub maximum_collateral_input_count: u32,
    pub minimum_collateral_percentage: u32,
    pub maximum_token_bundle_size: ByteSize,
    pub execution_unit_prices: ExecutionPrices,
    pub eras: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct SlotLength {
    #[serde(serialize_with = "quetzal_primitives::quantity::serialize_float")]
    pub quantity: f64,
    pub unit: String,
}
#[derive(Debug, Serialize)]
pub struct EpochLength {
    pub quantity: u64,
    pub unit: String,
}
#[derive(Debug, Serialize)]
pub struct ByteSize {
    pub quantity: u32,
    pub unit: &'static str,
}

#[derive(Debug, Serialize)]
pub struct ExecutionPrices {
    #[serde(serialize_with = "quetzal_primitives::quantity::serialize_float")]
    pub memory_unit_price: f64,
    #[serde(serialize_with = "quetzal_primitives::quantity::serialize_float")]
    pub step_price: f64,
}

#[derive(Debug, Serialize)]
pub struct NetworkClockResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<OffsetValue>,
}

#[derive(Debug, Serialize)]
pub struct OffsetValue {
    pub quantity: i64,
    pub unit: String,
}
