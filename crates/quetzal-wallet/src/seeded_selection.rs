//! Native counterpart of cardano-coin-selection 176611048d5b's Balance component.
//! Algorithm reference: Cardano Foundation / IOHK, Apache-2.0. RNG behavior is
//! pinned to random1.3.1 and splitmix0.1.3.1, including rejected candidate draws.
use crate::selection::{Bundle as InputBundle, Input};
use num_bigint::BigUint;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strategy {
    Minimal,
    #[default]
    Optimal,
}

#[derive(Debug, Deserialize)]
pub struct Request {
    pub utxos: Vec<Input>,
    pub outputs: Vec<InputBundle>,
    pub seed: u64,
    pub minimum_fee: u64,
    pub minimum_change_coin: u64,
    #[serde(default)]
    pub strategy: Strategy,
    #[serde(default = "maximum_coin")]
    pub maximum_output_coin: u64,
    #[serde(default = "maximum_quantity")]
    pub maximum_output_token_quantity: u64,
    #[serde(default = "maximum_assets")]
    pub maximum_assets_per_bundle: usize,
}
fn maximum_coin() -> u64 {
    45_000_000_000_000_000
}
fn maximum_quantity() -> u64 {
    i64::MAX as u64
}
fn maximum_assets() -> usize {
    32
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Bundle {
    pub coin: u128,
    pub assets: BTreeMap<String, u128>,
}
impl From<&InputBundle> for Bundle {
    fn from(value: &InputBundle) -> Self {
        Self {
            coin: u128::from(value.coin),
            assets: value
                .assets
                .iter()
                .filter(|(_, q)| **q > 0)
                .map(|(a, q)| (a.clone(), u128::from(*q)))
                .collect(),
        }
    }
}
impl Bundle {
    fn add(&mut self, other: &Self) {
        self.coin += other.coin;
        for (asset, quantity) in &other.assets {
            *self.assets.entry(asset.clone()).or_default() += quantity;
        }
    }
    fn difference(&self, other: &Self) -> Self {
        Self {
            coin: self.coin.saturating_sub(other.coin),
            assets: self
                .assets
                .iter()
                .filter_map(|(asset, quantity)| {
                    let q = quantity.saturating_sub(*other.assets.get(asset).unwrap_or(&0));
                    (q > 0).then(|| (asset.clone(), q))
                })
                .collect(),
        }
    }
    fn covers(&self, other: &Self) -> bool {
        self.coin >= other.coin
            && other
                .assets
                .iter()
                .all(|(a, q)| self.assets.get(a).unwrap_or(&0) >= q)
    }
}

#[derive(Debug, Serialize, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum Error {
    #[error("invalid selection input")]
    InvalidInput,
    #[error("empty UTxO")]
    EmptyUtxo,
    #[error("insufficient input balance")]
    BalanceInsufficient {
        available: Box<Bundle>,
        required: Box<Bundle>,
        shortfall: Box<Bundle>,
    },
    #[error("unable to construct change")]
    UnableToConstructChange {
        required_cost: u128,
        shortfall: u128,
    },
}

#[derive(Debug, Serialize)]
pub struct Selection {
    pub inputs: Vec<String>,
    pub change: Vec<Bundle>,
    pub fee: u128,
}

#[derive(Default)]
pub struct Accounting {
    pub extra_in: Bundle,
    pub extra_out: Bundle,
}

