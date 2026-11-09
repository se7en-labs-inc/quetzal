//! Durable pool production and registration counts, reversible on rollback.
use super::*;
use sled::transaction::{
    ConflictableTransactionError as Conflict, TransactionError, Transactional, TransactionalTree,
};

#[derive(serde::Serialize, serde::Deserialize)]
struct PoolBlock {
    pool: Option<[u8; 28]>,
    registrations: Vec<[u8; 28]>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum StoredBlock {
    Current(PoolBlock),
    Legacy(Option<[u8; 28]>),
}

fn transaction_error(error: TransactionError<WalletError>) -> WalletError {
    match error {
        TransactionError::Abort(error) => error,
        TransactionError::Storage(error) => WalletError::Db(error),
    }
}

fn adjust(tree: &TransactionalTree, pool: &[u8], add: bool) -> Result<(), Conflict<WalletError>> {
    let count: u64 = tree
        .get(pool)?
        .map(|bytes| cbor_decode(&bytes))
        .transpose()
        .map_err(|error| Conflict::Abort(WalletError::from(error)))?
        .unwrap_or(0);
    let count = if add {
        count.checked_add(1)
    } else {
        count.checked_sub(1)
    }
    .ok_or_else(|| {
        Conflict::Abort(WalletError::TxBuild(
            "pool counter overflow or underflow".into(),
        ))
    })?;
    tree.insert(
        pool,
        cbor_encode(&count).map_err(|error| Conflict::Abort(WalletError::from(error)))?,
    )?;
    Ok(())
}

impl WalletDb {
    pub(crate) fn pool_block(
        &self,
        slot: u64,
        hash: [u8; 32],
        pool: Option<[u8; 28]>,
        registrations: &[[u8; 28]],
    ) -> Result<(), WalletError> {
        let blocks = self.db.open_tree("pool_chain")?;
        let counts = self.db.open_tree("pool_block_counts")?;
        let registered = self.db.open_tree("pool_registration_counts")?;
        let mut key = slot_key(slot).to_vec();
        key.extend(hash);
        let encoded = cbor_encode(&PoolBlock {
            pool,
            registrations: registrations.to_vec(),
        })?;
        (&blocks, &counts, &registered)
            .transaction(|(blocks, counts, registered)| {
                if blocks.get(key.as_slice())?.is_some() {
                    return Ok(());
                }
                if let Some(pool) = pool {
                    adjust(counts, &pool, true)?;
                }
                for pool in registrations {
                    adjust(registered, pool, true)?;
                }
                blocks.insert(key.as_slice(), encoded.as_slice())?;
                Ok(())
            })
            .map_err(transaction_error)
    }

    pub(crate) fn rollback_pool_blocks(&self, slot: u64) -> Result<(), WalletError> {
        let blocks = self.db.open_tree("pool_chain")?;
        let counts = self.db.open_tree("pool_block_counts")?;
        let registered = self.db.open_tree("pool_registration_counts")?;
        let mut end = slot_key(slot).to_vec();
        end.extend([255; 32]);
        let removed = blocks
            .range((std::ops::Bound::Excluded(end), std::ops::Bound::Unbounded))
            .collect::<Result<Vec<_>, _>>()?;
        (&blocks, &counts, &registered)
            .transaction(|(blocks, counts, registered)| {
                for (key, bytes) in &removed {
                    if blocks.get(key.as_ref())?.is_none() {
                        continue;
                    }
                    let block: StoredBlock = cbor_decode(bytes)
                        .map_err(|error| Conflict::Abort(WalletError::from(error)))?;
                    let block = match block {
                        StoredBlock::Current(block) => block,
                        StoredBlock::Legacy(pool) => PoolBlock {
                            pool,
                            registrations: vec![],
                        },
                    };
                    if let Some(pool) = block.pool {
                        adjust(counts, &pool, false)?;
                    }
                    for pool in block.registrations {
                        adjust(registered, &pool, false)?;
                    }
                    blocks.remove(key.as_ref())?;
                }
                Ok(())
            })
            .map_err(transaction_error)
    }

    pub(crate) fn pool_block_counts(
        &self,
    ) -> Result<std::collections::BTreeMap<Vec<u8>, u64>, WalletError> {
        self.db
            .open_tree("pool_block_counts")?
            .iter()
            .map(|row| {
                let (key, value) = row?;
                Ok((key.to_vec(), cbor_decode(&value)?))
            })
            .collect()
    }

    pub(crate) fn registered_pools(
        &self,
    ) -> Result<std::collections::BTreeSet<Vec<u8>>, WalletError> {
        let mut pools = std::collections::BTreeSet::new();
        for row in self.db.open_tree("pool_registration_counts")?.iter() {
            let (key, value) = row?;
            let count: u64 = cbor_decode(&value)?;
            if count > 0 {
                pools.insert(key.to_vec());
            }
        }
        Ok(pools)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counts_registrations_replay_same_slot_blocks_and_rollback_atomically() {
        let db = WalletDb {
            db: sled::Config::new().temporary(true).open().unwrap(),
        };
        db.pool_block(10, [1; 32], Some([1; 28]), &[[1; 28]])
            .unwrap();
        db.pool_block(10, [1; 32], Some([1; 28]), &[[1; 28]])
            .unwrap();
        db.pool_block(10, [2; 32], None, &[]).unwrap();
        db.pool_block(11, [3; 32], Some([1; 28]), &[[1; 28]])
            .unwrap();
        db.pool_block(12, [4; 32], Some([2; 28]), &[[2; 28]])
            .unwrap();
        let before = db.pool_block_counts().unwrap();
        assert_eq!(before[&vec![1; 28]], 2);
        assert_eq!(before[&vec![2; 28]], 1);
        assert_eq!(db.registered_pools().unwrap().len(), 2);
        db.rollback_pool_blocks(10).unwrap();
        let after = db.pool_block_counts().unwrap();
        assert_eq!(after[&vec![1; 28]], 1);
        assert_eq!(after[&vec![2; 28]], 0);
        assert_eq!(
            db.registered_pools().unwrap(),
            std::collections::BTreeSet::from([vec![1; 28]])
        );
        db.rollback_pool_blocks(10).unwrap();
        assert_eq!(db.pool_block_counts().unwrap(), after);
        db.pool_block(11, [3; 32], Some([1; 28]), &[[1; 28]])
            .unwrap();
        db.pool_block(12, [4; 32], Some([2; 28]), &[[2; 28]])
            .unwrap();
        assert_eq!(db.pool_block_counts().unwrap(), before);
        db.rollback_pool_blocks(0).unwrap();
        assert!(db.registered_pools().unwrap().is_empty());
    }

    #[test]
    fn rolls_back_the_earlier_unreleased_production_only_record_format() {
        let db = WalletDb {
            db: sled::Config::new().temporary(true).open().unwrap(),
        };
        let pool = [1; 28];
        let mut key = slot_key(10).to_vec();
        key.extend([1; 32]);
        db.db
            .open_tree("pool_chain")
            .unwrap()
            .insert(key, cbor_encode(&Some(pool)).unwrap())
            .unwrap();
        db.db
            .open_tree("pool_block_counts")
            .unwrap()
            .insert(pool, cbor_encode(&1u64).unwrap())
            .unwrap();
        db.rollback_pool_blocks(0).unwrap();
        assert_eq!(db.pool_block_counts().unwrap()[&pool.to_vec()], 0);
    }
}
