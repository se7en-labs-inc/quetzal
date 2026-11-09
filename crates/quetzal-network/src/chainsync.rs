use pallas_network::miniprotocols::chainsync::{ClientError, N2CClient, NextResponse};
use pallas_network::miniprotocols::Point;

use quetzal_primitives::Checkpoint;

use crate::NetworkError;

/// Events emitted by the chain-sync driver.
#[derive(Debug, Clone)]
pub enum ChainSyncEvent {
    /// The server has no newer block: the consumer is at its current tip.
    Await,
    /// A new block is available; contains the raw CBOR bytes of the block and
    /// the slot number of the current chain tip.
    RollForward { block_cbor: Vec<u8>, tip_slot: u64 },
    /// A rollback occurred; the chain has returned to this point.
    RollBackward { to_slot: u64, to_hash: [u8; 32] },
}

/// Drives the pallas chainsync state machine, yielding [`ChainSyncEvent`]s.
pub struct ChainSyncDriver<'a> {
    client: &'a mut N2CClient,
}

impl<'a> ChainSyncDriver<'a> {
    /// Create a driver that borrows a mutable reference to a pallas N2C
    /// chainsync client.
    pub fn new(client: &'a mut N2CClient) -> Self {
        Self { client }
    }

    /// Set the intersection point on the node.
    ///
    /// Pass the checkpoints you already know about; the node will pick the
    /// most recent common ancestor.  Use an empty slice to start from genesis.
    pub async fn find_intersect(&mut self, checkpoints: &[Checkpoint]) -> Result<(), NetworkError> {
        let mut points: Vec<Point> = checkpoints
            .iter()
            .map(|c| Point::Specific(c.slot, c.block_hash.to_vec()))
            .collect();

        // Origin is the fallback when every stored checkpoint was rolled back.
        points.push(Point::Origin);

        self.client
            .find_intersect(points)
            .await
            .map_err(|e: ClientError| NetworkError::ChainSync(e.to_string()))?;

        Ok(())
    }

    /// Request the next block from the node and return the corresponding event.
    ///
    /// This method blocks until the node responds.  When the chain is at its
    /// tip, the node sends `AwaitReply`. Expose it so restored consumers can
    /// report readiness without requiring another block to be produced.
    pub async fn next(&mut self) -> Result<ChainSyncEvent, NetworkError> {
        let response = self
            .client
            .request_or_await_next()
            .await
            .map_err(|e: ClientError| NetworkError::ChainSync(e.to_string()))?;

        match response {
            NextResponse::RollForward(block_content, tip) => {
                let tip_slot = tip.0.slot_or_default();
                let block_cbor: Vec<u8> = block_content.into();
                Ok(ChainSyncEvent::RollForward {
                    block_cbor,
                    tip_slot,
                })
            }
            NextResponse::RollBackward(point, _tip) => {
                let (to_slot, to_hash) = match point {
                    Point::Origin => (0u64, [0u8; 32]),
                    Point::Specific(slot, hash_bytes) => {
                        let mut arr = [0u8; 32];
                        let len = hash_bytes.len().min(32);
                        arr[..len].copy_from_slice(&hash_bytes[..len]);
                        (slot, arr)
                    }
                };
                Ok(ChainSyncEvent::RollBackward { to_slot, to_hash })
            }
            NextResponse::Await => Ok(ChainSyncEvent::Await),
        }
    }
}