struct Random {
    seed: u64,
    gamma: u64,
}
impl Random {
    fn mix(mut n: u64) -> u64 {
        n = (n ^ (n >> 33)).wrapping_mul(0xff51afd7ed558ccd);
        n = (n ^ (n >> 33)).wrapping_mul(0xc4ceb9fe1a85ec53);
        n ^ (n >> 33)
    }
    fn new(seed: u64) -> Self {
        let mut gamma = seed.wrapping_add(0x9e3779b97f4a7c15);
        gamma = (gamma ^ (gamma >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
        gamma = (gamma ^ (gamma >> 27)).wrapping_mul(0x94d049bb133111eb);
        gamma = (gamma ^ (gamma >> 31)) | 1;
        if (gamma ^ (gamma >> 1)).count_ones() < 24 {
            gamma ^= 0xaaaaaaaaaaaaaaaa;
        }
        Self {
            seed: Self::mix(seed),
            gamma,
        }
    }
    fn next(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(self.gamma);
        Self::mix(self.seed)
    }
    fn index(&mut self, length: usize) -> usize {
        if length <= 1 {
            return 0;
        }
        let range = (length - 1) as u64;
        let mask = u64::MAX >> (range | 1).leading_zeros();
        loop {
            let value = self.next() & mask;
            if value <= range {
                return value as usize;
            }
        }
    }
}

/// Largest-remainder allocation, breaking ties by larger integral portions and
/// then original index, as the upstream exact rational implementation does.
fn partition(total: u128, weights: &[u128]) -> Result<Vec<u128>, Error> {
    let sum: u128 = weights.iter().sum();
    if sum == 0 {
        return Ok(vec![0; weights.len()]);
    }
    let divisor = BigUint::from(sum);
    let mut parts = Vec::with_capacity(weights.len());
    let mut priority = Vec::with_capacity(weights.len());
    for (index, weight) in weights.iter().enumerate() {
        let value = BigUint::from(total) * BigUint::from(*weight);
        let whole: u128 = (&value / &divisor)
            .try_into()
            .map_err(|_| Error::InvalidInput)?;
        priority.push((index, &value % &divisor, whole));
        parts.push(whole);
    }
    priority.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));
    let shortfall =
        usize::try_from(total - parts.iter().sum::<u128>()).map_err(|_| Error::InvalidInput)?;
    for (index, _, _) in priority.into_iter().take(shortfall) {
        parts[index] += 1;
    }
    Ok(parts)
}

fn equal_parts(total: u128, count: usize) -> Vec<u128> {
    let small = total / count as u128;
    let large = (total % count as u128) as usize;
    (0..count)
        .map(|index| small + u128::from(index >= count - large))
        .collect()
}

fn choose(
    asset: Option<&str>,
    inputs: &[(&Input, Bundle)],
    selected: &BTreeSet<usize>,
    rng: &mut Random,
) -> Option<usize> {
    for size in [1, 2, usize::MAX] {
        let matching: Vec<_> = inputs
            .iter()
            .enumerate()
            .filter_map(|(index, (_, value))| {
                let contains = asset.map_or(value.coin > 0, |a| value.assets.contains_key(a));
                let count = value.assets.len() + usize::from(value.coin > 0);
                (!selected.contains(&index) && contains && (size == usize::MAX || count == size))
                    .then_some(index)
            })
            .collect();
        if !matching.is_empty() {
            return Some(matching[rng.index(matching.len())]);
        }
    }
    None
}

fn split_assets(bundle: Bundle, limit: usize) -> Vec<Bundle> {
    if bundle.assets.len() <= limit {
        return vec![bundle];
    }
    let counts = equal_parts(bundle.assets.len() as u128, 2);
    let coins = equal_parts(bundle.coin, 2);
    let mut assets = bundle.assets.into_iter();
    let mut result = Vec::new();
    for (count, coin) in counts.into_iter().zip(coins) {
        result.extend(split_assets(
            Bundle {
                coin,
                assets: assets.by_ref().take(count as usize).collect(),
            },
            limit,
        ));
    }
    result
}

