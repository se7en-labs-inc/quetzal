use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// CIP-0119 off-chain metadata for a DRep.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DRepReference {
    pub label: String,
    pub uri: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DRepMetadata {
    pub name: Option<String>,
    pub objectives: Option<String>,
    pub motivations: Option<String>,
    pub qualifications: Option<String>,
    pub payment_address: Option<String>,
    pub references: Vec<DRepReference>,
    /// When true the DRep has asked not to be suggested to users.
    pub do_not_list: bool,
}

/// A single DRep registration fetched from the ledger.
#[derive(Debug, Clone)]
pub struct DRepRegistration {
    /// Bech32-encoded DRep id: `drep1...` (key) or `drep_script1...` (script).
    pub id: String,
    pub kind: u8,
    pub hash: [u8; 28],
    pub deposit: u64,
    pub is_active: bool,
    /// The epoch in which this DRep registration expires.
    pub expiry_epoch: u64,
    /// Total delegated voting power in lovelace (from GetDRepStakeDistr).
    pub voting_power: u64,
    /// Anchor URL from the registration certificate (CIP-0119).
    pub anchor_url: Option<String>,
    /// Hex-encoded blake2b-256 hash of the anchor document.
    pub anchor_data_hash: Option<String>,
    /// Off-chain metadata fetched from `anchor_url` and hash-verified.
    pub metadata: Option<DRepMetadata>,
}

/// Registry snapshots have the reference's 900-second TTL. Verified anchor
/// metadata remains reusable while its registered hash matches.
pub struct DRepLayer {
    cache: RwLock<Option<(Instant, Vec<DRepRegistration>)>>,
}

impl DRepLayer {
    /// Create a new, empty `DRepLayer` wrapped in an `Arc`.
    pub fn new() -> Arc<Self> {
        Arc::new(DRepLayer {
            cache: RwLock::new(None),
        })
    }

    /// Read the most recently observed registry and verified metadata.
    pub async fn list_dreps(&self) -> Vec<DRepRegistration> {
        self.cache
            .read()
            .await
            .as_ref()
            .map(|(_, rows)| rows.clone())
            .unwrap_or_default()
    }

    pub async fn cached_dreps(&self) -> Option<Vec<DRepRegistration>> {
        self.cache
            .read()
            .await
            .as_ref()
            .filter(|(time, _)| time.elapsed() < Duration::from_secs(900))
            .map(|(_, rows)| rows.clone())
    }

    /// Replace the cache with a freshly fetched set of registrations.
    pub async fn update(&self, dreps: Vec<DRepRegistration>) {
        let mut guard = self.cache.write().await;
        *guard = Some((Instant::now(), dreps));
    }

    /// Enrich the current snapshot without extending its ledger cache lifetime
    /// or replacing newer registrations that arrived during an HTTP fetch.
    pub async fn update_metadata(&self, fetched: &[DRepRegistration]) {
        if let Some((_, rows)) = self.cache.write().await.as_mut() {
            for row in rows {
                if let Some(hash) = &row.anchor_data_hash {
                    if let Some(metadata) = fetched
                        .iter()
                        .find(|entry| entry.anchor_data_hash.as_ref() == Some(hash))
                        .and_then(|entry| entry.metadata.clone())
                    {
                        row.metadata = Some(metadata);
                    }
                }
            }
        }
    }
}
