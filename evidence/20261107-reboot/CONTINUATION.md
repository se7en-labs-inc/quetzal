# Paused for reboot — 2026-11-07

The user explicitly requested this pause. Resume only when asked. Objective is
still all 643 conformance obligations, with **Shelley work before Byron**.
Do not end future work at a partial audit. Compatibility changes and the pinned
ed25519-bip32 0.3.2 legacy alias are explicitly approved. Do not ask again.

## Workspace and verification

Work in `/home/sam/work/se7en/quetzal/wallet-migration`, branch
`001-wallet-rust-backend`. Reuse the existing Spec Kit artifacts. C005–C008 remain
open. Preserve all dirty changes; nothing was staged or committed. Renamed
`crates/quetzal-*` files and evidence are untracked, while old Automata files
show deleted. Nix Git source filtering excludes untracked files: use an isolated
source copy for a complete packaged build, rather than staging user changes.
`.bare/config` is read-only and origin still points at the old Automata location.

Last coherent complete HTTP/internal audit plus funded evidence:
`../20261107-core-v10/coverage.json`: **299/643**, incomplete. Do not call the
wallet conformant. V13 has a newer complete nine-suite HTTP replay, but no fresh
combined gate. V13 families 62/102; v10 families 56/102. Key HTTP 46/46, basic
lifecycle 11/11 and signatures 4/4 pass. Preserve all failed reports.

V12 funded report `../20261107-core-v12/live-payments.json`: 149 checks, 147 pass.
Shelley payment/migration execution, confirmed change discovery and sequential
discovery pass. Remaining workflow failures: plaintext metadata decryption error
message and formatting the node's structured spent-input rejection. V13 corrects
those; funded V13 has not run. V11 stopped early because null optional selection
metadata violated schema; V12 omits the absent field.

V13 workspace tests and release build pass; logs copied into this directory.
Eight API dead-code warnings remain; Clippy is outstanding. Newest writer edits
compile in the debug adapter (`quetzal-writer-build.log`), but have not had their
post-fix vector replay or workspace tests. Release binaries are frozen V13 and
**do not contain the newest writer module**. Hashes are beside each generation.

## Changes since the earlier checkpoint

Real payment construction uses live fee/minimum-UTxO/max-size parameters, exact
signed ledger size, native asset conservation/change and authenticated metadata.
Actual node submission persists pending state before broadcast, flushes it,
serializes admission, rejects unavailable owned inputs and removes rejected
pending records. Pending/expired history and deletion are implemented. Uncertain
transport submissions still need startup/retry handling.

Per-wallet restoration workers scan from their own checkpoints. Atomic sled
block commits journal UTxOs, history, checkpoints, pending records and address
states; rollback restores spent inputs and removes dynamically discovered
addresses without reverting wallet names. Validity-aware consumes/produces are
used. Sequential Shelley/Icarus pools extend during block processing. Workers
still abort tasks on deletion, risking leaked Plexer connections; implement
graceful cancellation. Undo journals need eventual pruning.

Validity intervals now set lower slot zero and requested slot/relative-second
bounds; complete live validity/expiry verification remains. Relative clock
rounding and metadata-only validity need review.

Shelley migration planning/execution now sweeps actual available inputs and all
native assets, converges ledger fee, partitions oversized transactions, signs
all batches before broadcasting and submits to the node. Plans are 202 with
real selected/leftover/fee/selection data and derivation paths. Dust/many-input
boundary behavior needs further coverage. Funded Shelley migration passed V12.

CIP-83 crypto is in `quetzal-crypto/src/metadata.rs`: salted AES-256-CBC,
PBKDF2-HMAC-SHA256 10000 rounds, PKCS7. Ordered transformation and detailed-schema
bridge in wallet `metadata_encryption.rs`. All six supplied encryption vectors
passed V10. Public construct/decode hooks and metadata-only self-transactions are
implemented but live metadata still waits on complete shared-wallet behavior.
Construction returns base64 metadata CBOR in coin selection when present.

