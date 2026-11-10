use serde::Deserialize;
use std::{net::IpAddr, path::PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub socket_path: String,
    pub db_path: String,
    pub port: u16,
    #[serde(default)]
    pub ui_port: Option<u16>,
    #[serde(default)]
    pub temporary_database: bool,
    #[serde(default)]
    pub listen_address: Option<String>,
    #[serde(default = "default_tolerance")]
    pub sync_tolerance_seconds: u64,
    #[serde(default, deserialize_with = "deserialize_metadata_server")]
    pub token_metadata_server: Option<reqwest::Url>,
    #[serde(default)]
    pub pool_metadata_source: Option<String>,
    #[serde(default = "default_gateway")]
    pub ipfs_gateway_url: String,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default)]
    pub tracers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub shutdown_handler: bool,
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
    #[serde(skip)]
    pub native_eras: std::sync::Arc<std::sync::RwLock<Vec<quetzal_network::statequery::EraTiming>>>,
    #[serde(skip)]
    pub native_genesis:
        std::sync::Arc<std::sync::RwLock<Option<quetzal_network::statequery::NativeGenesis>>>,
}

fn loopback() -> IpAddr {
    IpAddr::from([127, 0, 0, 1])
}

fn default_tolerance() -> u64 {
    300
}
fn default_gateway() -> String {
    "https://ipfs.blockfrost.dev/ipfs/".into()
}
fn default_log_level() -> String {
    "debug".into()
}
fn deserialize_metadata_server<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Option<reqwest::Url>, D::Error> {
    Option::<String>::deserialize(d)?
        .map(|s| crate::cli::url(&s).map_err(serde::de::Error::custom))
        .transpose()
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
            ui_port: None,
            temporary_database: false,
            listen_address: None,
            sync_tolerance_seconds: default_tolerance(),
            token_metadata_server: None,
            pool_metadata_source: None,
            ipfs_gateway_url: default_gateway(),
            log_level: default_log_level(),
            tracers: Default::default(),
            shutdown_handler: false,
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
            native_eras: Default::default(),
            native_genesis: Default::default(),
        }
    }
}

