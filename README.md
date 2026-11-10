<p align="center">
  <img src="docs/branding/quetzal-sentinel.png" alt="Quetzal sentinel: an armored robotic Quetzalcoatlus with folded protective wings" width="320" />
</p>

<h1 align="center">Quetzal</h1>

<p align="center"><strong>A Rust Cardano wallet backend. Your keys. Your node. Your view of the chain.</strong></p>

Quetzal connects directly to `cardano-node` and provides a `cardano-wallet` compatible
HTTP API for wallet applications. It handles key management, address discovery,
transaction construction and signing, persistent wallet state, and recovery when
the chain rolls back or the node disconnects. The connected full node validates
Cardano's ledger and consensus; Quetzal follows that node through the native
node-to-client protocols.

The sentinel is a robotic **Quetzalcoatlus**, the prehistoric pterosaur. Its armor
and watchful posture represent local verification and resilient wallet state.

## Validation status

The current replacement implements the pinned public `cardano-wallet serve`
options, configured token metadata, native sync progress, mutual TLS and stdin
EOF shutdown. See the [replacement validation report](evidence/20261110-replacement/VALIDATION.md)
for exact results and remaining qualification gaps.

| Validation | Result |
| --- | --- |
| Open stdin / ordinary bytes / EOF / immediate EOF / flag absent | **Pass over HTTP and mutual TLS** |
| Same-database latest-acknowledged/funded/pending EOF recovery | **Pass: Shelley and Icarus Byron over mutual TLS** |
| Pinned public CLI / configured token registry profiles | **Pass: 5/5 and 9/9 cases** |
| Workspace tests | **99 passed, zero failures; one existing live-node test ignored** |
| Strict Clippy, formatting, release and Nix package build | **Pass** |

The current independent catalogue contains **652 obligations and 44 profiles**.
Its full runner aborts while replacing precreated reports; the
[harness handoff](evidence/20261110-replacement/HARNESS-BLOCKER.md) documents the
failure. Unchanged public profiles are executed separately with their own
receipts. The full cohort remains incomplete, with no imported passing reports.

The preceding mutual-TLS commit `dd6676e` passed **646/646 obligations, 3,808
checks and all 38 profiles** against one frozen candidate and adapter. Those
[historical receipts](evidence/20261109-mtls/VALIDATION.md) include differential
HTTP checks against the Haskell wallet, independent internal vectors and real
native-node controls. The conformance suite is maintained separately; its
assertions were not changed to make Quetzal pass.

Actual Daedalus launch-and-use and the user-deferred mainnet, lagging/ahead and
representative-history performance qualification remain outstanding. The earlier
[V58 receipts](evidence/20261109-v58-gate/VALIDATION.md)
remain historical evidence for the old 643/37 contract without TLS.

## Wallet capabilities

- **Wallet families:** Shelley, random and Icarus Byron, shared wallets, and
  watch-only workflows, with address and key derivation compatible with the reference.
- **Transactions:** construct, balance, sign, decode and submit; multiasset payments,
  mint/burn, metadata, validity bounds, collateral, fees and coin selection.
- **Staking and governance:** delegation, registration and refunds, rewards,
  stake-pool lifecycle, DRep lookup and vote delegation.
- **Recovery:** checkpoint restoration, atomic rollback of wallet ledger state,
  pending transaction persistence, expiry, forgetting and scheduled resubmission.
- **Credentials:** authenticated root-key storage, passphrase changes and recovery,
  and migration of supported legacy credential formats.
- **Integration:** versioned `/v2` HTTP routes and native chain-sync, local-state-query
  and local-transaction-submission protocols through Pallas.

The qualified native controls include Conway and a live Babbage-to-Conway
transition. The recorded toolchain uses `cardano-node` 11.1.3 and `cardano-cli`
11.2.3; exact independent tool identities are in the
[run manifest](evidence/20261109-mtls/identities.json).

## Build

The pinned Nix development shell supplies Rust and native build dependencies.
The flake currently targets **x86_64 Linux**.

```sh
nix --quiet develop --command cargo build --locked --release --workspace
nix --quiet develop --command install -Dm755 target/release/quetzal target/daedalus/cardano-wallet
```

This produces:

| Executable | Purpose |
| --- | --- |
| `target/release/quetzal` | Wallet HTTP service |
| `target/daedalus/cardano-wallet` | Tested drop-in copy of the same release binary |
| `target/release/quetzal-conformance-adapter` | JSON-line adapter exposing real wallet internals to the independent suite |

