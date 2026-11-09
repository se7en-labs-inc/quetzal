# Quetzal continuation — 2026-11-09, V35

All 643 obligations remain required, Shelley before Byron. The user explicitly
authorized autonomous continuation and forbids changes to the conformance suite.
Preserve existing dirty changes; nothing staged, committed or pushed.

V30 frozen complete replay covers 508/643 obligations, exhaustive false:
488/537 HTTP, 1050/1241 internal and 576/582 expanded funded checks. Reports are
in `../20261109-shelley-v30/`. No newer aggregate result has been claimed.

V35 complete sixteen-profile internal replay passes **1241/1241**. Production
credential migration/rotation now uses atomic Sled changes and actual KDF fault
injection; the seeded selector ports pinned SplitMix/random and Haskell change
allocation; wallet selection adds collateral and mint/burn value accounting.
The adapter calls these real wallet methods. Earlier failing attempts remain
preserved. Workspace tests, strict Clippy, formatting and release build pass.
Frozen adapter `/tmp/quetzal-adapter-v35`; digest in `adapter.sha256`.

Next source generation replaces the malformed stake-key stub with derived keys,
UTxO grouping and actual node rewards/delegation queries. Focused evidence is
in `../20261109-stake-keys/`. This does not complete staking/voting transactions.
Remaining work includes actual pool lifecycle queries/metadata, staking/voting,
withdrawals, HTTP mint/burn, script/external-input balancing, durable submission
and rollback controls, and later Byron compatibility. A trusted Haskell key
oracle executable still does not reproduce its required digest; preserve all
harness provenance checks. No successful stub is evidence of ledger behavior.

Current owned V30 cluster/reference/candidate: `/tmp/qz-1109-v30-xFoT0R`, ports
19092/19093, UI19113, faucet19089. Marker `/tmp/quetzal-v30-workspace`.
V36 focused candidate port19094 uses that node with its own database; marker
`/tmp/quetzal-v36-workspace`. Verify PID cmdlines against the exact run directory
before stopping a process. All previous generations have been stopped after
ownership checks; evidence and databases remain intact. Use a fresh cluster
before repeating the fixed funded delegation-fee fixture or a full clean replay.

Exact Nix binary pins and source inventory remain in prior continuation records.
Never enumerate `/nix/store`. Build/test via the project Nix dev shell. Worktree
is `/home/sam/work/se7en/quetzal/wallet-migration`, branch001-wallet-rust-backend.
