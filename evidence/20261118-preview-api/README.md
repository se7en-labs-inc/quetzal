# Direct Preview API diagnostic — 2026-11-18

The five-family live run completed 3,120 GET requests with zero HTTP errors,
invalid responses or ten-second deadlines. Maximum response time was 16.316 ms
(wallet list). This did not reproduce a hanging API under this workload.
It does not qualify the responsive-runtime feature or demonstrate improvement.

## Actual scope

- Public Preview, protocol magic 2; cardano-node 11.1.3, Conway.
- Verified Mithril snapshot, epoch 1485, immutable file 29710; exact identity in
  `mithril-snapshot.json`, verification stages in `mithril-verification.jsonl`.
  Immutable database/certificate verification uses Mithril; ancillary files use
  the published ancillary signing key, as recorded by the client caution.
- Official configurations downloaded from
  https://book.play.dev.cardano.org/environments/preview/ on this date;
  file identities in `config-sha256.txt`.
- Fresh isolated sled state. Five disposable empty wallets: Shelley, watch-only,
  Icarus, random Byron, complete shared. No existing databases, synthetic
  checkpoints, personal credentials or funds. Daedalus was not used or changed.
- HTTP loopback, no TLS; at most four GETs concurrently. 120 rounds with a
  one-second pause between rounds, not a fixed requests/second workload.
- Network information, parameters, latest header, settings, all three lists,
  per-wallet summaries, addresses, history capped at 100 and supported UTxO
  routes. Shared has no UTxO route. Receipt routes redact wallet IDs.
- The native node advanced from slot 128366094/block 4869816 to
  slot 128366234/block 4869822, reporting 100.00% sync. Independent CLI receipts
  are `node-tip-five-before.json` and `node-tip-after.json`.
- Wallets were still restoring: approximately 4.3–7.3% when observed afterward
  (`wallet-progress-after.jsonl`). This covers early restoration, not full replay,
  funded transaction histories, spending, migration, saturation or writer stalls.

## Runs and findings

`probe-empty/` sampled startup/reconnection: 21 requests, six fast HTTP 500
`internal_error` responses, all from parameters and latest-header while the node
socket was unavailable. No request hit its deadline. The first node invocation
bound its source address to loopback, which prevented outbound peers; it was
stopped and restarted with `--host-addr 0.0.0.0`. These receipts overlap that
restart and are retained separately. The Quetzal HTTP listener stayed on loopback.

`probe-restoring/` sampled one watch-only wallet: 132 requests, zero failures,
maximum 8.412 ms. The wallet advanced from 0.55% to 1.44% during these rounds.
Watch-only creation returned 201 in 16.158 ms. Four further disposable wallet
creations returned 201 in 32.786–243.956 ms.

`probe-five-families/` sampled five concurrently restoring wallets: 3,120 reads,
zero failures. Representative distributions (microseconds in JSON receipts):

| Endpoint | Samples | p95 (ms) | Max (ms) |
|---|---:|---:|---:|
| Network information | 120 | 13.989 | 15.019 |
| Network parameters | 120 | 13.523 | 14.673 |
| Latest header | 120 | 10.368 | 13.177 |
| Shelley/watch-only list | 120 | 14.403 | 16.316 |
| Shelley/watch-only summary | 240 | 8.984 | 13.242 |
| Settings control | 120 | 0.446 | 0.757 |

Summary and history figures combine families sharing a route template; receipts
are diagnostic distributions, not per-family performance qualification.
All reads completed. Fast errors in the first run were counted as failures.
No API behavior was changed to make this experiment pass.

## Reproduction and identities

Production executable was the previously built feature-005 package:
`/nix/store/pxsf9hw8vm3k7fsj0bxfhspfs834j4pa-quetzal-0.1.0/bin/quetzal`.
Its source snapshot is recorded in `../20261118-responsive-runtime/`.
The new probe is optional and is not part of this production package.
Binary hashes are in `binary-sha256.txt`; probe source/binary in
`probe-sha256.txt`. See `../../docs/runtime-benchmark.md` for probe semantics.

Mithril release 2642.0 (client 0.13.29+e85df3d), archive SHA256
`aba4d675c2c1cf62bf16748a9d448e1a15bf410a142306e6548dfcc38ee89500`,
was installed with a temporary Nix derivation using project-pinned nixpkgs.
Published Preview trust anchors and aggregator were used:
https://aggregator.pre-release-preview.api.mithril.network/aggregator

```sh
mithril-client --json --origin-tag BENCHMARK --log-format-json \
  --log-output "$ROOT/mithril.log" cardano-db download --include-ancillary \
  --download-dir "$ROOT/node-db" \
  51c321956b4321423f12111781e89ce14fd40483c64f7e62d3b3199e4928517e

cardano-node run --config "$ROOT/config/config.json" \
  --topology "$ROOT/config/topology.json" --database-path "$ROOT/node-db/db" \
  --socket-path "$ROOT/node.socket" --host-addr 0.0.0.0 --port 31001 \
  --start-as-non-producing-node

quetzal serve --node-socket "$ROOT/node.socket" \
  --testnet "$ROOT/config/byron-genesis.json" --database "$ROOT/wallet-state" \
  --random-port --shutdown-handler --log-level info

quetzal-runtime-bench probe http://127.0.0.1:42625 \
  "$ROOT/probe-five-families" 120 1000 10000
```

Set `AGGREGATOR_ENDPOINT`, `GENESIS_VERIFICATION_KEY` and
`ANCILLARY_VERIFICATION_KEY` for Mithril. Set ROOT to a private, empty directory;
commands also require the recorded executables and disposable wallet creation.
Full logs, requests and wallet state remain private at
`/home/sam/work/se7en/quetzal/.preview-api-20261118-tZMi4e`.
The verified chain database is retained there for another test.
Both owned servers were stopped and joined: Quetzal reported clean stdin-EOF
shutdown; the node was interrupted after Quetzal exited.

Validation: all five runtime-bench tests passed; formatting, strict Clippy for
this executable and `git diff --check` passed. No compilation ran during the
five-family observation. Independent conformance was not rerun or modified.
