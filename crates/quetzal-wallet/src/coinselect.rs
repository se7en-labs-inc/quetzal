use crate::error::WalletError;
use quetzal_primitives::{Coin, UtxoEntry};
use rand::seq::SliceRandom;

/// Largest-first coin selection.
///
/// Sorts UTxOs by lovelace value descending, then greedily adds until the
/// accumulated sum is >= `target`.  Returns `Err(WalletError::InsufficientFunds)`
/// if the total available balance is less than `target`.
pub fn largest_first(utxos: &[UtxoEntry], target: Coin) -> Result<Vec<UtxoEntry>, WalletError> {
    let mut sorted: Vec<&UtxoEntry> = utxos.iter().collect();
    sorted.sort_by_key(|entry| std::cmp::Reverse(entry.value.coin));

    let mut selected = Vec::new();
    let mut total = Coin::ZERO;

    for utxo in sorted {
        selected.push(utxo.clone());
        total = total
            .checked_add(utxo.value.coin)
            .ok_or(WalletError::TxBuild("coin overflow".to_string()))?;
        if total >= target {
            return Ok(selected);
        }
    }

    Err(WalletError::InsufficientFunds)
}

/// Random-improve coin selection.
///
/// Phase 1 (random): shuffle available UTxOs and pick until sum >= target.
/// Phase 2 (improve): try to replace selected UTxOs with ones from the
/// remaining pool that bring the total closer to target * 2.
pub fn random_improve(utxos: &[UtxoEntry], target: Coin) -> Result<Vec<UtxoEntry>, WalletError> {
    if utxos.is_empty() {
        return Err(WalletError::InsufficientFunds);
    }

    let ideal = target.checked_add(target).unwrap_or(target); // cap at u64::MAX via saturation

    // Phase 1: random selection until sum >= target
    let mut rng = rand::thread_rng();
    let mut shuffled: Vec<&UtxoEntry> = utxos.iter().collect();
    shuffled.shuffle(&mut rng);

    let mut selected: Vec<UtxoEntry> = Vec::new();
    let mut remaining: Vec<&UtxoEntry> = Vec::new();
    let mut total = Coin::ZERO;

    for utxo in shuffled {
        if total >= target {
            remaining.push(utxo);
        } else {
            selected.push(utxo.clone());
            total = total
                .checked_add(utxo.value.coin)
                .ok_or(WalletError::TxBuild("coin overflow".to_string()))?;
        }
    }

    if total < target {
        return Err(WalletError::InsufficientFunds);
    }

    // Phase 2: improve — try swapping selected UTxOs with remaining ones
    // to bring total closer to `ideal`
    remaining.shuffle(&mut rng);
    for candidate in &remaining {
        // Distance of current total from ideal
        let current_dist = distance(total, ideal);

        // Try each selected UTxO as a swap candidate
        let mut best_swap: Option<(usize, Coin)> = None;
        for (i, sel) in selected.iter().enumerate() {
            let new_total_opt = total
                .checked_add(candidate.value.coin)
                .and_then(|t| t.checked_sub(sel.value.coin));
            if let Some(new_total) = new_total_opt {
                if new_total >= target {
                    let new_dist = distance(new_total, ideal);
                    if new_dist < current_dist {
                        match best_swap {
                            None => best_swap = Some((i, new_total)),
                            Some((_, best_total))
                                if distance(new_total, ideal) < distance(best_total, ideal) =>
                            {
                                best_swap = Some((i, new_total));
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        if let Some((idx, new_total)) = best_swap {
            selected[idx] = (*candidate).clone();
            total = new_total;
        }
    }

    Ok(selected)
}

/// Absolute distance between two Coin values.
fn distance(a: Coin, b: Coin) -> u64 {
    if a >= b {
        a.0 - b.0
    } else {
        b.0 - a.0
    }
}