External `/proxy/transactions` now actually submits raw CBOR and returns the
ledger body ID; it previously returned a placeholder. Node rejection parsing
recognizes the actual Conway mempool text constructor. Captured spent rejection:
`82028182068182077844416c6c20696e7075747320617265207370656e742e205472616e73616374696f6e206861732070726f6261626c7920616c7265616479206265656e20696e636c75646564`.
Other structured ledger rejections still fall back to raw CBOR; implement their
actual constructors, especially invalid witnesses/validity. Wallet submission
still has its older error formatting and early signature rejection.

Shared wallets now use upstream ID = Blake2b160(account xpub + canonical payment
script text + optional delegation script text), resolve `self` to checksummed
acct_shared_xvk, return actual wallet schema, support list/rename, and use base
script addresses when delegation is supplied. Active creation/ready/rename now
pass differential checks. Address index 1 still differs from reference; investigate
shared discovery ordering/derivation. Incomplete templates are detected and no
addresses derived, but activation, signing and startup worker handling remain
unfinished. Shared cosigner parsing now validates bech32 checksum/HRP. Native
script encoding has its own old helper; consolidate with typed script functions.

Newest `wallet/src/writer.rs` implements real Shelley/bootstrap witnesses,
idempotent witness merging, native script witnesses and Conway staking/voting
certificates. First replay 22/28 passed. Remaining six were witness ordering and
error envelope codes. Latest patch sorts vkeys by Blake2b224(key), matching ledger
Ord, and uses `unsupported_era` / `deposit_required` per the reference adapter.
The corrected debug adapter compiles; **rerun writer vectors first**. API signing
still appends duplicate witnesses: integrate `writer::merge_vkey` there. Favor
Shelley work; defer additional Byron functionality per the user's instruction.

## Remaining major domains

- Exact Haskell seeded minimal/optimal selection: 157 failures; generic Rust
  selector invariants pass but exact seeded oracle behavior is unimplemented.
- Wallet credential transitions/migration/fault atomicity: 61 profiles unimplemented.
- Wallet-selection/mint/burn profiles: 34 unimplemented.
- Shelley staking, voting/delegation, withdrawals, collateral and partial balancing
  still contain stubs; remove synthetic transaction records before coverage claims.
- Shared activation and merged signing; watch-only funded external signer workflow.
- Durable retry/resubmission, restart/rollback controls, pool metadata/lifecycle,
  enactment/era-boundary checks and all remaining HTTP rejection precedence.
- DB is globally keyed by WalletId; identical root IDs across wallet families can
  collide. Family numeric-decimal creation reports 409, causing cascading undefined
  capture variables. Investigate the actual preceding wallet/family cleanup.
- Required full 37-profile gate and provenance issue below remain unresolved.

## Conformance commands

Suite root `/home/sam/work/se7en/cardano-wallet-conformance/master`; already built
`target/debug/wallet-conformance`. Profiles in `fixtures/execution-profiles.json`:
9 HTTP, 16 internal, 537 HTTP cases and 1241 internal vectors. Adapter consumes
one newline-terminated JSON request per process. Do not read expected vectors
as an implementation lookup; use actual production functions and upstream code.

Build with `nix --quiet develop --command cargo test --workspace` and
`nix --quiet develop --command cargo build --release`. No Python/global installs.
Never enumerate `/nix/store`; only read specifically resolved files.

HTTP: `wallet-conformance http --target http://127.0.0.1:18792/v2/
--reference http://127.0.0.1:18793/v2/ --environment LABEL --suite ABS_SUITE
--report REPORT --timeout-seconds 10`. Internals: `vectors --adapter ABS_ADAPTER
--target SAME_TARGET --environment SAME_LABEL --corpus ABS_CORPUS ...
--operation OP ... --report REPORT`. Empty profile operation list means no filters.
Keep frozen binaries during replays. Never run list-wallet HTTP suites concurrently
with live workflows creating wallets. Gate includes latest report for each profile
and live reports from the same generation/target/environment, not older failures.

Live basic plus funded Shelley: `live --target http://127.0.0.1:18792/v2/
--node-socket RUN/node.socket --network-magic 42 --faucet http://127.0.0.1:18779/
--cardano-cli PINNED_CLI --shelley-genesis RUN/cluster/shelley-genesis.json
--byron-genesis RUN/cluster/byron-genesis.json --environment LABEL --report REPORT
--timeout-seconds 60 --payments --history --discovery --concurrency`.
Other flags: --validity, --metadata --openssl PIN, --input-limits, --balance,
--collateral, --delegation-fees, --voting; see --help for other required controls.

