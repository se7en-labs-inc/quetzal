use pallas_network::miniprotocols::localtxsubmission::{Client, EraTx, Response};

use crate::NetworkError;

/// The node's Conway mempool guard has a distinct ledger constructor (tag 7).
/// Keep its actual message for the API and for transient-conflict scheduling.
pub fn mempool_failure_message(raw: &[u8]) -> Option<&str> {
    let mut decoder = pallas_codec::minicbor::Decoder::new(raw);
    if decoder.array().ok()? != Some(2)
        || decoder.u8().ok()? != 2
        || decoder.array().ok()? != Some(1)
        || decoder.array().ok()? != Some(2)
        || decoder.u16().ok()? != 6
        || decoder.array().ok()? != Some(1)
        || decoder.array().ok()? != Some(2)
        || decoder.u8().ok()? != 7
    {
        return None;
    }
    let message = decoder.str().ok()?;
    (decoder.position() == raw.len()).then_some(message)
}

pub fn retryable_rejection(raw: &[u8]) -> bool {
    mempool_failure_message(raw).is_some_and(|message| {
        message.starts_with("All inputs are spent. Transaction has probably already been included")
    })
}

/// Submits signed transactions to the connected cardano-node via the
/// local-tx-submission mini-protocol.
pub struct TxSubmitter<'a> {
    client: &'a mut Client,
}

impl<'a> TxSubmitter<'a> {
    /// Create a submitter that borrows a mutable reference to a pallas
    /// local-tx-submission client.
    pub fn new(client: &'a mut Client) -> Self {
        Self { client }
    }

    /// Submit a raw, CBOR-encoded transaction.
    ///
    /// The `era` parameter identifies the Cardano era the transaction belongs
    /// to (e.g. `6` for Babbage, `7` for Conway).  When in doubt, pass the
    /// current era number returned by a state query.
    ///
    /// Returns `Ok(())` if the node accepted the transaction, or an error
    /// containing the node's raw rejection reason.
    pub async fn submit_era(&mut self, era: u16, tx_cbor: Vec<u8>) -> Result<(), NetworkError> {
        let era_tx = EraTx(era, tx_cbor);

        let response = self
            .client
            .submit_tx(era_tx)
            .await
            .map_err(|e| NetworkError::TxSubmit(e.to_string()))?;

        match response {
            Response::Accepted => Ok(()),
            Response::Rejected(reason) => Err(NetworkError::TxRejected(reason.0)),
        }
    }

    /// Submit a raw CBOR-encoded transaction, defaulting to Conway era (7).
    pub async fn submit(&mut self, tx_cbor: Vec<u8>) -> Result<(), NetworkError> {
        // Default to Conway (era 7).  Callers that need a specific era should
        // use `submit_era` directly.
        self.submit_era(7, tx_cbor).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retry_admission_requires_the_complete_typed_node_mempool_guard() {
        let message = "All inputs are spent. Transaction has probably already been included in the blockchain.";
        let raw = pallas_codec::minicbor::to_vec((2u8, vec![(6u8, vec![(7u8, message)])])).unwrap();
        assert_eq!(mempool_failure_message(&raw), Some(message));
        assert!(retryable_rejection(&raw));
        for length in 0..raw.len() {
            assert!(!retryable_rejection(&raw[..length]));
        }
        let mut trailing = raw;
        trailing.push(0);
        assert!(!retryable_rejection(&trailing));
        let wrong_constructor =
            pallas_codec::minicbor::to_vec((2u8, vec![(6u8, vec![(1u8, message)])])).unwrap();
        assert!(!retryable_rejection(&wrong_constructor));
        let other_failure = pallas_codec::minicbor::to_vec((
            2u8,
            vec![(6u8, vec![(7u8, "Unrelated mempool failure")])],
        ))
        .unwrap();
        assert!(!retryable_rejection(&other_failure));
    }
}
