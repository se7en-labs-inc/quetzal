//! Views of locally submitted transactions before ledger confirmation.
use crate::WalletError;
impl crate::WalletService {
    pub fn mark_pending_submission(
        &self,
        wallet: &quetzal_primitives::WalletId,
        id: &quetzal_primitives::TxHash,
        slot: u64,
    ) -> Result<(), WalletError> {
        self.db.mark_pending_submission(wallet, id, slot)
    }
}
use pallas_primitives::{conway, Fragment};
use pallas_traverse::MultiEraTx;
use quetzal_primitives::tx::{BlockHeight, CoinBundle, TxBlock, TxInput, TxOutput, TxTime};
use quetzal_primitives::{
    Address, AddressEntry, Coin, PendingTx, TxDirection, TxHash, TxRecord, TxStatus, UtxoEntry,
    UtxoId,
};

pub(crate) fn record(
    pending: &PendingTx,
    addresses: &[AddressEntry],
    utxos: &[UtxoEntry],
    status: TxStatus,
) -> Result<TxRecord, WalletError> {
    let tx = conway::Tx::decode_fragment(&pending.cbor)
        .map_err(|e| WalletError::TxBuild(e.to_string()))?;
    let mut inputs = Vec::new();
    let mut consumed = Coin::ZERO;
    for input in tx.transaction_body.inputs.iter() {
        let id = UtxoId::new(
            TxHash(*input.transaction_id),
            u32::try_from(input.index)
                .map_err(|_| WalletError::TxBuild("input index overflow".into()))?,
        );
        let owned = utxos.iter().find(|utxo| utxo.id == id);
        if let Some(owned) = owned {
            consumed = consumed
                .checked_add(owned.value.coin)
                .ok_or_else(|| WalletError::TxBuild("input amount overflow".into()))?;
        }
        inputs.push(TxInput {
            id: id.tx_hash,
            index: id.output_index,
            address: owned.map(|utxo| utxo.address.clone()),
            amount: owned.map(|utxo| CoinBundle::lovelace(utxo.value.coin)),
            assets: owned
                .map(|utxo| crate::ledger::asset_amounts(&utxo.value))
                .unwrap_or_default(),
        });
    }
    let multi =
        MultiEraTx::decode(&pending.cbor).map_err(|e| WalletError::TxBuild(e.to_string()))?;
    let mut outputs = Vec::new();
    let mut received = Coin::ZERO;
    let mut external = Coin::ZERO;
    for output in multi.outputs() {
        let address = Address(
            output
                .address()
                .map_err(|e| WalletError::TxBuild(e.to_string()))?
                .to_string(),
        );
        let value = crate::ledger::value(&output.value());
        if addresses.iter().any(|entry| entry.address == address) {
            received = received
                .checked_add(value.coin)
                .ok_or_else(|| WalletError::TxBuild("output amount overflow".into()))?;
        } else {
            external = external
                .checked_add(value.coin)
                .ok_or_else(|| WalletError::TxBuild("output amount overflow".into()))?;
        }
        outputs.push(TxOutput {
            address,
            amount: CoinBundle::lovelace(value.coin),
            assets: crate::ledger::asset_amounts(&value),
        });
    }
    let amount = if consumed > Coin::ZERO {
        Coin(consumed.0.abs_diff(received.0))
    } else {
        external
            .checked_add(Coin(tx.transaction_body.fee))
            .ok_or_else(|| WalletError::TxBuild("outgoing amount overflow".into()))?
    };
    let metadata = multi
        .metadata()
        .as_alonzo()
        .map(crate::metadata::decode_ledger)
        .transpose()?;
    let stake_entry = addresses.iter().find(|entry| {
        entry.path.role == quetzal_primitives::AddressRole::External && entry.path.address_ix == 0
    });
    let stake_hash = stake_entry
        .and_then(|entry| {
            entry
                .address
                .as_str()
                .parse::<pallas_addresses::Address>()
                .ok()
        })
        .and_then(|address| match address {
            pallas_addresses::Address::Shelley(address) => match address.delegation() {
                pallas_addresses::ShelleyDelegationPart::Key(hash) => Some(**hash),
                _ => None,
            },
            _ => None,
        });
    let staking = stake_hash
        .map(|hash| crate::staking::owned_certificates(&multi, &hash))
        .transpose()?
        .unwrap_or_default();
    Ok(TxRecord {
        id: pending.id.clone(),
        slot: pending.submitted_slot,
        block_hash: [0; 32],
        block_height: pending.submitted_height,
        direction: if consumed > Coin::ZERO && received > consumed {
            TxDirection::Incoming
        } else {
            TxDirection::Outgoing
        },
        amount,
        fee: Some(Coin(tx.transaction_body.fee)),
        inputs,
        outputs,
        status,
        inserted_at: TxTime {
            time: pending.submitted_at.clone(),
            block: TxBlock {
                slot_no: pending.submitted_slot,
                epoch_no: 0,
                height: BlockHeight::new(pending.submitted_height),
            },
        },
        metadata,
        withdrawals: crate::ledger::withdrawals(&multi)?,
        expires_at_slot: pending.expiry_slot,
        certificates: staking.encoded()?,
        deposit_taken: Coin(staking.deposit_taken),
        deposit_returned: Coin(staking.deposit_returned),
        stake_derivation_path: stake_entry.map(|entry| quetzal_primitives::DerivationPath {
            account_ix: entry.path.account_ix,
            role: quetzal_primitives::AddressRole::Stake,
            address_ix: 0,
        }),
    })
}
