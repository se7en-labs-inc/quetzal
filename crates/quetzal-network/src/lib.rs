pub mod chainsync;
pub mod client;
pub mod clock;
pub mod dreps;
pub mod pools;
pub mod statequery;
pub mod txsubmit;

pub use chainsync::ChainSyncEvent;
pub use client::NodeClient;
pub use statequery::StateQuerier;
pub use txsubmit::TxSubmitter;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("network protocol error: {0}")]
    Protocol(String),
    #[error("chainsync error: {0}")]
    ChainSync(String),
    #[error("tx submission failed: {0}")]
    TxSubmit(String),
    #[error("transaction rejected: {0:?}")]
    TxRejected(Vec<u8>),
    #[error("state query failed: {0}")]
    StateQuery(String),
}
