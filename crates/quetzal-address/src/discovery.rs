use pallas_addresses::Network;
use quetzal_crypto::Bip32PublicKey;

use crate::shelley::enterprise_address;

/// BIP-44 / CIP-1852 gap limit: scan this many consecutive unused addresses
/// before concluding that no further addresses exist on the chain.
pub const GAP_LIMIT: u32 = 20;

/// Sequential address discovery using the gap-limit algorithm.
///
/// Tracks the "watermark" for the external (receive) and internal (change)
/// chains independently. Call [`gap_addresses_external`] or
/// [`gap_addresses_internal`] to get the next batch of addresses to query,
/// then call [`mark_used_external`] / [`mark_used_internal`] when you learn
/// that an index has been used on-chain.
pub struct AddressDiscovery {
    /// Next unscanned index on the external (role=0) chain.
    external_next: u32,
    /// Next unscanned index on the internal (role=1) chain.
    internal_next: u32,
}

impl AddressDiscovery {
    /// Create a new discovery state starting from index 0 on both chains.
    pub fn new() -> Self {
        AddressDiscovery {
            external_next: 0,
            internal_next: 0,
        }
    }

    /// Returns the next [`GAP_LIMIT`] external addresses starting from the
    /// current watermark, together with their address indices.
    ///
    /// The caller should query the chain for each address and call
    /// [`mark_used_external`] for any index that has received funds.
    pub fn gap_addresses_external(
        &self,
        account: &Bip32PublicKey,
        network: Network,
    ) -> Vec<(u32, quetzal_primitives::Address)> {
        self.gap_addresses(account, network, 0, self.external_next)
    }

    /// Returns the next [`GAP_LIMIT`] internal addresses starting from the
    /// current watermark, together with their address indices.
    pub fn gap_addresses_internal(
        &self,
        account: &Bip32PublicKey,
        network: Network,
    ) -> Vec<(u32, quetzal_primitives::Address)> {
        self.gap_addresses(account, network, 1, self.internal_next)
    }

    /// Advance the external watermark if `ix` is at or past the current edge.
    pub fn mark_used_external(&mut self, ix: u32) {
        if ix >= self.external_next {
            self.external_next = ix + 1;
        }
    }

    /// Advance the internal watermark if `ix` is at or past the current edge.
    pub fn mark_used_internal(&mut self, ix: u32) {
        if ix >= self.internal_next {
            self.internal_next = ix + 1;
        }
    }

    // ── private helpers ──────────────────────────────────────────────────────

    /// Derive `GAP_LIMIT` enterprise addresses for the given `role` starting
    /// at `start_ix`.  Indices where derivation fails are silently skipped.
    fn gap_addresses(
        &self,
        account: &Bip32PublicKey,
        network: Network,
        role: u32,
        start_ix: u32,
    ) -> Vec<(u32, quetzal_primitives::Address)> {
        let mut out = Vec::with_capacity(GAP_LIMIT as usize);
        for offset in 0..GAP_LIMIT {
            let ix = start_ix + offset;
            match enterprise_address(account, role, ix, network) {
                Ok(addr) => out.push((ix, addr)),
                Err(_) => {
                    // Soft derivation at this index failed — skip it.
                }
            }
        }
        out
    }
}

impl Default for AddressDiscovery {
    fn default() -> Self {
        Self::new()
    }
}
