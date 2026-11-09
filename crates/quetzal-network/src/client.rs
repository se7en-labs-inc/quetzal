use pallas_network::facades::NodeClient as PallasNodeClient;

use crate::NetworkError;

/// A connection to a cardano-node Unix socket.
///
/// Wraps the pallas-network `NodeClient` (N2C) and exposes typed handles for
/// the chainsync, local-tx-submission, and local-state-query mini-protocols.
pub struct NodeClient {
    inner: PallasNodeClient,
}

// Pallas spawns its multiplexer before negotiating the handshake. Own it until
// negotiation completes so cancellation cannot detach those background tasks.
struct Connecting(Option<PallasNodeClient>);

impl Drop for Connecting {
    fn drop(&mut self) {
        if let Some(client) = self.0.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(client.abort());
            }
        }
    }
}

impl NodeClient {
    /// Current native header, independent of background replay progress.
    pub async fn chain_header_at_tip(
        socket: &str,
        magic: u64,
    ) -> Result<(u64, Vec<u8>, u64), NetworkError> {
        let connected = Self::connect(socket, magic).await?;
        let mut owned = Connecting(Some(connected.inner));
        let client = owned
            .0
            .as_mut()
            .ok_or_else(|| NetworkError::Protocol("missing query connection".into()))?;
        let result = crate::StateQuerier::new(client.statequery())
            .chain_header()
            .await;
        if let Some(client) = owned.0.take() {
            client.abort().await;
        }
        result
    }

    /// Submit persisted signed bytes with cancellation-safe mux ownership.
    pub async fn submit_at_tip(
        socket: &str,
        magic: u64,
        cbor: Vec<u8>,
    ) -> Result<(), NetworkError> {
        let connected = Self::connect(socket, magic).await?;
        let mut owned = Connecting(Some(connected.inner));
        let client = owned
            .0
            .as_mut()
            .ok_or_else(|| NetworkError::Protocol("missing query connection".into()))?;
        let result = async {
            let parameters = crate::StateQuerier::new(client.statequery())
                .protocol_params()
                .await?;
            crate::TxSubmitter::new(client.submission())
                .submit_era(parameters.era, cbor)
                .await
        }
        .await;
        if let Some(client) = owned.0.take() {
            client.abort().await;
        }
        result
    }
    /// Cancellation-safe registry snapshot query.
    pub async fn dreps_at_tip(
        socket: &str,
        magic: u64,
    ) -> Result<crate::dreps::DRepSnapshot, NetworkError> {
        let connected = Self::connect(socket, magic).await?;
        let mut owned = Connecting(Some(connected.inner));
        let client = owned
            .0
            .as_mut()
            .ok_or_else(|| NetworkError::Protocol("missing query connection".into()))?;
        let result = crate::StateQuerier::new(client.statequery()).dreps().await;
        if let Some(client) = owned.0.take() {
            client.abort().await;
        }
        result
    }
    /// One-shot reward query with cancellation-safe connection ownership.
    /// Callers may put a timeout around this without leaking mux tasks.
    pub async fn stake_accounts_at_tip(
        socket: &str,
        magic: u64,
        credentials: &[(u8, [u8; 28])],
    ) -> Result<Vec<crate::statequery::StakeAccount>, NetworkError> {
        let connected = Self::connect(socket, magic).await?;
        let mut owned = Connecting(Some(connected.inner));
        let client = owned
            .0
            .as_mut()
            .ok_or_else(|| NetworkError::Protocol("missing query connection".into()))?;
        let result = crate::StateQuerier::new(client.statequery())
            .stake_accounts(credentials)
            .await;
        if let Some(client) = owned.0.take() {
            client.abort().await;
        }
        result
    }
    /// Connect to a cardano-node via its Unix domain socket.
    ///
    /// `socket_path` – path to the node socket (e.g. `/run/cardano-node/node.socket`).
    /// `network_magic` – well-known magic number for the target network.
    #[cfg(unix)]
    pub async fn connect(socket_path: &str, network_magic: u64) -> Result<Self, NetworkError> {
        Self::connect_with_timeout(
            socket_path,
            network_magic,
            std::time::Duration::from_secs(10),
        )
        .await
    }