fn change(
    request: &Request,
    inputs: &[&Bundle],
    outputs: &[Bundle],
    extra: u128,
    accounting: &Accounting,
) -> Result<Vec<Bundle>, Error> {
    let mut total = accounting.extra_in.clone();
    total.coin += extra;
    for input in inputs {
        total.add(input);
    }
    let mut requested = accounting.extra_out.clone();
    for output in outputs {
        requested.add(output);
    }
    let excess = total.difference(&requested);
    let fee = u128::from(request.minimum_fee);
    let available = excess
        .coin
        .checked_sub(fee)
        .ok_or(Error::UnableToConstructChange {
            required_cost: fee,
            shortfall: fee.saturating_sub(excess.coin),
        })?;
    let count = outputs.len();
    let mut pairs = outputs
        .iter()
        .map(|o| Bundle {
            coin: o.coin,
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let specified = outputs
        .iter()
        .flat_map(|o| o.assets.keys().cloned())
        .collect::<BTreeSet<_>>();
    for asset in &specified {
        let weights: Vec<_> = outputs
            .iter()
            .map(|o| *o.assets.get(asset).unwrap_or(&0))
            .collect();
        let portions = partition(*excess.assets.get(asset).unwrap_or(&0), &weights)?;
        for (pair, quantity) in pairs.iter_mut().zip(portions) {
            if quantity > 0 {
                pair.assets.insert(asset.clone(), quantity);
            }
        }
    }
    pairs.sort_by_key(|p| p.assets.len());
    for asset in excess.assets.keys().filter(|a| !specified.contains(*a)) {
        let mut quantities = inputs
            .iter()
            .filter_map(|b| b.assets.get(asset).copied())
            .collect::<Vec<_>>();
        quantities.sort_unstable();
        while quantities.len() > count {
            let sum = quantities.remove(0) + quantities.remove(0);
            let position = quantities.partition_point(|q| *q < sum);
            quantities.insert(position, sum);
        }
        while quantities.len() < count {
            quantities.insert(0, 0);
        }
        if let Some(last) = quantities.last_mut() {
            *last += accounting.extra_in.assets.get(asset).copied().unwrap_or(0);
        }
        let mut burn = accounting.extra_out.assets.get(asset).copied().unwrap_or(0);
        for quantity in &mut quantities {
            let used = (*quantity).min(burn);
            *quantity -= used;
            burn -= used;
        }
        for (pair, quantity) in pairs.iter_mut().zip(quantities) {
            if quantity > 0 {
                pair.assets.insert(asset.clone(), quantity);
            }
        }
    }
    pairs = pairs
        .into_iter()
        .flat_map(|bundle| split_assets(bundle, request.maximum_assets_per_bundle))
        .collect();
    let mut split = Vec::new();
    for pair in pairs {
        let maximum = pair.assets.values().copied().max().unwrap_or(0);
        let count = maximum
            .div_ceil(u128::from(request.maximum_output_token_quantity))
            .max(1);
        let count = usize::try_from(count).map_err(|_| Error::InvalidInput)?;
        // The encoded input is bounded by the adapter; reject unrepresentable
        // allocations rather than panicking or silently truncating quantities.
        if count > 1_000_000 {
            return Err(Error::InvalidInput);
        }
        let mut bundles = equal_parts(pair.coin, count)
            .into_iter()
            .map(|coin| Bundle {
                coin,
                ..Default::default()
            })
            .collect::<Vec<_>>();
        for (asset, quantity) in pair.assets {
            for (bundle, quantity) in bundles.iter_mut().zip(equal_parts(quantity, count)) {
                if quantity > 0 {
                    bundle.assets.insert(asset.clone(), quantity);
                }
            }
        }
        split.extend(bundles);
    }
    let minimum = u128::from(request.minimum_change_coin);
    let mut start = 0;
    while available < minimum * (split.len() - start) as u128 {
        if !split[start].assets.is_empty() {
            return Err(Error::UnableToConstructChange {
                required_cost: fee,
                shortfall: minimum * (split.len() - start) as u128 - available,
            });
        }
        start += 1;
        if start == split.len() {
            return Ok(Vec::new());
        }
    }
    let mut pairs = split.into_iter().skip(start).collect::<Vec<_>>();
    let remaining = available - minimum * pairs.len() as u128;
    let weights = pairs.iter().map(|p| p.coin).collect::<Vec<_>>();
    if weights.iter().sum::<u128>() == 0 {
        return Err(Error::InvalidInput);
    }
    for (pair, coin) in pairs.iter_mut().zip(partition(remaining, &weights)?) {
        pair.coin = minimum + coin;
    }
    Ok(pairs)
}

pub fn select(request: &Request) -> Result<Selection, Error> {
    select_accounted(request, &Accounting::default())
}

pub fn select_accounted(request: &Request, accounting: &Accounting) -> Result<Selection, Error> {
    if request.maximum_output_coin == 0
        || request.maximum_output_coin < request.minimum_change_coin
        || request.maximum_output_token_quantity == 0
        || request.maximum_assets_per_bundle == 0
    {
        return Err(Error::InvalidInput);
    }
    let mut names = BTreeSet::new();
    if request.utxos.iter().any(|i| !names.insert(&i.id)) {
        return Err(Error::InvalidInput);
    }
    if request
        .utxos
        .iter()
        .any(|input| input.value.clone().into_value().is_err())
        || request
            .outputs
            .iter()
            .any(|output| output.clone().into_value().is_err())
    {
        return Err(Error::InvalidInput);
    }
    let mut inputs = request
        .utxos
        .iter()
        .filter(|i| i.eligible)
        .map(|i| (i, Bundle::from(&i.value)))
        .collect::<Vec<_>>();
    inputs.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let mut available = Bundle::default();
    for (_, value) in &inputs {
        available.add(value);
    }
    if available.coin > u128::from(request.maximum_output_coin) {
        return Err(Error::InvalidInput);
    }
    if request.outputs.iter().any(|o| {
        o.coin < request.minimum_change_coin
            || o.coin > request.maximum_output_coin
            || o.assets
                .values()
                .any(|q| *q > request.maximum_output_token_quantity)
            || o.assets.values().filter(|q| **q > 0).count() > request.maximum_assets_per_bundle
    }) {
        return Err(Error::InvalidInput);
    }
    let mut outputs = request.outputs.iter().map(Bundle::from).collect::<Vec<_>>();
    let extra = if outputs.is_empty() {
        outputs.push(Bundle {
            coin: 1,
            ..Default::default()
        });
        1
    } else {
        0
    };
    let mut required = Bundle::default();
    for output in &outputs {
        required.add(output);
    }
    required.add(&accounting.extra_out);
    required = required.difference(&accounting.extra_in);
    required.coin = required.coin.saturating_sub(extra);
    if !available.covers(&required) {
        let shortfall = required.difference(&available);
        return Err(Error::BalanceInsufficient {
            available: Box::new(available),
            required: Box::new(required),
            shortfall: Box::new(shortfall),
        });
    }
    let mut rng = Random::new(request.seed);
    let mut selected = BTreeSet::new();
    let mut balance = Bundle::default();
    let mut processors = required
        .assets
        .iter()
        .rev()
        .map(|(asset, quantity)| (Some(asset.as_str()), *quantity))
        .collect::<Vec<_>>();
    processors.push((None, required.coin));
    while !processors.is_empty() {
        let mut again = Vec::new();
        for (asset, minimum) in processors {
            let current = asset.map_or(balance.coin, |a| *balance.assets.get(a).unwrap_or(&0));
            if let Some(index) = choose(asset, &inputs, &selected, &mut rng) {
                let added = asset.map_or(inputs[index].1.coin, |a| {
                    *inputs[index].1.assets.get(a).unwrap_or(&0)
                });
                let target = minimum
                    * match request.strategy {
                        Strategy::Minimal => 1,
                        Strategy::Optimal => 2,
                    };
                if current < minimum
                    || (current + added).abs_diff(target) < current.abs_diff(target)
                {
                    selected.insert(index);
                    balance.add(&inputs[index].1);
                    again.push((asset, minimum));
                }
            }
        }
        processors = again;
    }
    if selected.is_empty() {
        let index = choose(None, &inputs, &selected, &mut rng).ok_or(Error::EmptyUtxo)?;
        selected.insert(index);
    }
    loop {
        let bundles = selected
            .iter()
            .map(|index| &inputs[*index].1)
            .collect::<Vec<_>>();
        let result = change(request, &bundles, &outputs, extra, accounting);
        if let Ok(change) = &result {
            if change.len() >= outputs.len() {
                return finish(request, &inputs, &selected, change.clone(), accounting);
            }
        }
        if let Some(index) = choose(None, &inputs, &selected, &mut rng) {
            selected.insert(index);
        } else {
            return result
                .and_then(|change| finish(request, &inputs, &selected, change, accounting));
        }
    }
}

fn finish(
    request: &Request,
    inputs: &[(&Input, Bundle)],
    selected: &BTreeSet<usize>,
    change: Vec<Bundle>,
    accounting: &Accounting,
) -> Result<Selection, Error> {
    let input_coin: u128 = selected.iter().map(|i| inputs[*i].1.coin).sum();
    let output_coin: u128 = request.outputs.iter().map(|o| u128::from(o.coin)).sum();
    let change_coin: u128 = change.iter().map(|o| o.coin).sum();
    let fee = (input_coin + accounting.extra_in.coin)
        .checked_sub(output_coin + change_coin + accounting.extra_out.coin)
        .ok_or(Error::InvalidInput)?;
    Ok(Selection {
        inputs: selected
            .iter()
            .rev()
            .map(|i| inputs[*i].0.id.clone())
            .collect(),
        change,
        fee,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splitmix_matches_upstream_published_examples() {
        let mut random = Random::new(1337);
        assert_eq!(
            [random.next(), random.next(), random.next()],
            [0xb5c19e300e8b07b3, 0xd600e0e216c0ac76, 0xc54efc3b3cc5af29]
        );
        let random = Random::new(42);
        assert_eq!(
            (random.seed, random.gamma),
            (9297814886316923340, 13679457532755275413)
        );
        let mut random = Random::new(137);
        assert_eq!(
            (0..10).map(|_| 20 + random.index(11)).collect::<Vec<_>>(),
            [23, 21, 28, 25, 28, 28, 26, 25, 29, 27]
        );
    }
}