impl Config {
    pub fn set_native_genesis(&self, genesis: Option<quetzal_network::statequery::NativeGenesis>) {
        match self.native_genesis.write() {
            Ok(mut guard) => *guard = genesis,
            Err(poisoned) => *poisoned.into_inner() = genesis,
        }
    }
    pub fn active_slot_coefficient(&self) -> Option<f64> {
        self.native_genesis
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|g| g.active_slot_coefficient))
            .or_else(|| {
                self.shelley_genesis
                    .as_ref()
                    .and_then(|g| g["activeSlotsCoeff"].as_f64())
            })
            .or_else(|| (self.network == Network::Mainnet).then_some(0.05))
    }
    pub fn security_parameter(&self) -> Option<u64> {
        self.native_genesis
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|g| g.security_parameter))
            .or_else(|| {
                self.shelley_genesis
                    .as_ref()
                    .and_then(|g| g["securityParam"].as_u64())
            })
            .or_else(|| (self.network == Network::Mainnet).then_some(2160))
    }
    pub fn set_native_eras(&self, eras: Vec<quetzal_network::statequery::EraTiming>) {
        match self.native_eras.write() {
            Ok(mut guard) => *guard = eras,
            Err(poisoned) => *poisoned.into_inner() = eras,
        }
    }
    fn era_at_slot(&self, slot: u64) -> Option<quetzal_network::statequery::EraTiming> {
        self.native_eras
            .read()
            .ok()?
            .iter()
            .rev()
            .find(|era| era.start_slot <= slot)
            .cloned()
    }
    pub fn current_slot_millis(&self) -> u64 {
        self.native_eras
            .read()
            .ok()
            .and_then(|eras| eras.last().map(|era| era.slot_millis))
            .unwrap_or(self.slot_millis)
    }
    pub fn current_epoch_length(&self) -> u64 {
        self.native_eras
            .read()
            .ok()
            .and_then(|eras| eras.last().map(|era| era.epoch_length))
            .unwrap_or(self.epoch_length)
    }
    /// The pinned wallet's default sync tolerance is five minutes. Public
    /// progress is relative to wall time, independently of worker catch-up.
    pub fn wallet_sync_progress_at(&self, slot: u64, now: std::time::SystemTime) -> Option<f64> {
        let (covered, now) = self.sync_progress_times_at(slot, now)?;
        let scaled = covered * 10000;
        let floor = scaled / now;
        let remainder = scaled % now;
        let round_up = remainder * 2 > now || remainder * 2 == now && !floor.is_multiple_of(2);
        Some((floor + u128::from(round_up)) as f64 / 100.0)
    }

    fn sync_progress_times_at(
        &self,
        slot: u64,
        now: std::time::SystemTime,
    ) -> Option<(u128, u128)> {
        let elapsed_nanos = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .saturating_sub(u128::from(self.genesis_start) * 1_000_000_000);
        let millis = elapsed_nanos / 1_000_000;
        let fraction = elapsed_nanos % 1_000_000;
        let now = (millis
            + u128::from(fraction > 500_000 || fraction == 500_000 && !millis.is_multiple_of(2)))
        .max(1);
        let covered = if let Some(era) = self.era_at_slot(slot) {
            u128::from(era.relative_millis)
                + u128::from(slot - era.start_slot) * u128::from(era.slot_millis)
        } else if self.network == Network::Mainnet {
            u128::from(slot.min(4_492_800)) * 20_000
                + u128::from(slot.saturating_sub(4_492_800)) * 1000
        } else {
            u128::from(slot) * u128::from(self.slot_millis)
        };
        if elapsed_nanos.saturating_sub(covered * 1_000_000)
            <= u128::from(self.sync_tolerance_seconds) * 1_000_000_000
        {
            None
        } else {
            Some((covered, now))
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
        let relative = target.checked_sub(u128::from(self.genesis_start) * 1_000_000_000)?;
        if let Some(era) = self
            .native_eras
            .read()
            .ok()?
            .iter()
            .rev()
            .find(|era| u128::from(era.relative_millis) * 1_000_000 <= relative)
        {
            let elapsed = relative.checked_sub(u128::from(era.relative_millis) * 1_000_000)?;
            let length = u128::from(era.slot_millis) * 1_000_000;
            return era.start_slot.checked_add(
                u64::try_from(
                    elapsed / length + u128::from(ceiling && !elapsed.is_multiple_of(length)),
                )
                .ok()?,
            );
        }
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
        crate::cli::parse(args)
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
        let (epoch, local_slot, start, elapsed) = if let Some(era) = self.era_at_slot(slot) {
            let delta = slot - era.start_slot;
            (
                era.start_epoch + delta / era.epoch_length,
                delta % era.epoch_length,
                self.genesis_start.saturating_mul(1000),
                era.relative_millis
                    .saturating_add(delta.saturating_mul(era.slot_millis)),
            )
        } else if self.network == Network::Mainnet {
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
        if let Some(era) = self
            .native_eras
            .read()
            .ok()?
            .iter()
            .rev()
            .find(|era| era.start_epoch <= epoch)
        {
            return (epoch - era.start_epoch)
                .checked_mul(era.epoch_length)?
                .checked_add(era.start_slot);
        }
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
    fn native_clock_is_shared_across_config_clones_and_respects_era_boundaries() {
        use quetzal_network::statequery::EraTiming;
        let config = Config::default();
        let cloned = config.clone();
        config.set_native_eras(vec![
            EraTiming {
                start_slot: 0,
                start_epoch: 0,
                relative_millis: 0,
                slot_millis: 20000,
                epoch_length: 21600,
            },
            EraTiming {
                start_slot: 4_492_800,
                start_epoch: 208,
                relative_millis: 89_856_000_000,
                slot_millis: 1000,
                epoch_length: 432000,
            },
        ]);
        assert_eq!(
            cloned.slot_reference(4_492_801),
            (208, 1, "2020-07-29T21:44:52Z".into())
        );
        assert_eq!(cloned.epoch_first_slot(209), Some(4_924_800));
        assert_eq!(
            cloned.time_to_slot(
                std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_596_059_092),
                0,
                false
            ),
            Some(4_492_801)
        );
    }
    #[test]
    fn configurable_tolerance_and_reference_percentage_rounding_are_independent_of_replay() {
        let mut config = Config {
            network: Network::Testnet,
            genesis_start: 1000,
            slot_millis: 1,
            sync_tolerance_seconds: 0,
            ..Default::default()
        };
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_020_000);
        assert_eq!(config.wallet_sync_progress_at(6667, now), Some(33.34));
        assert_eq!(config.wallet_sync_progress_at(6665, now), Some(33.32));
        config.sync_tolerance_seconds = 14;
        assert_eq!(config.wallet_sync_progress_at(6665, now), None);
        config.sync_tolerance_seconds = 0;
        assert_eq!(config.wallet_sync_progress_at(20000, now), None);
    }

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
            Some(33.33)
        );
        assert_eq!(config.wallet_sync_progress_at(1000, now(100_000)), None);
    }

    #[test]
    fn wallet_progress_accounts_for_mainnet_byron_slot_lengths() {
        let config = Config::default();
        let now =
            std::time::UNIX_EPOCH + std::time::Duration::from_secs(config.genesis_start + 600);
        assert_eq!(config.wallet_sync_progress_at(10, now), Some(33.33));
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
            "--mainnet",
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
    fn shutdown_handler_is_a_valueless_opt_in_flag() {
        assert!(!Config::default().shutdown_handler);
        for arguments in [
            vec!["serve", "--shutdown-handler", "--port", "8091"],
            vec!["serve", "--port", "8091", "--shutdown-handler"],
        ] {
            let config = Config::from_args(
                arguments
                    .into_iter()
                    .chain(["--mainnet", "--node-socket", "/tmp/owned.socket"])
                    .map(str::to_owned),
            )
            .unwrap();
            assert!(config.shutdown_handler);
            assert_eq!(config.port, 8091);
        }
        assert!(
            Config::from_args(
                [
                    "--shutdown-handler",
                    "--mainnet",
                    "--node-socket",
                    "/tmp/owned.socket"
                ]
                .map(str::to_owned)
            )
            .unwrap()
            .shutdown_handler
        );
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
        let parse = |args: &[&str]| {
            Config::from_args(
                ["--mainnet", "--node-socket", "/tmp/owned.socket"]
                    .into_iter()
                    .chain(args.iter().copied())
                    .map(str::to_owned),
            )
        };
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
