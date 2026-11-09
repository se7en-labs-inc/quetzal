//! Deterministic selection of eligible multi-asset inputs with explicit limits.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Bundle {
    pub coin: u64,
    #[serde(default)]
    pub assets: BTreeMap<String, u64>,
}
impl Bundle {
    pub fn from_value(value: &quetzal_primitives::Value) -> Self {
        let assets = value
            .assets
            .iter()
            .flat_map(|(policy, names)| {
                names
                    .iter()
                    .map(move |(name, quantity)| (format!("{policy}.{name}"), *quantity))
            })
            .collect();
        Self {
            coin: value.coin.0,
            assets,
        }
    }
    pub fn into_value(self) -> Result<quetzal_primitives::Value, crate::WalletError> {
        let mut value = quetzal_primitives::Value::lovelace(quetzal_primitives::Coin(self.coin));
        for (asset, quantity) in self.assets {
            let (policy, name) = asset
                .split_once('.')
                .ok_or_else(|| crate::WalletError::TxBuild("invalid asset identifier".into()))?;
            if hex::decode(policy).map_or(true, |bytes| bytes.len() != 28)
                || hex::decode(name).map_or(true, |bytes| bytes.len() > 32)
            {
                return Err(crate::WalletError::TxBuild(
                    "invalid asset identifier".into(),
                ));
            }
            if quantity > 0 {
                value
                    .assets
                    .entry(policy.into())
                    .or_default()
                    .insert(name.into(), quantity);
            }
        }
        Ok(value)
    }
    fn add(&mut self, other: &Self) -> Option<()> {
        self.coin = self.coin.checked_add(other.coin)?;
        for (asset, quantity) in &other.assets {
            let total = self.assets.entry(asset.clone()).or_default();
            *total = total.checked_add(*quantity)?;
        }
        Some(())
    }
    fn subtract(&self, other: &Self) -> Option<Self> {
        let mut result = self.clone();
        result.coin = result.coin.checked_sub(other.coin)?;
        for (asset, quantity) in &other.assets {
            if *quantity == 0 {
                continue;
            }
            let total = result.assets.get_mut(asset)?;
            *total = total.checked_sub(*quantity)?;
        }
        result.assets.retain(|_, quantity| *quantity > 0);
        Some(result)
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct Input {
    pub id: String,
    pub value: Bundle,
    pub eligible: bool,
}
#[derive(Debug, Deserialize)]
pub struct Request {
    pub utxos: Vec<Input>,
    pub outputs: Vec<Bundle>,
    pub fee: u64,
    pub minimum_change_coin: u64,
    pub maximum_inputs: usize,
}
#[derive(Debug, Serialize)]
pub struct Selection {
    pub inputs: Vec<String>,
    pub change: Vec<Bundle>,
    pub fee: u64,
}
pub fn select(request: &Request) -> Result<Selection, crate::WalletError> {
    select_accounted(request, &Bundle::default(), &Bundle::default())
}

/// Select real spending inputs with additional value supplied or consumed by
/// the ledger (mint/burn, rewards, and stake deposits).
pub fn select_accounted(
    request: &Request,
    extra_in: &Bundle,
    extra_out: &Bundle,
) -> Result<Selection, crate::WalletError> {
    let insufficient = || crate::WalletError::InsufficientFunds;
    let mut ids = BTreeSet::new();
    if request.utxos.iter().any(|input| !ids.insert(&input.id)) {
        return Err(crate::WalletError::CoinSelection);
    }
    let mut target = Bundle {
        coin: request.fee,
        ..Default::default()
    };
    for output in &request.outputs {
        target.add(output).ok_or_else(insufficient)?;
    }
    target.add(extra_out).ok_or_else(insufficient)?;
    let mut selected = Vec::new();
    let mut total = extra_in.clone();
    let mut remaining: Vec<&Input> = request
        .utxos
        .iter()
        .filter(|input| input.eligible)
        .collect();
    loop {
        if !selected.is_empty() {
            if let Some(change) = total.subtract(&target) {
                let empty = change.coin == 0 && change.assets.is_empty();
                if empty || change.coin >= request.minimum_change_coin {
                    return Ok(Selection {
                        inputs: selected,
                        change: if empty { vec![] } else { vec![change] },
                        fee: request.fee,
                    });
                }
            }
        }
        if selected.len() >= request.maximum_inputs {
            return Err(insufficient());
        }
        // First cover missing native assets; then prefer larger ADA inputs.
        let best = remaining
            .iter()
            .enumerate()
            .max_by_key(|(_, input)| {
                let coverage: u128 = target
                    .assets
                    .iter()
                    .map(|(asset, required)| {
                        let missing =
                            required.saturating_sub(total.assets.get(asset).copied().unwrap_or(0));
                        u128::from(missing.min(input.value.assets.get(asset).copied().unwrap_or(0)))
                    })
                    .sum();
                (
                    coverage,
                    input.value.coin,
                    std::cmp::Reverse(input.id.as_str()),
                )
            })
            .map(|(index, _)| index)
            .ok_or_else(insufficient)?;
        let input = remaining.remove(best);
        total.add(&input.value).ok_or_else(insufficient)?;
        selected.push(input.id.clone());
    }
}
