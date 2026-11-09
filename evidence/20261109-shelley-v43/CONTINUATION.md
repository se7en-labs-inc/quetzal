# Quetzal continuation — V43 aggregate, V45 delegation work

All643 obligations remain required, Shelley before Byron. The user authorizes
autonomous continuation and forbids changing the conformance suite or cheating.
Preserve all dirty rename/implementation work. No staging, commits or pushes.

Frozen V43 aggregate covers **567/643**, exhaustivefalse. Complete nine HTTP
suites508/537, sixteen internal profiles1241/1241, expanded funded Shelley
replay709/709. Gate inputs were explicit catalogue entries plus live.json;
startup probes and http-failures-summary.json are not conformance reports.
All reports, including prior failures, remain preserved. Workspace tests,
format, strict Clippy and release pass. V30 previous aggregate was508/643.

V43 implements real pool parameters/current-future changes, retirements,
non-myopic rewards and ratios from one acquired ledger state. Block-production
counts are persisted and reversible. Metadata is size bounded, hash authenticated
and validated, direct/SMASH modes supported; real SMASH delisting GC is persisted.
Metadata/GC/count/metric tests pass. Query source is the exact Nix-resolved
ouroboros-consensus4.2.1.0 archive, copied to /tmp/quetzal-consensus-4.2.1.0.tar.gz,
extracted /tmp/quetzal-consensus-source; digest in ../20261109-pools/.

V43 had one substantive pool differential mismatch: it listed genesis-only
pools. The pinned reference lists only pools with observed registration
certificates (Cardano.Wallet.Pools.combineDbAndLsqData drops LSQ-only pools).
V44 corrects this with journalled registration counts; replay/rollback and old
unreleased record-format tests pass. The network suite becomes13/14, only DRep
summary fails. Do not reinstate genesis-only listing to satisfy a guard fixture.
The supplied prepare-pool-fixture provides real registrations before pool tests.

Current frozen V45 replaces synthetic pool joining with real constructors,
certificates, stake witnesses, deposits, history and submission. Removed the old
synthetic join_pool/quit_pool methods. Constructor supports join/quit and vote
combination. Initial legacy Conway joins default to abstention per pinned source.
Pool activation is scheduled two epochs ahead, with actual timestamps and combined
voting/delegating states. Rewards are read from the node with cancellation-safe
one-shot ownership and a500ms optional cache refresh on wallet reads. Reward cache
updates preserve concurrent metadata/credentials. Wallet totals include rewards.
Self withdrawals and retirement residual rewards use actual ledger balances,
stake witnesses, value accounting and pending/confirmed/decode history. Explicit
self withdrawal applies the source146-byte conservative cost/twice-cost policy.
External reward-account withdrawal remains explicitly unsupported.

V45 real delegation/rewards/refund plus voting replay is running in
../20261109-pools/live-delegation-v45.{json,log}, timeout240 for actual reward epochs.
Do not claim it passes until inspected. Workspace22-wallet-test generation,
strict Clippy and release pass; targeted live node query passes. V45 source
has not been changed after freezing at this checkpoint.

Active fresh cluster/reference/V43 candidate RUN /tmp/qz-1109-v43-WNE48i,
marker /tmp/quetzal-v43-workspace: node.socket, candidate19092, reference19093,
UI19113, faucet19189. Cluster session94572, candidate29930, reference68447.
All V30 services and V42 candidate stopped after checking exact PID cmdlines.
V45 focused candidate19094 RUN /tmp/qz-1109-v45-6QWM5M (resolve marker
/tmp/quetzal-v45-workspace), independent DB, uses V43 node; session36263.
V44 candidate stopped. Actual pool fixture receipt:
/tmp/qz-1109-v44-LbBuIq/pool-fixture-1/public/receipt.json,
pool1as50x0wtumtyqzs7tceeh5ry0syh8jnvpnuu9wlxswxuv48sw4w.
Fixture preparation passed and registration confirmed on the node. Preserve
private keys under RUN; never print them. Future full fixed-fee replay needs
another fresh cluster. No HTTP wallet-list suites during funded workflows.

Remaining major domains: governance real DRep state/summary cache, complete pool
lifecycle controls, external/script balancing, durable retry/restart/rollback and
era/parameter controls, later Byron payment/selection/migration compatibility,
remaining exact HTTP errors and shared async-progress differential race. Trusted
Haskell key-oracle executable digest remains unreproduced; never bypass it.
Use Nix devshell; never enumerate /nix/store. Exact pins in prior checkpoints.