## Connect to your node

Start a `cardano-node` for the network you want to use and make its Unix socket
accessible to the wallet process. Quetzal does not launch or configure that node.

For a testnet or dedicated local cluster:

```sh
./target/release/quetzal serve \
  --database ./state/testnet-wallet \
  --node-socket /path/to/node.socket \
  --testnet /path/to/genesis/byron-genesis.json \
  --port 8090
```

The `--testnet` argument is the **Byron genesis file**, not a network name or magic
number. Native node queries supply era timing and Shelley genesis parameters;
no neighboring file named `shelley-genesis.json` is required.
The genesis and socket must belong to the same network.
Use a separate database directory for each network.

For mainnet, pass `--mainnet`:

```sh
./target/release/quetzal serve \
  --database ./state/mainnet-wallet \
  --node-socket /path/to/mainnet/node.socket \
  --mainnet \
  --port 8090
```

Check the service and its native chain header:

```sh
curl --fail http://127.0.0.1:8090/v2/network/information
curl --fail http://127.0.0.1:8090/v2/blocks/latest/header
```

`--node-socket` and exactly one of `--mainnet` or `--testnet FILE` are required.
The default listener is loopback `127.0.0.1:8090`. Omitting `--database` uses a
temporary database; pass a persistent directory for real wallets.
Without TLS flags the loopback listener serves HTTP for
local development and semantic fixtures; it does not satisfy transport conformance.

## Mutual TLS

Daedalus requires mutually authenticated TLS. Supply all three certificate paths:

```sh
./target/release/quetzal serve \
  --database ./state/mainnet-wallet \
  --node-socket /path/to/mainnet/node.socket \
  --mainnet \
  --port 8090 \
  --tls-ca-cert /private/tls/ca.crt \
  --tls-sv-cert /private/tls/server.crt \
  --tls-sv-key /private/tls/server.key

curl --fail --cacert /private/tls/ca.crt \
  --cert /private/tls/client.crt --key /private/tls/client.key \
  https://127.0.0.1:8090/v2/network/information
```

The CA file trusts client certificates; each client needs a certificate signed by
that CA with client-auth purpose. The PEM server certificate chain and matching
private key identify the server. Its certificate needs server-auth purpose and
a subject alternative name matching the host clients use. Clients must verify
the server CA and hostname. Keep keys private and provision certificates through
the owning application or deployment.

Missing or malformed TLS files, mismatched certificate/key pairs and partial flag
sets fail startup. TLS mode exposes
secure listeners and rejects plaintext and unauthenticated TLS clients.
`--listen-address` accepts an IP, hostname or supported wildcard; `--host` remains
an alias. Non-loopback binds require TLS. The optional UI uses the same TLS policy.
These flags support the secure conformance launch. An actual pinned Daedalus
launch-and-use run is still required before claiming drop-in application compatibility.

## Drop-in executable and server options

The tested artifact is `target/daedalus/cardano-wallet`, copied byte-for-byte from
the qualified release binary. The Nix package installs `bin/cardano-wallet` as an
alias of `quetzal`:

```sh
nix build .#quetzal
./result/bin/cardano-wallet serve --help
```

Point Daedalus's wallet executable setting at the absolute `cardano-wallet` path.
Its public `serve` arguments go directly to the binary. No argument-filtering
wrapper is needed. `version` and `--version` report Quetzal's actual identity.

Fixed or random API/UI ports, `--listen-address`, `--sync-tolerance`,
`--pool-metadata-fetching`, `--token-metadata-server`, `--ipfs-gateway-url`,
`--log-level` and the named tracing options are effective. Use `serve --help` and
`serve --help-tracing` for their syntax and conflicts. The optional UI is Quetzal's
read-only wallet view. Pool metadata configuration persists in the wallet database.

`--sync-tolerance 300s` is the default. Network progress uses the connected node's
actual tip and native era clock independently of historical pool replay. Syncing
responses include the reference percentage; a disconnected node reports
`not_responding` without a percentage.

`--token-metadata-server URL` queries the registry's `metadata/query` endpoint
for subjects formed from policy ID and asset name. Asset list/detail responses
include validated name/description and available ticker, URL, logo and decimals.
Missing metadata is omitted; fetch/parse failures expose the reference metadata
error. Omitting the option disables registry requests. Decimals never scale
ledger quantities.

## Parent-controlled shutdown

