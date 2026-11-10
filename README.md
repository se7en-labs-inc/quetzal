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

The mutual-TLS implementation passes the corrected independent contract:

| Validation | Result |
| --- | --- |
| Required conformance obligations | **646 / 646** |
| Executed checks | **3,808 passed, zero failures** |
| Fresh execution profiles | **38 / 38** |
| Mutual TLS, server authentication and plaintext rejection | **Pass** |
| Native rollback and reapplication | **224 / 224 checks** |
| Workspace tests, strict Clippy, formatting, release and Nix package build | **Pass** |

These receipts describe one complete run against one frozen candidate and adapter,
with unchanged binary identities throughout. The suite includes differential HTTP checks against
the Haskell wallet, independent internal vectors, and real native-node controls.

Start with the [validation report](evidence/20261109-mtls/VALIDATION.md),
[machine-readable result](evidence/20261109-mtls/result.json), and
[coverage gate](evidence/20261109-mtls/coverage.json). All
[38 reports](evidence/20261109-mtls/reports/) and earlier failed attempts are
retained for review. The conformance suite is maintained separately; its assertions
were not changed to make Quetzal pass.

Actual Daedalus launch-and-use and performance acceptance remain separate and
unqualified. The earlier [V58 receipts](evidence/20261109-v58-gate/VALIDATION.md)
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
```

This produces:

| Executable | Purpose |
| --- | --- |
| `target/release/quetzal` | Wallet HTTP service |
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
number. Quetzal also reads `shelley-genesis.json` in the same directory to obtain
slot and epoch lengths. Both files and the socket must belong to the same network.
Use a separate database directory for each network.

For mainnet, omit `--testnet`:

```sh
./target/release/quetzal serve \
  --database ./state/mainnet-wallet \
  --node-socket /path/to/mainnet/node.socket \
  --port 8090
```

Check the service and its native chain header:

```sh
curl --fail http://127.0.0.1:8090/v2/network/information
curl --fail http://127.0.0.1:8090/v2/blocks/latest/header
```

The default configuration uses mainnet, `/tmp/cardano-node.socket`, loopback
`127.0.0.1`, port `8090`, and `./quetzal.sled`. Pass explicit paths when operating
more than one instance. Without TLS flags the loopback listener serves HTTP for
local development and semantic fixtures; it does not satisfy transport conformance.

## Mutual TLS

Daedalus requires mutually authenticated TLS. Supply all three certificate paths:

```sh
./target/release/quetzal serve \
  --database ./state/mainnet-wallet \
  --node-socket /path/to/mainnet/node.socket \
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
one secure listener and rejects plaintext and unauthenticated TLS clients.
`--host` accepts an explicit IP, including `::1`; non-loopback binds require TLS.
These flags support the secure conformance launch. An actual pinned Daedalus
launch-and-use run is still required before claiming drop-in application compatibility.

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
all 38 profiles and writes `result.json`, `coverage.json` and individual reports.
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
