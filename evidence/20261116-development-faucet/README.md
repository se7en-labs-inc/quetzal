# Development faucet verification, 2026-11-16

Implementation: separate `quetzal-faucet` Rust workspace crate; Pallas-only node
queries, Conway construction, signing and submission. No subprocess or external
wallet/CLI runtime dependency. API: GET /health and POST /fund. ADA-only,
loopback/testnet-only; durable pending-input reservations.

Ten focused tests pass, covering address/network/CRC handling; ordinary/extended
signatures, fees and coin conservation; configuration and source identity checks;
durable reservations, concurrency and task cancellation; HTTP validation/offline
failure; malformed extended layouts/envelopes; Byron source roots and bootstrap
attributes. Explicit fixture testing verifies all 20 real extended-key sources.
Workspace-wide strict Clippy and formatting pass. See extended-key-tests-final.log;
validation.log records the earlier six-test seed-only implementation.

The earlier seed-only Nix release package built successfully with the executable:
`/nix/store/91wd0nzlpygja5ydxsh6xinvhbb6wnfd-quetzal-0.1.0/bin/quetzal-faucet`.
Its --help command was exercised. Extended-key package rebuilds are recorded in
extended-key-package-final.log; the protected current GC root is
`/home/sam/work/se7en/quetzal/restart-context/20261116-development-faucet/release-package`.
The final extended-key package is
`/nix/store/ps85hzksrbyi4inbri78lda938yqh117-quetzal-0.1.0`.

Workspace testing encountered an existing API integration test timeout at
`crates/quetzal-api/tests/shutdown.rs:307`, on the HTTPS wallets request. The
isolated repeat also times out. No API implementation or shutdown-test source was
modified. See workspace-tests-attempt1.log and tls-rerun.log. Do not report the
whole workspace test suite as passing.

The owned migration driver now starts the Quetzal faucet on port 19494 and calls
POST /fund, rather than the local-cluster faucet. Native attempt 4 stops at source
key loading: retained fixture files are typed PaymentSigningKeyByron_ed25519_bip32,
not ordinary 32-byte Shelley Ed25519 seeds. User subsequently required extended
keys. Implemented Pallas extended signing, validated 64/96/128-byte layouts,
embedded public-key checks, and bootstrap witnesses for matching Byron sources.
See native-attempt4.log and native-attempt4-faucet.log for the historical failure.

Attempt 5 then exposed a submission era-tag offset: query era 6 is already the
correct Conway submission tag. Passing 7 selected Dijkstra and was rejected. The
offset was removed, matching the existing wallet submission path. See
native-attempt5.log and native-attempt5-faucet.log.

Attempt 6 successfully funded Shelley, Icarus, random Byron and shared destinations
from real extended-key Byron genesis sources. Four distinct native transactions
each paid 30,000,000 lovelace, with receipts in native-funding-receipts.json. The
fixture verified ready/funded balances for all five families, including watch-only
tracking of the Shelley funds. It then proceeded to migration and failed on
`invalid imported wallet: unowned or duplicate UTxO`. Faucet funding passed;
complete migration qualification did not. See native-attempt6.log and
native-attempt6-faucet.log. All owned processes were cleaned up. Conformance source
remains read-only. No cardano-wallet/CLI process is used by the faucet service;
the existing migration comparison harness retains its external reference/oracle
tools solely for test provenance and verification.
