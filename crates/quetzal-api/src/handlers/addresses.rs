use crate::{error::ApiError, json::address::AddressResponse, state::AppState};
use axum::{
    extract::{Path, Query, State},
    Json,
};
use quetzal_primitives::WalletId;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AddressListQuery {
    pub state: Option<String>,
}

pub async fn list_addresses(
    State(state): State<AppState>,
    Path(wallet_id): Path<String>,
    Query(q): Query<AddressListQuery>,
) -> Result<Json<Vec<AddressResponse>>, ApiError> {
    if q.state
        .as_deref()
        .is_some_and(|s| s != "used" && s != "unused")
    {
        return Err(ApiError::bad_request("bad_request", "Error parsing query parameter state failed: Unable to decode the given text value. Please specify one of the following values: used, unused."));
    }
    let id = WalletId::from_hex(&wallet_id).map_err(|_| {
        ApiError::bad_request(
            "bad_request",
            "wallet id should be a hex-encoded string of 40 characters",
        )
    })?;
    let mut addresses = state
        .wallet_service
        .get_known_addresses(&id)
        .await
        .map_err(ApiError::from)?;
    addresses.sort_by_key(|a| (a.path.address_ix, a.path.role.index()));
    let filtered: Vec<_> = addresses
        .iter()
        .filter(|a| {
            if let Some(ref s) = q.state {
                matches!(
                    (s.as_str(), &a.state),
                    ("used", quetzal_primitives::AddressState::Used)
                        | ("unused", quetzal_primitives::AddressState::Unused)
                )
            } else {
                true
            }
        })
        .map(AddressResponse::from_entry)
        .collect();
    Ok(Json(filtered))
}

pub async fn inspect_address(
    Path(address_id): Path<String>,
) -> Result<Json<serde_json::Value>, ApiError> {
    use std::str::FromStr;
    let address = pallas_addresses::Address::from_str(&address_id)
        .map_err(|_| ApiError::bad_request("bad_request", "Unrecognized address encoding: must be either bech32 or base58. Perhaps your address is not entirely correct? Please double-check each character within the address and try again."))?;
    let raw = address.to_vec();
    let header = *raw
        .first()
        .ok_or_else(|| ApiError::bad_request("bad_request", "Empty address"))?;
    if header >> 4 == 8 {
        let byron = match address {
            pallas_addresses::Address::Byron(address) => address,
            _ => {
                return Err(ApiError::bad_request(
                    "bad_request",
                    "Invalid Byron address",
                ))
            }
        };
        let payload = byron
            .decode()
            .map_err(|_| ApiError::bad_request("bad_request", "Invalid Byron address"))?;
        let random = payload.attributes.iter().any(|attr| {
            matches!(
                attr,
                pallas_addresses::byron::AddrAttrProperty::DerivationPath(_)
            )
        });
        Ok(Json(
            serde_json::json!({"address_style":if random {"Byron"}else{"Icarus"},"stake_reference":"none"}),
        ))
    } else {
        let reference = match header >> 4 {
            0..=3 | 14..=15 => "by value",
            4..=5 => "by pointer",
            _ => "none",
        };
        let kind = header >> 4;
        let mut response = serde_json::json!({"address_style":"Shelley","address_type":kind,"stake_reference":reference,"network_tag":header&15});
        let mut credential = |field: &str, hrp: &str, hash: &[u8]| -> Result<(), ApiError> {
            response[format!("{field}_hash")] = serde_json::json!(hex::encode(hash));
            response[format!("{field}_hash_bech32")] =
                serde_json::json!(quetzal_address::codec::bech32_encode(hrp, hash)
                    .map_err(|_| ApiError::internal("Credential encoding failed"))?);
            Ok(())
        };
        if matches!(kind, 0 | 2 | 4 | 6) {
            credential("spending_key", "addr_vkh", &raw[1..29])?;
        }
        if matches!(kind, 0 | 1) {
            credential("stake_key", "stake_vkh", &raw[29..57])?;
        }
        if kind == 14 {
            credential("stake_key", "stake_vkh", &raw[1..29])?;
        }
        Ok(Json(response))
    }
}

pub async fn create_address(
    State(state): State<AppState>,
    Json(request): Json<serde_json::Value>,
) -> Result<(axum::http::StatusCode, Json<serde_json::Value>), ApiError> {
    use quetzal_address::codec::{AddressKind, Credential};
    fn credential(value: &serde_json::Value, role: &str) -> Result<Credential, ApiError> {
        let encoded = value.as_str().ok_or_else(|| {
            ApiError::bad_request("bad_request", "Expected an encoded public credential")
        })?;
        let (hrp, raw) = quetzal_address::codec::bech32_decode(encoded)
            .map_err(|_| ApiError::bad_request("bad_request", "Invalid encoded credential"))?;
        let hash = if hrp == format!("{role}_vkh") && raw.len() == 28 {
            raw
        } else if (hrp == format!("{role}_vk") && raw.len() == 32)
            || (hrp == format!("{role}_xvk") && raw.len() == 64)
        {
            pallas_crypto::hash::Hasher::<224>::hash(&raw[..32])
                .as_ref()
                .to_vec()
        } else {
            return Err(ApiError::bad_request(
                "bad_request",
                "Invalid credential type or length",
            ));
        };
        Ok(Credential::Key {
            hash_hex: hex::encode(hash),
        })
    }
    let payment = request
        .get("payment")
        .map(|value| credential(value, "addr"))
        .transpose()?;
    let stake = request
        .get("stake")
        .map(|value| credential(value, "stake"))
        .transpose()?;
    let kind = match (payment, stake) {
        (Some(payment), Some(stake)) => AddressKind::Base { payment, stake },
        (Some(payment), None) => AddressKind::Enterprise { payment },
        (None, Some(stake)) => AddressKind::Reward { stake },
        _ => {
            return Err(ApiError::bad_request(
                "bad_request",
                "A payment or stake credential is required",
            ))
        }
    };
    let network = if state.config.network == crate::config::Network::Mainnet {
        1
    } else {
        0
    };
    let (address, _) = quetzal_address::codec::address(&kind, network)
        .map_err(|_| ApiError::bad_request("bad_request", "Invalid address credentials"))?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(serde_json::json!({"address":address})),
    ))
}
