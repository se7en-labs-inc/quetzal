# Development faucet

`quetzal-faucet` is a separate loopback HTTP service for funding development
wallets from existing Conway testnet funds. It uses Pallas directly for node
queries, transaction construction, Ed25519 signing and submission. It has no
cardano-wallet, cardano-cli, local-cluster, shell or subprocess dependency.

Build with `nix build` (the workspace package includes `bin/quetzal-faucet`), or
run from the development shell:

```sh
nix develop --command cargo run -p quetzal-faucet -- --config /private/faucet.json
```

Configure a running testnet node and an already-funded Shelley enterprise or
Byron address whose payment key is in the configured signing-key file:

```json
{
  "node_socket": "/run/cardano-node/node.socket",
  "network_magic": 42,
  "listen": "127.0.0.1:19494",
  "state_directory": "/private/faucet-reservations",
  "max_lovelace": 1000000000,
  "sources": [
    {
      "address": "addr_test1...",
      "signing_key_file": "/private/funding.seed"
    }
  ]
}
```

The signing-key file can contain raw hex for a 32-byte Ed25519 seed, a 64-byte
extended secret, a 96-byte extended secret plus chain code, or a 128-byte legacy
extended key (64-byte secret, 32-byte public key, 32-byte chain code).
JSON text envelopes use a CBOR byte string in `cborHex`; supported types are
`PaymentSigningKeyShelley_ed25519` (32 bytes),
`PaymentExtendedSigningKeyShelley_ed25519_bip32` (96 or 128 bytes), and
`PaymentSigningKeyByron_ed25519_bip32` (128 bytes). Extended scalar structure and
embedded public keys are checked. Malformed lengths and trailing CBOR are rejected.

No key-generation executable is required by the service. Keep keys and the state
directory private. Shelley source addresses must match the key's testnet enterprise
address. Byron sources require a key with chain code; their full address root is
validated against the public key, chain code and stored attributes. Pallas creates
the corresponding bootstrap witness, preserving those attributes. The service
does not create genesis funds or restore a wallet.

`GET /health` reports process readiness. To request funding:

```sh
curl --fail-with-body http://127.0.0.1:19494/fund \
  -H 'content-type: application/json' \
  -d '{"payments":[{"address":"addr_test1...","lovelace":30000000}]}'
```

`POST /fund` accepts 1–32 payments and returns HTTP 202 with `transaction_id`,
`fee`, `lovelace` and `status: "accepted"` after node admission. This is not
confirmation. Payment outputs support Shelley key/script/shared addresses and
Byron Base58 addresses. A bech32 wrapper of Byron bytes is decoded directly to
the same binary address; no external address renderer is involved. Shelley
recipients must use the testnet network ID; Byron recipients must carry the
configured testnet magic. Reward addresses and mainnet operation are rejected.

This initial version funds ADA only. It selects one sufficiently large ADA-only
UTxO at a configured source, pays live size-based fees, checks live minimum ADA
and returns change to that source. Native assets, minting and combining small
UTxOs are not supported. Confirmed change becomes available to future requests.

Requests are serialized. A sled reservation containing the signed transaction
is flushed before submission, preventing reuse of a pending input even after
restart. Explicit node rejection releases the input; timeout, disconnect or
process interruption retains it because acceptance is uncertain. Keep the same
state directory on restart. Do not delete reservations to retry an ambiguous
request: inspect the chain first. A lost response may still represent an accepted
payment, so blind retries can send additional funds from another input.

Malformed requests return a client error; unavailable funds/node failures return
503 and node-operation timeout returns 504. Keys are never returned by HTTP.
The service binds only to loopback and rejects mainnet magic. It is intended for
local development, not a public faucet or production funds service.
