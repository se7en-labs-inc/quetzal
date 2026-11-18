# Shared sync Preview diagnostic, 2026-11-18

Uncommitted source in `005-responsive-runtime`, based on
`84f2f4d691df3a32928766733def609164888475`. This is a focused native diagnostic,
not complete feature 005 qualification or an unchanged-baseline comparison.

## Implementation under test

One owned 16 MiB-stack synchronization thread runs a live stream and a fair
catch-up stream. Cohorts share fetching/decoding and normalized block indexes;
wallets retain their own address-byte ownership/discovery and UTxO maps. Same-block
gap discovery reaches a fixed point before ledger-order effects. Relevant effects
commit immediately, empty progress at most every 256 blocks. Pool history uses the
same service, reversible 256-block batches and persisted intersection points.
See `../../docs/shared-sync.md` for bounds, lifecycle and remaining requirements.

## Environment and identities

- Same Ryzen AI 9 HX 370 / approximately 62 GiB RAM / Linux 6.18.60 / ZFS host
  as the preceding diagnostic. Builds and tests ran concurrently with replay;
  this was not a quiet-host performance trial.
- Preview magic 2; node 11.1.3; CLI 11.2.3.0; Mithril client 2642.0.
- Verified Mithril snapshot
  `51c321956b4321423f12111781e89ce14fd40483c64f7e62d3b3199e4928517e`,
  epoch 1485, immutable 29710. The node subsequently followed the live network.
- Node descriptor limit explicitly raised to 65,536 for the isolated run.
  API is loopback plaintext on port 40281. No Daedalus integration or TLS latency
  is exercised here.
- The original private wallet store is preserved. The candidate uses a copy;
  no personal wallet database was imported and no spending was authorized.
- Tested executable: worktree `target/release/quetzal`; independent Nix package
  build also passed at
  `/nix/store/b2dvx9jjjsq39ffd8raidzcmi4s8gp2k-quetzal-0.1.0`.
  Exact executable and lock hashes are in `binaries.sha256`; the development and
  Nix binaries have different hashes and are not represented as byte-identical.
  `runtime-source.sha256` records the changed runtime source files. The package
  build log is retained in `package-build.log`.

## Native receipt and restart

All original six wallets reached ready on the final candidate by 18:17:57 UTC.
The newly funded public-test Shelley wallet recognized its receipt and reported
10,000 test ADA available. `native-utxo.json` independently confirms the unspent
output; `funded-history-before-restart.json` records the matching `in_ledger`
transaction. These are public Preview test credentials/outputs, never mainnet.

Healthy EOF shutdown at 18:19:22 completed in approximately 0.19 seconds, joining
the shared thread and flushing the stores. `current-run.jsonl` retains counters
and shutdown events. At that shutdown the counters recorded 248,178 decoded
blocks, 248,268 wallet applications and 48,832 pool applications. These are only
this resumed run's counters, not a full genesis replay benchmark.

Restart at 18:19:39 preserved the funded balance, ready state and advancing tip;
see `funded-before-restart.json` and `funded-after-restart.json`. Pool following
resumed from persisted points instead of starting a separate genesis replay.

## Concurrent late join and HTTP reads

A new watch-only wallet was created at 18:22:02.714 UTC with a known public test
account key and gap 20, beginning at origin. Existing wallets were already live.
`late-wallet-created.json` and `late-join-progress.jsonl` retain its progression
and the funded wallet's continuing ready state, balance and tip.

The fresh wallet was first observed ready at **18:28:22 UTC**, at block 4,869,986,
matching the funded live follower. Creation to this observation took **379.3
seconds (6 minutes 19.3 seconds)**, including the final polling delay of up to
15 seconds. This is a complete fresh watch-only restoration diagnostic, with
approximately 12,800 blocks/second averaged over that upper-bound duration. It
is not the five-fresh-family qualification trial. The funded wallet stayed ready
with its full balance and its tip advanced from slot 128,370,208 in the first
progress receipt to 128,370,483 at completion. Final native tip and API snapshots
are retained separately. The progress log's `wallets`/`live_wallets` counts include
the pool-history consumer; `wallet_blocks` counts wallet applications only.

The optional read-only probe ran 120 rounds at one-second intervals, with at most
four concurrent requests and independent ten-second deadlines:

```sh
target/release/quetzal-runtime-bench probe http://127.0.0.1:40281 OUTPUT 120 1000 10000
```

Across seven wallets (four modern, two Byron, one shared), all 4,080 requests
succeeded, with zero HTTP failures, transport failures or deadlines. The maximum
observed latency was 28.842 ms (modern transaction history). Summary p95 values
were 18.888 ms for modern wallet summaries, 21.021 ms for modern history and
2.109 ms for network information. Endpoint distributions and incremental receipts
are in `api-probe-summary.json` and `api-probe-events.jsonl`. Family/endpoint
samples are insufficient for the independent qualification protocol, and the
probe is not fixed-rate 50 requests/sec.

A second 240-round probe covered later restoration and the transition to live
following: **8,160 more reads, zero failures/deadlines, maximum 50.325 ms**.
The largest endpoint p95 was 21.005 ms. Its receipts are
`api-probe-later-summary.json` and `api-probe-later-events.jsonl`. Together the
two probes completed 12,240 reads; their sample windows are separate and their
per-endpoint percentiles must not be averaged.

## Checks

`nix develop -c cargo test --workspace --all-features`: **143 passed, zero failed,
three explicitly ignored external-fixture tests**. Full output is in
`workspace-tests.log`. Formatting, strict workspace/all-target/all-feature Clippy,
release build and Nix package build also passed on this runtime source.

Focused tests cover shared decode/application counts using the real Pallas
node-to-client protocol; initial rollback-to-intersection handling; reversed
same-block gap triggers; same-block receipt/spend; rollback/replay; discovery
cache invalidation; deferred checkpoint recovery; pool batch idempotence/rollback;
and cancelling/joining multiple wallets during a stalled shared handshake.

## Earlier failures retained

1. The preceding independent-wallet executable overflowed a Tokio worker stack
   during restoration around 17:48. See `baseline-stack-overflow.txt`. This is
   a failure observation, not a controlled baseline speed comparison.
2. The first shared trial mishandled the protocol's initial rollback to the
   chosen intersection, repeatedly reconnecting without processing blocks. It
   also exhausted the node's inherited 1,024-descriptor limit. Shutdown during
   that outage missed its ten-second grace period. The protocol handling was
   corrected and regression-tested; node limits were recorded explicitly.
   See `failed-protocol-trial.jsonl`.
3. An intermediate candidate with the old separate pool-history follower still
   on Tokio progressed to approximately 93%, then overflowed a Tokio worker
   stack at 18:12:20. See `intermediate-progress.jsonl` and
   `intermediate-stack-overflow.txt`. The pool path was moved onto the shared
   service. The exact stack-overflow call site was not independently profiled;
   successful later replay does not establish that root cause by itself.

## Remaining qualification

No full five-fresh-family restoration trial, controlled rollback/outage trial,
abrupt crash matrix, writer-stall/overload trial or unchanged external conformance
run is established here. Historical-range backfill for newly eligible older
addresses, complete byte budgets, typed storage writers and committed API views
remain pending. This diagnostic must not be used to mark all of T034–T038 or
feature 005 complete.
