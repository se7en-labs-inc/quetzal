//! Public server options from cardano-wallet2fc5c383; no ignored compatibility flags.
use crate::config::{Config, Network};
use anyhow::{bail, ensure, Context, Result};
use std::collections::BTreeSet;

pub const HELP: &str = "Quetzal — cardano-wallet compatible service
Usage: quetzal serve --node-socket FILE (--mainnet | --testnet FILE) [OPTIONS]

  -h,--help                       Show this help
  --help-tracing                  Show logging controls
  --listen-address HOST           IP, hostname or '*' (default: 127.0.0.1)
  --node-socket FILE              Node-to-client socket
  --sync-tolerance DURATION       Whole seconds followed by 's' (default: 300s)
  --random-port                  Bind an available API port (conflicts with --port)
  --port INT                     API port, 1024..65535 (default: 8090)
  --ui-random-port               Bind an available UI port (conflicts with --ui-port)
  --ui-port INT                  Personal wallet UI port, 1024..65535
  --tls-ca-cert FILE             Client certificate authority
  --tls-sv-cert FILE             Server certificate chain
  --tls-sv-key FILE              Server private key
  --mainnet                      Select mainnet
  --testnet FILE                 Byron genesis JSON; no sibling file required
  --database DIR                 Persistent wallet directory; otherwise temporary
  --shutdown-handler             Clean shutdown on stdin EOF
  --pool-metadata-fetching SOURCE none, direct, or SMASH URL; persists across restart
  --token-metadata-server URL     Token registry; otherwise disabled
  --ipfs-gateway-url URL          HTTPS gateway ending in '/'
  --log-level SEVERITY            Global log threshold (default: debug)
  --trace-NAME SEVERITY           Component threshold or off (default: info)

Quetzal alias: --host IP. Non-loopback listeners require mutual TLS.
";
pub const TRACING_HELP: &str = "Tracing severities: debug info notice warning error critical alert emergency
--log-level SEVERITY caps all components (default: debug).
--trace-NAME SEVERITY sets the component threshold, or off (default: info).
Tracers: application api-server wallet-engine wallet-db token-metadata pools-engine pools-db ntp-client network
";
const TRACERS: &[&str] = &[
    "application",
    "api-server",
    "wallet-engine",
    "wallet-db",
    "token-metadata",
    "pools-engine",
    "pools-db",
    "ntp-client",
    "network",
];