    #[cfg(unix)]
    async fn connect_with_timeout(
        socket_path: &str,
        network_magic: u64,
        timeout: std::time::Duration,
    ) -> Result<Self, NetworkError> {
        use pallas_network::{
            miniprotocols::handshake::{self, Confirmation},
            multiplexer::Bearer,
        };
        let bearer = Bearer::connect_unix(socket_path)
            .await
            .map_err(|e| NetworkError::Protocol(e.to_string()))?;
        let mut connecting = Connecting(Some(PallasNodeClient::new(bearer)));
        let client = connecting
            .0
            .as_mut()
            .ok_or_else(|| NetworkError::Protocol("missing pending connection".into()))?;
        let result = tokio::time::timeout(
            timeout,
            client
                .handshake()
                .handshake(handshake::n2c::VersionTable::v10_and_above(network_magic)),
        )
        .await;
        let error = match result {
            Ok(Ok(Confirmation::Accepted(_, _))) => {
                let inner = connecting.0.take().ok_or_else(|| {
                    NetworkError::Protocol("missing negotiated connection".into())
                })?;
                return Ok(Self { inner });
            }
            Ok(Ok(other)) => format!("node handshake was not accepted: {other:?}"),
            Ok(Err(error)) => error.to_string(),
            Err(_) => "node handshake timed out".into(),
        };
        if let Some(client) = connecting.0.take() {
            client.abort().await;
        }
        Err(NetworkError::Protocol(error))
    }

    /// Obtain a mutable reference to the pallas chainsync client.
    pub fn chainsync(&mut self) -> &mut pallas_network::miniprotocols::chainsync::N2CClient {
        self.inner.chainsync()
    }

    /// Obtain a mutable reference to the pallas local-tx-submission client.
    pub fn submission(&mut self) -> &mut pallas_network::miniprotocols::localtxsubmission::Client {
        self.inner.submission()
    }

    /// Obtain a mutable reference to the pallas local-state-query client.
    pub fn statequery(&mut self) -> &mut pallas_network::miniprotocols::localstate::Client {
        self.inner.statequery()
    }

    /// Gracefully shut down the connection.
    pub async fn abort(self) {
        self.inner.abort().await
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::AsyncReadExt;

    async fn stalled_handshake(cancel: bool) {
        let path = std::env::temp_dir().join(format!(
            "qz-handshake-{}-{cancel}.socket",
            std::process::id()
        ));
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let socket_path = path.to_str().unwrap().to_owned();
        let connecting = tokio::spawn(async move {
            NodeClient::connect_with_timeout(&socket_path, 42, Duration::from_millis(100)).await
        });
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = [0; 1024];
        assert!(
            tokio::time::timeout(Duration::from_secs(2), stream.read(&mut bytes))
                .await
                .unwrap()
                .unwrap()
                > 0
        );
        if cancel {
            connecting.abort();
            assert!(matches!(connecting.await, Err(error) if error.is_cancelled()));
        } else {
            assert!(
                matches!(connecting.await.unwrap(), Err(NetworkError::Protocol(message)) if message == "node handshake timed out")
            );
        }
        let mut remainder = Vec::new();
        tokio::time::timeout(Duration::from_secs(2), stream.read_to_end(&mut remainder))
            .await
            .unwrap()
            .unwrap();
        drop(stream);
        drop(listener);
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test]
    async fn cancelled_handshake_closes_the_socket() {
        stalled_handshake(true).await;
    }

    #[tokio::test]
    async fn stalled_handshake_times_out_and_closes_the_socket() {
        stalled_handshake(false).await;
    }
}
