use serde::Deserialize;
use std::{net::IpAddr, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub socket_path: String,
    pub db_path: String,
    pub port: u16,
    #[serde(default = "loopback")]
    pub host: IpAddr,
    #[serde(default)]
    pub tls_ca_cert: Option<PathBuf>,
    #[serde(default)]
    pub tls_server_cert: Option<PathBuf>,
    #[serde(default)]
    pub tls_server_key: Option<PathBuf>,
    pub network_magic: u64,
    pub network: Network,
    pub genesis_start: u64,
    pub slot_millis: u64,
    pub epoch_length: u64,
    pub genesis_hash: String,
    pub shelley_genesis: Option<serde_json::Value>,
}

fn loopback() -> IpAddr {
    IpAddr::from([127, 0, 0, 1])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Network {
    Mainnet,
    Testnet,
    Preview,
    Preprod,
}

impl Network {
    pub fn to_pallas(self) -> pallas_addresses::Network {
        match self {
            Network::Mainnet => pallas_addresses::Network::Mainnet,
            _ => pallas_addresses::Network::Testnet,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            socket_path: "/tmp/cardano-node.socket".to_string(),
            db_path: "./quetzal.sled".to_string(),
            port: 8090,
            host: loopback(),
            tls_ca_cert: None,
            tls_server_cert: None,
            tls_server_key: None,
            network_magic: 764824073,
            network: Network::Mainnet,
            genesis_start: 1506203091,
            slot_millis: 1000,
            epoch_length: 432000,
            genesis_hash: "5f20df933584822601f9e3f8c024eb5eb252fe8cefb24d1317dc3d432e940ebb".into(),
            shelley_genesis: None,
        }
    }
}

impl Config {
    /// The pinned wallet's default sync tolerance is five minutes. Public
    /// progress is relative to wall time, independently of worker catch-up.
    pub fn wallet_sync_progress_at(&self, slot: u64, now: std::time::SystemTime) -> Option<f64> {
        let now = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .saturating_sub(u128::from(self.genesis_start) * 1000);
        let covered = if self.network == Network::Mainnet {
            u128::from(slot.min(4_492_800)) * 20_000
                + u128::from(slot.saturating_sub(4_492_800)) * 1000
        } else {
            u128::from(slot) * u128::from(self.slot_millis)
        };
        if now.saturating_sub(covered) <= 300_000 {
            None
        } else {
            Some(100.0 * covered as f64 / now as f64)
        }
    }

    /// Resolve relative validity bounds against wall time, including the
    /// fractional part of the current slot and an explicit rounding direction.
    pub fn time_to_slot(
        &self,
        time: std::time::SystemTime,
        seconds: u64,
        ceiling: bool,
    ) -> Option<u64> {
        let target = time
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos()
            .checked_add(u128::from(seconds) * 1_000_000_000)?;
        let (origin, initial_slot, length) =
            if self.network == Network::Mainnet && target >= 1_596_059_091_000_000_000 {
                (1_596_059_091u64, 4_492_800u64, 1_000_000_000u128)
            } else if self.network == Network::Mainnet {
                (self.genesis_start, 0, 20_000_000_000)
            } else {
                (
                    self.genesis_start,
                    0,
                    u128::from(self.slot_millis) * 1_000_000,
                )
            };
        let elapsed = target.checked_sub(u128::from(origin) * 1_000_000_000)?;
        let floor = elapsed.checked_div(length)?;
        let rounded = floor.checked_add(u128::from(ceiling && !elapsed.is_multiple_of(length)))?;
        initial_slot.checked_add(u64::try_from(rounded).ok()?)
    }

    /// Parse explicit service paths so conformance runs own isolated state.
    pub fn from_args(args: impl IntoIterator<Item = String>) -> anyhow::Result<Self> {
        let mut config = Self::default();
        let mut args = args.into_iter();
        while let Some(flag) = args.next() {
            if flag == "serve" {
                continue;
            }
            let value = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?;
            match flag.as_str() {
                "--database" => config.db_path = value,
                "--node-socket" => config.socket_path = value,
                "--port" => config.port = value.parse()?,
                "--host" => config.host = value.parse()?,
                "--tls-ca-cert" => config.tls_ca_cert = Some(value.into()),
                "--tls-sv-cert" => config.tls_server_cert = Some(value.into()),
                "--tls-sv-key" => config.tls_server_key = Some(value.into()),
                "--testnet" => {
                    let genesis_path = std::path::PathBuf::from(&value);
                    let genesis: serde_json::Value =
                        serde_json::from_slice(&std::fs::read(value)?)?;
                    config.network_magic = genesis
                        .pointer("/protocolConsts/protocolMagic")
                        .and_then(serde_json::Value::as_u64)
                        .ok_or_else(|| {
                            anyhow::anyhow!("Byron genesis lacks protocolConsts.protocolMagic")
                        })?;
                    config.network = Network::Testnet;
                    config.genesis_hash = hex::encode(pallas_crypto::hash::Hasher::<256>::hash(
                        &serde_json::to_vec(&genesis)?,
                    ));
                    config.genesis_start = genesis["startTime"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("Byron genesis lacks startTime"))?;
                    let shelley: serde_json::Value = serde_json::from_slice(&std::fs::read(
                        genesis_path.with_file_name("shelley-genesis.json"),
                    )?)?;
                    let seconds = shelley["slotLength"]
                        .as_f64()
                        .ok_or_else(|| anyhow::anyhow!("Shelley genesis lacks slotLength"))?;
                    anyhow::ensure!(seconds.is_finite() && seconds > 0.0, "invalid slotLength");
                    config.slot_millis = (seconds * 1000.0).round() as u64;
                    anyhow::ensure!(
                        config.slot_millis > 0,
                        "slotLength must be at least one millisecond"
                    );
                    config.epoch_length = shelley["epochLength"]
                        .as_u64()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow::anyhow!("invalid epochLength"))?;
                    config.shelley_genesis = Some(shelley);
                }
                _ => anyhow::bail!("unknown option {flag}"),
            }
        }
        config.validate_transport()?;
        Ok(config)
    }

    pub fn validate_transport(&self) -> anyhow::Result<()> {
        let supplied = [
            self.tls_ca_cert.is_some(),
            self.tls_server_cert.is_some(),
            self.tls_server_key.is_some(),
        ];
        anyhow::ensure!(
            supplied.iter().all(|present| *present) || supplied.iter().all(|present| !present),
            "TLS requires --tls-ca-cert, --tls-sv-cert and --tls-sv-key together"
        );
        anyhow::ensure!(
            self.tls_ca_cert.is_some() || self.host.is_loopback(),
            "unauthenticated HTTP requires a loopback --host; configure mutual TLS for remote access"
        );
        Ok(())
    }

    pub fn slot_reference(&self, slot: u64) -> (u64, u64, String) {
        let (epoch, local_slot, start, elapsed) = if self.network == Network::Mainnet {
            if slot < 4492800 {
                (
                    slot / 21600,
                    slot % 21600,
                    1506203091000u64,
                    slot.saturating_mul(20000),
                )
            } else {
                let offset = slot - 4492800;
                (
                    208 + offset / 432000,
                    offset % 432000,
                    1596059091000u64,
                    offset.saturating_mul(1000),
                )
            }
        } else {
            (
                slot / self.epoch_length,
                slot % self.epoch_length,
                self.genesis_start.saturating_mul(1000),
                slot.saturating_mul(self.slot_millis),
            )
        };
        let millis = start.saturating_add(elapsed);
        let time = i64::try_from(millis)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|time| time.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true))
            .unwrap_or_default();
        (epoch, local_slot, time)
    }

    pub fn epoch_first_slot(&self, epoch: u64) -> Option<u64> {
        if self.network == Network::Mainnet {
            if epoch < 208 {
                epoch.checked_mul(21600)
            } else {
                (epoch - 208).checked_mul(432000)?.checked_add(4492800)
            }
        } else {
            epoch.checked_mul(self.epoch_length)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wallet_progress_uses_wall_time_and_the_reference_tolerance_boundary() {
        let config = Config {
            network: Network::Testnet,
            genesis_start: 1000,
            slot_millis: 200,
            ..Config::default()
        };
        let now = |millis: u64| {
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_000_000 + millis)
        };
        assert_eq!(config.wallet_sync_progress_at(0, now(300_000)), None);
        assert_eq!(config.wallet_sync_progress_at(0, now(300_001)), Some(0.0));
        assert_eq!(config.wallet_sync_progress_at(1000, now(500_000)), None);
        assert_eq!(
            config.wallet_sync_progress_at(1000, now(600_000)),
            Some(100.0 / 3.0)
        );
        assert_eq!(config.wallet_sync_progress_at(1000, now(100_000)), None);
    }

    #[test]
    fn wallet_progress_accounts_for_mainnet_byron_slot_lengths() {
        let config = Config::default();
        let now =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(config.genesis_start + 600);
        assert_eq!(config.wallet_sync_progress_at(10, now), Some(100.0 / 3.0));
        let fork = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_596_059_091 + 300);
        assert_eq!(config.wallet_sync_progress_at(4_492_800, fork), None);
        assert!(config
            .wallet_sync_progress_at(4_492_800, fork + std::time::Duration::from_millis(1))
            .is_some());
    }

    #[test]
    fn isolated_service_paths_and_port() {
        let args = [
            "serve",
            "--database",
            "/tmp/owned-wallet",
            "--node-socket",
            "/tmp/owned-node.socket",
            "--port",
            "18792",
        ];
        let config = Config::from_args(args.map(str::to_owned)).unwrap();
        assert_eq!(config.db_path, "/tmp/owned-wallet");
        assert_eq!(config.socket_path, "/tmp/owned-node.socket");
        assert_eq!(config.port, 18792);
    }

    #[test]
    fn invalid_options_are_rejected() {
        for args in [
            vec!["--database"],
            vec!["--port", "65536"],
            vec!["--unknown", "x"],
        ] {
            assert!(Config::from_args(args.into_iter().map(str::to_owned)).is_err());
        }
    }

    #[test]
    fn transport_configuration_is_complete_and_http_is_loopback_only() {
        for mask in 1..7 {
            let mut config = Config::default();
            if mask & 1 != 0 {
                config.tls_ca_cert = Some("ca.pem".into());
            }
            if mask & 2 != 0 {
                config.tls_server_cert = Some("server.pem".into());
            }
            if mask & 4 != 0 {
                config.tls_server_key = Some("server.key".into());
            }
            assert!(config.validate_transport().is_err());
        }
        let parse = |args: &[&str]| Config::from_args(args.iter().map(|arg| (*arg).to_owned()));
        assert!(parse(&["--host", "0.0.0.0"]).is_err());
        assert!(parse(&["--host", "::"]).is_err());
        assert!(parse(&["--host", "::1"]).is_ok());
        assert_eq!(Config::default().host.to_string(), "127.0.0.1");
        assert!(parse(&[
            "--host",
            "0.0.0.0",
            "--tls-ca-cert",
            "ca.pem",
            "--tls-sv-cert",
            "server.pem",
            "--tls-sv-key",
            "server.key"
        ])
        .is_ok());
    }

    #[test]
    fn mainnet_slot_clock_crosses_shelley_boundary() {
        let config = Config::default();
        assert_eq!(
            config.slot_reference(4492799),
            (207, 21599, "2020-07-29T21:44:31Z".into())
        );
        assert_eq!(
            config.slot_reference(4492800),
            (208, 0, "2020-07-29T21:44:51Z".into())
        );
        assert_eq!(
            config.slot_reference(4924800),
            (209, 0, "2020-08-03T21:44:51Z".into())
        );
    }

    #[test]
    fn fractional_slot_rounding_is_exact_on_testnet_and_mainnet() {
        use std::time::{Duration, UNIX_EPOCH};
        let config = Config {
            network: Network::Testnet,
            genesis_start: 100,
            slot_millis: 200,
            ..Default::default()
        };
        let time = UNIX_EPOCH + Duration::from_millis(100_250);
        assert_eq!(config.time_to_slot(time, 0, false), Some(1));
        assert_eq!(config.time_to_slot(time, 0, true), Some(2));
        assert_eq!(config.time_to_slot(time, 90, true), Some(452));
        assert_eq!(
            config.time_to_slot(UNIX_EPOCH + Duration::from_millis(100_200), 0, true),
            Some(1)
        );
        let mainnet = Config::default();
        assert_eq!(
            mainnet.time_to_slot(
                UNIX_EPOCH + Duration::from_millis(1_596_059_091_001),
                0,
                false
            ),
            Some(4_492_800)
        );
        assert_eq!(
            mainnet.time_to_slot(
                UNIX_EPOCH + Duration::from_millis(1_596_059_091_001),
                0,
                true
            ),
            Some(4_492_801)
        );
    }
}
