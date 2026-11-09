//! Ledger-value selection with independent ADA-only collateral candidates.
//! Follows cardano-coin-selection 176611048d5b / cardano-balance-tx8975773.
use crate::seeded_selection::{self, Accounting, Selection};
use std::collections::BTreeMap;

pub struct Request {
    pub balance: seeded_selection::Request,
    pub accounting: Accounting,
    pub collateral_required: bool,
    pub maximum_collateral_inputs: usize,
    pub collateral_percentage: u64,
    /// Only independently eligible ADA-only entries belong in this map.
    pub collateral_candidates: BTreeMap<String, u128>,
}

pub struct Result {
    pub balance: Selection,
    pub collateral: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Balance(#[from] seeded_selection::Error),
    #[error("insufficient collateral: at least {minimum_collateral} lovelace required")]
    InsufficientCollateral {
        minimum_collateral: u128,
        largest_available: Vec<String>,
    },
}

pub fn select(request: &Request) -> std::result::Result<Result, Error> {
    let balance = seeded_selection::select_accounted(&request.balance, &request.accounting)?;
    let collateral = if request.collateral_required {
        let minimum = (balance.fee * u128::from(request.collateral_percentage)).div_ceil(100);
        collateral(
            &request.collateral_candidates,
            request.maximum_collateral_inputs,
            minimum,
        )?
    } else {
        Vec::new()
    };
    Ok(Result {
        balance,
        collateral,
    })
}

fn combinations_within_limit(n: usize, k: usize) -> bool {
    if k > n {
        return false;
    }
    let k = k.min(n - k);
    let mut value = 1u128;
    for i in 0..k {
        let Some(product) = value.checked_mul((n - i) as u128) else {
            return false;
        };
        value = product / (i + 1) as u128;
        if value > 1_000_000 {
            return false;
        }
    }
    true
}

fn best_of_size(
    coins: &[(String, u128)],
    size: usize,
    minimum: u128,
) -> Option<(u128, Vec<String>)> {
    fn visit(
        coins: &[(String, u128)],
        remaining: usize,
        start: usize,
        sum: u128,
        chosen: &mut Vec<String>,
        minimum: u128,
        best: &mut Option<(u128, Vec<String>)>,
    ) {
        if remaining == 0 {
            if sum >= minimum && best.as_ref().is_none_or(|(value, _)| sum < *value) {
                *best = Some((sum, chosen.clone()));
            }
            return;
        }
        if coins.len() - start < remaining {
            return;
        }
        for index in start..=coins.len() - remaining {
            chosen.push(coins[index].0.clone());
            visit(
                coins,
                remaining - 1,
                index + 1,
                sum + coins[index].1,
                chosen,
                minimum,
                best,
            );
            chosen.pop();
        }
    }
    let mut best = None;
    visit(coins, size, 0, 0, &mut Vec::new(), minimum, &mut best);
    best
}

fn collateral(
    coins: &BTreeMap<String, u128>,
    maximum: usize,
    minimum: u128,
) -> std::result::Result<Vec<String>, Error> {
    let mut ordered = coins
        .iter()
        .map(|(id, value)| (id.clone(), *value))
        .collect::<Vec<_>>();
    ordered.sort_by_key(|(_, value)| *value);
    if let Some(index) = ordered.iter().position(|(_, value)| *value >= minimum) {
        ordered.truncate(index + 1);
    }
    let mut best = None;
    for size in 1..=maximum.min(ordered.len()) {
        if combinations_within_limit(ordered.len(), size) {
            if let Some(candidate) = best_of_size(&ordered, size, minimum) {
                if best.as_ref().is_none_or(|(value, _)| candidate.0 < *value) {
                    best = Some(candidate);
                }
            }
        }
    }
    if let Some((_, mut ids)) = best {
        ids.sort();
        return Ok(ids);
    }
    let mut largest = coins
        .iter()
        .map(|(id, value)| (id.clone(), *value))
        .collect::<Vec<_>>();
    largest.sort_by_key(|a| std::cmp::Reverse(a.1));
    largest.truncate(maximum);
    let total: u128 = largest.iter().map(|(_, v)| *v).sum();
    if total < minimum {
        let mut ids = largest.into_iter().map(|(id, _)| id).collect::<Vec<_>>();
        ids.sort();
        return Err(Error::InsufficientCollateral {
            minimum_collateral: minimum,
            largest_available: ids,
        });
    }
    // The upstream fallback searches submaps of the largest permitted set.
    // Enumerating only fixed-size combinations avoids materializing its powerset.
    largest.sort_by(|a, b| a.0.cmp(&b.0));
    let mut best = (
        total,
        largest.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
    );
    if minimum == 0 {
        return Ok(Vec::new());
    }
    for size in 1..=largest.len() {
        if let Some(mut candidate) = best_of_size(&largest, size, minimum) {
            candidate.1.sort();
            if candidate.0 < best.0 || (candidate.0 == best.0 && candidate.1 < best.1) {
                best = candidate;
            }
        }
    }
    Ok(best.1)
}
