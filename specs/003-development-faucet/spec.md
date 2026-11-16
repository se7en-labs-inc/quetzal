# Quetzal development faucet

Provide a standalone Rust HTTP faucet for development testnets. It must not
depend on cardano-wallet, cardano-cli, local-cluster, a wallet API, or subprocesses.
Use Pallas for local-state queries, Conway transaction construction, Ed25519
signing and local transaction submission through an existing cardano-node socket.

POST /fund accepts ADA payment outputs and returns the submitted transaction ID.
GET /health reports process readiness. Funding sources are explicitly configured
testnet Shelley or Byron key addresses and signing-key files. Accept Shelley, shared/script
and Base58 Byron recipients, including a bech32 representation of Byron bytes.
Validate address network, positive amounts, request limits and live minimum ADA.
Extended Ed25519 funding keys are required (user confirmed 2026-11-16). Accept
64-byte extended secrets, 96-byte extended keys with chain code, and 128-byte
legacy keys with embedded public key and chain code, as raw hex or supported
Shelley/Byron text envelopes. Validate lengths, scalar structure, embedded public
key and source address; sign with Pallas SecretKeyExtended. Reject malformed
layouts rather than truncate or reinterpret them as ordinary seeds.
Reject reward addresses, mainnet operation and unsupported eras explicitly.

Use live UTxOs, protocol parameters, signed-size fees, a finite validity interval
and change to the funding address. Never mint funds or invent genesis inputs.
Initially support ADA-only inputs and outputs and one sufficiently large funding
UTxO per request; assets and multi-input selection are out of scope.

Serialize requests and persist input reservations before submission so parallel
requests and process restarts cannot reuse pending inputs. Ambiguous submission
failures retain reservations. Explicit node rejection may release them. No
success response before node acceptance; acceptance does not imply confirmation.
Bound node operations and bind only to loopback. Never return key material.

Acceptance: meaningful validation, signature/fee/conservation, reservation and HTTP
tests; workspace fmt, strict Clippy and tests; release Nix package containing the
faucet; real-node funding of Shelley and Byron recipients with node-confirmed
outputs. Point the existing migration driver at this endpoint without weakening
its five-family migration comparisons. Reference SQLite fixture provenance remains
test-only; the faucet itself can run against any configured Conway testnet node.
