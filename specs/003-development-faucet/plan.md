# Implementation plan

New `quetzal-faucet` workspace crate with its own binary and testable library.
Reuse workspace-pinned Pallas 0.32.1 and quetzal-network's connection/query/submission
code. Use axum for HTTP, sled for durable reservations, zeroize for loaded seeds.
Load JSON configuration: socket, testnet magic, loopback listen address, state
directory, limits and funding sources (address, signing-key file). Signing-key
files accept a raw hex 32-byte seed or the standard JSON Ed25519 text envelope;
parsing that file format requires no external executable. Extended keys are
required: use Pallas SecretKeyExtended for 64-byte secrets and secret portions
of 96-byte (secret/chain-code) and 128-byte (secret/public/chain-code) layouts.
Real retained genesis sources are Byron, not Shelley enterprise addresses.
Retain the extended chain code, validate Byron source roots against public key,
chain code and persisted attributes, and construct Pallas bootstrap witnesses
for those inputs. Ordinary Shelley inputs continue to use vkey witnesses.
For 128-byte layouts verify the embedded public key against the derived one.
Accept supported ShelleyExtended and Byron envelope types with exact layout
length validation. Keep temporary key material zeroized and avoid secret Debug.

Query address-filtered UTxOs through Pallas queries_v16, validate source credentials
against the signing public key and network, select an unreserved ADA-only output,
build Conway outputs/change and iterate the fee using the fully signed CBOR size.
Check output minima, max transaction size and arithmetic. Persist/flush reservations
before local submission; preserve them across cancelled or ambiguous requests.
Connection guards abort Pallas multiplexers on errors and cancellation.

Do not alter production wallet endpoints or add faucet credentials there. Include
the separate binary in the existing workspace Nix package and document setup.
Integration changes stay in 002's owned migration driver. Conformance is read-only.
Constitution: Pallas era-aware types, testnet-only key handling, sled storage,
Nix builds and focused transaction/HTTP/native validation satisfy relevant rules.