pub(crate) fn url(value: &str) -> Result<reqwest::Url> {
    let url = reqwest::Url::parse(value).context("invalid absolute URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "URL must use HTTP or HTTPS with a host"
    );
    Ok(url)
}
fn port(value: &str) -> Result<u16> {
    ensure!(
        !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()),
        "invalid port"
    );
    let value: u16 = value.parse()?;
    ensure!(
        value >= 1024,
        "expected a TCP port number between 1024 and 65535"
    );
    Ok(value)
}
fn severity(value: &str, off: bool) -> Result<String> {
    let value = value.to_ascii_lowercase();
    ensure!(
        [
            "debug",
            "info",
            "notice",
            "warning",
            "error",
            "critical",
            "alert",
            "emergency"
        ]
        .contains(&value.as_str())
            || off && value == "off",
        "invalid log severity"
    );
    Ok(value)
}
pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Config> {
    let mut config = Config {
        temporary_database: true,
        ..Default::default()
    };
    let mut seen = BTreeSet::new();
    let mut args = args.into_iter().peekable();
    if args.peek().is_some_and(|s| s == "serve") {
        args.next();
    }
    while let Some(argument) = args.next() {
        let (flag, attached) = argument
            .split_once('=')
            .map_or((argument.as_str(), None), |(f, v)| (f, Some(v.to_owned())));
        ensure!(
            seen.insert(flag.to_owned()),
            "option {flag} supplied more than once"
        );
        if [
            "--shutdown-handler",
            "--mainnet",
            "--random-port",
            "--ui-random-port",
        ]
        .contains(&flag)
        {
            ensure!(attached.is_none(), "{flag} does not take a value");
            match flag {
                "--shutdown-handler" => config.shutdown_handler = true,
                "--random-port" => config.port = 0,
                "--ui-random-port" => config.ui_port = Some(0),
                _ => {}
            }
            continue;
        }
        let value = attached
            .or_else(|| args.next().filter(|value| !value.starts_with("--")))
            .with_context(|| format!("missing value for {flag}"))?;
        match flag {
            "--database" => {
                ensure!(!value.is_empty(), "empty database path");
                config.db_path = value;
                config.temporary_database = false;
            }
            "--node-socket" => {
                ensure!(!value.is_empty(), "empty socket path");
                config.socket_path = value;
            }
            "--port" => config.port = port(&value)?,
            "--ui-port" => config.ui_port = Some(port(&value)?),
            "--host" => config.host = value.parse()?,
            "--listen-address" => {
                ensure!(!value.is_empty(), "empty listen address");
                if let Ok(ip) = value.parse() {
                    config.host = ip;
                }
                config.listen_address = Some(value);
            }
            "--sync-tolerance" => {
                let seconds = value
                    .strip_suffix('s')
                    .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
                    .context("sync tolerance must be whole seconds followed by 's'")?;
                config.sync_tolerance_seconds = seconds.parse()?;
            }
            "--tls-ca-cert" => config.tls_ca_cert = Some(value.into()),
            "--tls-sv-cert" => config.tls_server_cert = Some(value.into()),
            "--tls-sv-key" => config.tls_server_key = Some(value.into()),
            "--testnet" => {
                let genesis: serde_json::Value = serde_json::from_slice(&std::fs::read(value)?)?;
                config.network = Network::Testnet;
                config.network_magic = genesis
                    .pointer("/protocolConsts/protocolMagic")
                    .and_then(serde_json::Value::as_u64)
                    .context("Byron genesis lacks protocolConsts.protocolMagic")?;
                config.genesis_start = genesis["startTime"]
                    .as_u64()
                    .context("Byron genesis lacks startTime")?;
                config.genesis_hash = hex::encode(pallas_crypto::hash::Hasher::<256>::hash(
                    &serde_json::to_vec(&genesis)?,
                ));
            }
            "--token-metadata-server" => config.token_metadata_server = Some(url(&value)?),
            "--pool-metadata-fetching" => {
                if value != "none" && value != "direct" {
                    url(&value)?;
                }
                config.pool_metadata_source = Some(value);
            }
            "--ipfs-gateway-url" => {
                let parsed = url(&value)?;
                ensure!(
                    parsed.scheme() == "https"
                        && value.ends_with('/')
                        && parsed.query().is_none()
                        && parsed.fragment().is_none(),
                    "IPFS gateway must use HTTPS and end with '/'"
                );
                config.ipfs_gateway_url = value;
            }
            "--log-level" => config.log_level = severity(&value, false)?,
            _ if flag.starts_with("--trace-") => {
                let tracer = &flag[8..];
                ensure!(TRACERS.contains(&tracer), "unknown tracer {tracer}");
                config
                    .tracers
                    .insert(tracer.into(), severity(&value, true)?);
            }
            _ => bail!("unknown option {flag}"),
        }
    }
    ensure!(seen.contains("--node-socket"), "--node-socket is required");
    ensure!(
        seen.contains("--mainnet") != seen.contains("--testnet"),
        "exactly one of --mainnet or --testnet is required"
    );
    ensure!(
        !(seen.contains("--random-port") && seen.contains("--port")),
        "--random-port conflicts with --port"
    );
    ensure!(
        !(seen.contains("--ui-random-port") && seen.contains("--ui-port")),
        "--ui-random-port conflicts with --ui-port"
    );
    ensure!(
        !(seen.contains("--host") && seen.contains("--listen-address")),
        "--host conflicts with --listen-address"
    );
    config.validate_transport()?;
    Ok(config)
}