## Restart private services after reboot

All owned services were stopped for this pause. Old disposable run directory was
`/tmp/qz-1107-approved-9Glkn5`, marker `/tmp/quetzal-node-workspace`; do not assume
/tmp or PTY sessions survive reboot. Use a fresh cluster/DB for coherent evidence.

Pinned binaries (resolve through Nix again if unavailable):

- Cluster: `/nix/store/fxhz93bwjb7mz1v1z8yghs7fmlwlcsyb-local-cluster-exe-local-cluster-2026.9.16/bin/local-cluster`
- Wallet: `/nix/store/3pvrzwp16icjdlyl85cz9wpkqnyyrn75-cardano-wallet-application-exe-cardano-wallet-2026.9.16/bin/cardano-wallet`
- Node: `/nix/store/fga6fircrxgi6i9zv79ckvsabmcs10yw-cardano-node-exe-cardano-node-11.1.3/bin/cardano-node`
- CLI: `/nix/store/fgk64gnhp9h23a8rjdjhr4armw50qfv2-cardano-cli-exe-cardano-cli-11.2.3.0/bin/cardano-cli`
- OpenSSL: `/nix/store/3ffi89rlwcqww9x71pf95miyik5jhnv9-openssl-3.6.2-bin/bin/openssl`
- Cluster configs: `/nix/store/0j7mw9mkdbv6knk0ymv3j59jp3bwm1wh-source/lib/local-cluster/test/data/cluster-configs`

Create RUN with mktemp, write marker, and **create** RUN/funds.json before cluster
launch with `{"pureAdaFunds":[],"maryAllegraFunds":[],"massiveWalletFunds":[]}`.
Launch foreground `LOCAL_CLUSTER_ERA=conway` with pinned CLI/node dirs prepended
to PATH, then cluster `--cluster-configs CONFIGS --cluster RUN/cluster
--cluster-logs RUN/logs --socket-path RUN/node.socket --monitoring-port 18779
--faucet-funds RUN/funds.json +RTS -N2`. Keep each foreground process owned by its
tool session and redirect logs to RUN. Genesis files appear under RUN/cluster.
Candidate `target/release/quetzal serve --database RUN/candidate.sled --node-socket
RUN/node.socket --testnet RUN/cluster/byron-genesis.json --port 18792`.
Reference uses `serve --database RUN/reference --node-socket RUN/node.socket
--testnet RUN/cluster/byron-genesis.json --port 18793 --ui-port 18813
--pool-metadata-fetching none +RTS -N2`.

## External sources and oracle provenance

Agentic-nix skill was used. Pinned wallet source indexing completed successfully:
`git::https://github.com/cardano-foundation/cardano-wallet@2fc5c383c94b`.
Balance source index: `git::https://github.com/cardano-foundation/cardano-balance-transaction@897577325745`.
Other indexed dependencies include Pallas 0.32.1, cryptoxide 0.4.4,
ed25519-bip32 0.3.2, cardano-addresses 4.0.2, cardano-crypto-wallet 0.2.0.0.
Use BM25 exact symbols while Ollama/embedding endpoint is unavailable; the
previous failure was explained. Do not restart completed ingestion jobs.

Trusted key oracle required SHA256:
`9326e291df8ccb064cd77740a3a7063cf84cc382d5dc4481be28a47680d44114`.
Actual pinned KeyAdapter.hs source SHA matches
`5ebf8ed97a16cf3a15757e70e919d2d56ce8b9a43ddb34823fca716c4340d09c`,
but rebuilding with documented GHC flags produces
`337e2b28428a1a241ff6d27bae025069c619fb5a5836c55d25b4ae4ed2122993`.
Original required executable is missing. Do not bypass or change fixed harness
digests. Fullconform/rollback trusted executable controls remain blocked by this.
GHC environment resolved to
`/nix/store/3f6zix5gy2f9gq7ihzsjp4qa578bp5yf-ghc-shell-for-packages-ghc-9.12.3-env`.
Relative/absolute source paths gave same digest; O0/O2/threaded variants differed
but none matched. Immutable wallet source pin confirmed identical narHash through
Nix metadata. See conformance `docs/reference-keys.md` and toolchain Nix files.