Pass the valueless `--shutdown-handler` flag when a parent application owns
Quetzal's stdin pipe. The service keeps running while that pipe is open; ordinary
input bytes do not stop it. Closing the last writer sends EOF and requests a clean
shutdown. The flag works with the mutual TLS options above.

Shutdown stops accepting connections, drains admitted requests, joins background
and wallet workers, and flushes both wallet database families before returning
success. SIGTERM and Ctrl-C use the same cleanup path. A failed cleanup returns
a nonzero exit status.

With the flag, an already closed stdin (including `/dev/null`) requests immediate
shutdown. Without it, stdin EOF is ignored. Parents that use the flag must keep
the pipe open for the service's lifetime and allow up to 30 seconds for shutdown.

## Security model

**Keys stay with the wallet process.** Current root-key envelopes use Argon2id
and authenticated XChaCha20-Poly1305 encryption, with the reference production
KDF parameters. Sensitive key material uses zeroizing buffers. Supported legacy
credentials authenticate before an atomic migration to the current envelope.
Wallet databases and their backups remain sensitive material.

**Ledger state comes from your full node.** Wallet workers consume native blocks,
persist checkpoints and undo ledger mutations on rollback. Locally submitted
transactions retain their provenance and signed bytes across restarts and retry.
The latest-header endpoint reads one acquired native snapshot, keeping its slot,
hash and block height consistent across a branch switch.

**The API defaults to loopback.** Configure mutual TLS to authenticate API clients;
non-loopback HTTP is rejected. A trusted client certificate grants access to the
wallet API, not a separate per-wallet authorization policy. A spending passphrase
protects key use and does not replace transport authentication. Restrict certificate
and database access to the owning application or operator.

The committed conformance receipts document tested behavior. They are separate
from a security audit or a production deployment assessment.

## Development and validation

Run checks through the project shell:

```sh
nix --quiet develop --command cargo test --locked --workspace
nix --quiet develop --command cargo fmt --all --check
nix --quiet develop --command cargo clippy --locked --workspace --all-targets -- -D warnings
```

For the complete external suite, build its `wallet-conformance` runner and supply
an implementation configuration containing the candidate launch, adapter, native
cluster configuration and independent reference tools:

```sh
wallet-conformance conform \
  --config /absolute/path/to/implementation.json \
  --workspace /absolute/path/to/a-new-private-workspace
```

The workspace must be new. The runner owns fresh fixtures and services, executes
all configured profiles and writes `result.json`, `coverage.json` and individual reports.
The committed [TLS configuration](evidence/20261109-mtls/execution-config.json)
and [identity manifest](evidence/20261109-mtls/identities.json) record the qualified
setup; their machine-specific paths must be provisioned for another host. The
configuration points HTTPS at a distinct loopback port and passes the three TLS
file placeholders to the same candidate executable as the HTTP fixture launch.
Keep fixture keys and node/wallet databases private. Commit public receipts rather
than complete runtime workspaces.

## Architecture

| Crate | Responsibility |
| --- | --- |
| [`quetzal-api`](crates/quetzal-api) | HTTP routes, JSON projections, configuration and background workers |
| [`quetzal-wallet`](crates/quetzal-wallet) | Wallet operations, transactions, ledger synchronization and sled persistence |
| [`quetzal-network`](crates/quetzal-network) | Native node connections and mini-protocols |
| [`quetzal-address`](crates/quetzal-address) | Address derivation, codecs and discovery |
| [`quetzal-crypto`](crates/quetzal-crypto) | Keys, signatures, password handling and encrypted root envelopes |
| [`quetzal-primitives`](crates/quetzal-primitives) | Shared wallet, transaction, address and ledger types |

Pallas supplies Cardano primitives and protocol codecs. Sled supplies embedded
persistence. Tokio and Axum provide the asynchronous service runtime.
The [specification](specs/001-wallet-rust-backend/spec.md),
[implementation plan](specs/001-wallet-rust-backend/plan.md) and
[task record](specs/001-wallet-rust-backend/tasks.md) describe the implementation.

## Database compatibility

Current wallet identifiers use the reference's 20-byte identity format.
There is no database migration from the
earlier 32-byte-ID prototype: retain its data and restore into a fresh database
for this version.

## License and identity

The code is licensed under [Apache-2.0](LICENSE), as declared in the workspace
manifests. The sentinel asset and its generation prompt are documented in
[branding](docs/branding/README.md).