pub fn logging_filter(config: &Config) -> String {
    fn level(s: &str) -> usize {
        match s {
            "debug" => 0,
            "info" => 1,
            "notice" => 2,
            "warning" => 3,
            "error" => 4,
            "critical" => 5,
            "alert" => 6,
            "emergency" => 7,
            _ => 8,
        }
    }
    fn rust_level(level: usize) -> &'static str {
        match level {
            0 => "debug",
            1 | 2 => "info",
            3 => "warn",
            4..=7 => "error",
            _ => "off",
        }
    }
    let targets = [
        ("application", vec!["quetzal"]),
        ("api-server", vec!["tower_http", "quetzal::handlers"]),
        ("wallet-engine", vec!["quetzal::workers", "quetzal_wallet"]),
        ("wallet-db", vec!["quetzal_wallet::db", "sled"]),
        ("token-metadata", vec!["quetzal::metadata"]),
        ("pools-engine", vec!["quetzal::handlers::pool_catalogue"]),
        ("pools-db", vec!["quetzal_wallet::db::pool_chain"]),
        ("ntp-client", vec!["quetzal_network::clock"]),
        ("network", vec!["quetzal_network", "quetzal::follower"]),
    ];
    let mut directives = vec!["off".to_owned()];
    for (name, targets) in targets {
        let threshold = level(
            config
                .tracers
                .get(name)
                .map(String::as_str)
                .unwrap_or("info"),
        )
        .max(level(&config.log_level));
        directives.extend(
            targets
                .into_iter()
                .map(|target| format!("{target}={}", rust_level(threshold))),
        );
    }
    directives.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn arguments(extra: &[&str]) -> Vec<String> {
        ["serve", "--mainnet", "--node-socket", "/tmp/owned.socket"]
            .into_iter()
            .chain(extra.iter().copied())
            .map(str::to_owned)
            .collect()
    }
    #[test]
    fn complete_options_have_real_configuration_and_defaults() {
        let default = parse(arguments(&[])).unwrap();
        assert!(default.temporary_database);
        assert_eq!(default.sync_tolerance_seconds, 300);
        assert!(default.token_metadata_server.is_none());
        assert!(default.ui_port.is_none());
        let configured = parse(arguments(&[
            "--listen-address=::1",
            "--database",
            "/tmp/owned.db",
            "--sync-tolerance=7s",
            "--token-metadata-server",
            "https://metadata.example/base/",
            "--pool-metadata-fetching",
            "direct",
            "--ipfs-gateway-url",
            "https://ipfs.example/custom/",
            "--random-port",
            "--ui-random-port",
            "--shutdown-handler",
            "--log-level",
            "WARNING",
            "--trace-network",
            "off",
        ]))
        .unwrap();
        assert_eq!(configured.host.to_string(), "::1");
        assert_eq!(configured.port, 0);
        assert_eq!(configured.ui_port, Some(0));
        assert!(!configured.temporary_database);
        assert_eq!(configured.sync_tolerance_seconds, 7);
        assert_eq!(configured.pool_metadata_source.as_deref(), Some("direct"));
        assert_eq!(
            configured.token_metadata_server.unwrap().as_str(),
            "https://metadata.example/base/"
        );
        assert_eq!(configured.ipfs_gateway_url, "https://ipfs.example/custom/");
        assert_eq!(configured.log_level, "warning");
        assert_eq!(configured.tracers["network"], "off");
    }
    #[test]
    fn malformed_and_conflicting_options_fail_with_otherwise_valid_launch() {
        for extra in [
            vec!["--sync-tolerance", "1.5s"],
            vec!["--sync-tolerance", "-1s"],
            vec!["--sync-tolerance", "20"],
            vec!["--token-metadata-server", "://invalid"],
            vec!["--token-metadata-server", "file:///tmp/metadata"],
            vec!["--ipfs-gateway-url", "http://insecure/"],
            vec!["--ipfs-gateway-url", "https://gateway/no-slash"],
            vec!["--port", "0"],
            vec!["--port", "1023"],
            vec!["--ui-port", "65536"],
            vec!["--random-port", "--port", "8090"],
            vec!["--ui-random-port", "--ui-port", "8091"],
            vec!["--database", "--unknown"],
            vec!["--unknown", "ignored"],
            vec!["--mainnet=true"],
            vec!["--trace-invented", "info"],
            vec!["--trace-network", "invented"],
            vec!["--log-level", "off"],
        ] {
            assert!(parse(arguments(&extra)).is_err(), "accepted {extra:?}");
        }
        assert!(parse(Vec::new()).is_err());
        assert!(parse(["serve", "--mainnet"].map(str::to_owned)).is_err());
        assert!(parse(["serve", "--node-socket", "/tmp/owned.socket"].map(str::to_owned)).is_err());
    }
    #[test]
    fn byron_genesis_alone_is_accepted_and_conflicting_networks_are_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let genesis = directory.path().join("byron-any-filename.json");
        std::fs::write(
            &genesis,
            br#"{"protocolConsts":{"protocolMagic":42},"startTime":1000}"#,
        )
        .unwrap();
        let file = genesis.to_str().unwrap();
        let args = [
            "serve",
            "--node-socket",
            "/tmp/owned.socket",
            "--testnet",
            file,
        ];
        let config = parse(args.map(str::to_owned)).unwrap();
        assert_eq!(config.network, Network::Testnet);
        assert_eq!(config.network_magic, 42);
        assert_eq!(config.genesis_start, 1000);
        assert!(!directory.path().join("shelley-genesis.json").exists());
        assert!(parse(args.into_iter().chain(["--mainnet"]).map(str::to_owned)).is_err());
    }
    #[test]
    fn logging_global_threshold_caps_component_levels_and_off_disables_component() {
        let config = parse(arguments(&[
            "--log-level",
            "warning",
            "--trace-network",
            "debug",
            "--trace-token-metadata",
            "off",
        ]))
        .unwrap();
        let filter = logging_filter(&config);
        assert!(filter.contains("quetzal_network=warn"));
        assert!(filter.contains("quetzal::metadata=off"));
    }
}
