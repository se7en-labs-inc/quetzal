use pallas_addresses::{
    Address, Network, ShelleyAddress, ShelleyDelegationPart, ShelleyPaymentPart, StakeAddress,
};
use pallas_crypto::hash::Hasher;
use quetzal_crypto::Bip32PublicKey;

use crate::AddressError;

/// Hash a 32-byte Ed25519 verification key with Blake2b-224 to get a 28-byte key hash.
fn vk_to_key_hash(vk_bytes: &[u8]) -> pallas_crypto::hash::Hash<28> {
    Hasher::<224>::hash(vk_bytes)
}

/// Derive a payment key at `role / ix` via soft derivation from `account_pub`,
/// returning the 32-byte vk bytes.
fn derive_payment_vk(
    account_pub: &Bip32PublicKey,
    role: u32,
    ix: u32,
) -> Result<[u8; 32], AddressError> {
    let role_key = account_pub.derive(role)?;
    let addr_key = role_key.derive(ix)?;
    Ok(addr_key.to_vk_bytes())
}

/// Derive a stake key at role=2 / `ix` via soft derivation from `account_pub`,
/// returning the 32-byte vk bytes.
fn derive_stake_vk(account_pub: &Bip32PublicKey, ix: u32) -> Result<[u8; 32], AddressError> {
    let stake_role = account_pub.derive(2)?;
    let stake_key = stake_role.derive(ix)?;
    Ok(stake_key.to_vk_bytes())
}

/// Enterprise address (no staking): payment key only.
///
/// Derives the payment key at `role / ix`, hashes it with Blake2b-224, and
/// returns the bech32-encoded Shelley enterprise address.
pub fn enterprise_address(
    account_pub: &Bip32PublicKey,
    role: u32,
    ix: u32,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    let vk = derive_payment_vk(account_pub, role, ix)?;
    enterprise_from_public_key(&vk, network)
}

/// Encode an enterprise payment address from an already derived verification key.
pub fn enterprise_from_public_key(
    vk: &[u8; 32],
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    let payment_hash = vk_to_key_hash(vk);
    let payment = ShelleyPaymentPart::key_hash(payment_hash);
    let shelley = ShelleyAddress::new(network, payment, ShelleyDelegationPart::Null);
    let bech32 = shelley.to_bech32()?;
    Ok(quetzal_primitives::Address(bech32))
}

/// Base address: payment key + stake key delegation.
///
/// Derives the payment key at role=0 / `payment_ix` and the stake key at
/// role=2 / `stake_ix`, then returns the bech32-encoded Shelley base address.
pub fn base_address(
    account_pub: &Bip32PublicKey,
    payment_ix: u32,
    stake_ix: u32,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    base_address_for_role(account_pub, 0, payment_ix, stake_ix, network)
}

pub fn base_address_for_role(
    account_pub: &Bip32PublicKey,
    role: u32,
    payment_ix: u32,
    stake_ix: u32,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    let payment_vk = derive_payment_vk(account_pub, role, payment_ix)?;
    let stake_vk = derive_stake_vk(account_pub, stake_ix)?;

    let payment = ShelleyPaymentPart::key_hash(vk_to_key_hash(&payment_vk));
    let delegation = ShelleyDelegationPart::key_hash(vk_to_key_hash(&stake_vk));

    let shelley = ShelleyAddress::new(network, payment, delegation);
    let bech32 = shelley.to_bech32()?;
    Ok(quetzal_primitives::Address(bech32))
}

/// Enterprise address with a script payment credential (shared/multi-sig wallets).
///
/// Takes the 28-byte native script hash and builds a `addr1z…` address.
pub fn script_enterprise_address(
    script_hash: pallas_crypto::hash::Hash<28>,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    let payment = ShelleyPaymentPart::script_hash(script_hash);
    let shelley = ShelleyAddress::new(network, payment, ShelleyDelegationPart::Null);
    let bech32 = shelley.to_bech32()?;
    Ok(quetzal_primitives::Address(bech32))
}

/// Stake (reward) address derived from the stake key at role=2 / `ix`.
///
/// Returns the `stake1…` (mainnet) or `stake_test1…` (testnet) bech32 string.
pub fn stake_address(
    account_pub: &Bip32PublicKey,
    ix: u32,
    network: Network,
) -> Result<quetzal_primitives::Address, AddressError> {
    let stake_vk = derive_stake_vk(account_pub, ix)?;
    let stake_hash = vk_to_key_hash(&stake_vk);

    // Build a StakeAddress via the pallas type using a dummy ShelleyAddress conversion.
    // ShelleyAddress -> StakeAddress via TryFrom works when delegation part is Key.
    let dummy_payment = ShelleyPaymentPart::key_hash(stake_hash);
    let delegation = ShelleyDelegationPart::key_hash(stake_hash);
    let dummy_shelley = ShelleyAddress::new(network, dummy_payment, delegation);
    let stake_addr: StakeAddress = dummy_shelley.try_into()?;
    let addr: Address = stake_addr.into();
    let bech32 = addr.to_bech32()?;
    Ok(quetzal_primitives::Address(bech32))
}
