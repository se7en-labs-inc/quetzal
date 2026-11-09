# Frozen V56 validation

The unchanged conformance gate covers **642/643 obligations**. The sole missing
requirement is `wallet.rollback`. Its four mandatory cases were not executed:
the immutable helper rejects the rebuilt independent key oracle before starting
the node/wallet rollback workflow. The exact rejection is retained in
`rollback-blocked.json`: `key adapter differs from pinned source-built oracle`.

`coverage.json` includes this blocked attempt and reports `exhaustive: false`,
`missing: [wallet.rollback]` and `failures: [rollback:workflow]`. The exact36-report
input list is `gate-reports.txt`; no report bodies were edited. The35 reports
with valid prerequisites contain **3,481 passing checks**, with no failures:

| Verification | Checks passed |
|---|---:|
| Nine differential HTTP profiles | 537 |
| Sixteen internal profiles | 1,241 |
| Actual Babbage→Conway transition | 49 |
| Pending crash/restart | 56 |
| Icarus/random Byron pending controls | 180 |
| Expiry | 79 |
| Checkpoint reuse and catch-up | 29 |
| Real conflicting-mempool retry, three wallet styles | 108 |
| Native registered-DRep lookup | 6 |
| Real parameter enactment and pool lifecycle | 161 |
| Expanded funded workflows, including staking/rewards and families | 953 |
| Actual node disconnect/reconnect and catch-up | 82 |

Every candidate report uses target `http://127.0.0.1:19097/v2/`, environment
`quetzal-v56`, the same frozen implementation and unchanged conformance source.
Binary checksums are in `../20261109-retry/binaries-v56.sha256`. Workspace tests,
strict Clippy, formatting and release build pass; logs are in that directory.

The initial `live.json` remains unchanged as diagnostic evidence. Its989 passing
checks and two fixture-setup failures are superseded by clean controlled/core
replays. The committee had expired, and pool lifecycle requires an initially
wallet-invisible fixture while the earlier join guard requires a visible one.
The clean replay runs enactment/pool lifecycle first on a fresh committee node,
then the funded workflows, then disconnect/reconnect. These are actual node
fixtures with independent native checks, not altered responses or expectations.
The final gate excludes that superseded report and all non-report metadata.

Original HTTP/DRep cluster: `/tmp/qz-1109-v51-JTOyGQ`.
Fresh controlled/funded cluster: `/tmp/qz-1109-fresh-8OMSBB`.
The suite's owned pending/era fixtures use their own native clusters. All owned
wallet services and the two persistent clusters were stopped after verification;
private fixture keys/databases and receipts remain in their scratch workspaces.

The exact upstream wallet revision was fetched again from cardano-foundation:
`2fc5c383c94bb907deaa0cd27b1b407f86224b6c`. Its documented minimal toolchain
rebuild reproduces a different executable digest from the rollback oracle pin;
the actual rebuilt Haskell program passes all133 independent key/root/signature
vectors. Those diagnostic oracle reports have a distinct target/environment and
are not candidate evidence or substitutes for rollback. Full provenance,
checksums and the required recovery/requalification handoff are recorded in
`../20261109-retry/ORACLE-PROVENANCE.md`. The conformance suite is unchanged.
