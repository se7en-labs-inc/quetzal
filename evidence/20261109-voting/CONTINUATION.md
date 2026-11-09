# Quetzal continuation — V42, 2026-11-09

All643 obligations remain required. User authorized autonomous continuation;
never change the supplied conformance suite, fabricate successful operations,
stage, commit or push. Existing large dirty rename/implementation is preserved.

Latest complete aggregate is still frozen V30:508/643, exhaustivefalse.
Do not mix generations to claim increased aggregate coverage. New focused proofs:

- V35 complete internal catalogue1241/1241.
- V36 stake keys match actual pinned reference, including funded stake.
- V40 funded assets and precedence162/162: real mint/burn, node rejection of
  corrupt policy witness, balances/assets/UTxO snapshots, random Byron and
  Icarus asset receipts. Extended HTTP75/85; all new address/import/stake/asset
  cases pass. Reports in ../20261109-byron-addresses/.
- V41 complete internal catalogue1241/1241 in ../20261109-internals-v41/.
  Its adapter SHA256 equals V42's adapter (see binaries-v41/v42.sha256).
- V42 actual voting/register/change/duplicate rejection/deregister/refund and
  independent node checks100/100. V41 prior attempt168/169 (empty quit error
  precedence) retained; V42 fixes registration-before-funds retirement guard.

Production changes include source-grounded native mint supply accounting,
policy scripts and CIP1855 signatures; plain outputs use compact legacy arrays
and zero-ADA outputs receive the conservative minimum. Actual node rejection
is propagated instead of local invalid-signature403. Absent asset metadata is
omitted. Byron random generation and batch import are durable/atomic, password
and payload authenticated; duplicate indices serialized. Assets/history reuse
real family storage. Existing old prototype Byron data migration is not done.

Staking uses real Conway certificates from writer.rs, actual node registration
and deposit queries, stake witnesses, and actual pending/confirmed history.
Stake state lives in its own journalled Sled tree, preserving concurrent credential
and name changes through rollback. Rollback test passes. Node deposit query needs
tag258 and script-before-key canonical credential order. All failed encodings
and compile/test attempts retained. Workspace tests, strict Clippy, format and
release pass V41; V42 is one guard-precedence correction, with release/live passing.

Next: replace empty pool catalogue/no-op refresh with actual node data and
accurate metrics, metadata lifecycle and retirement. Actual pool joining still
uses an old synthetic method and must be replaced. Withdrawals, script/external
balancing, retry/restart/rollback controls, later Byron payments/migrations and
trusted Haskell key-oracle binary digest remain unfinished. Never bypass provenance.

Active private cluster/reference remain /tmp/qz-1109-v30-xFoT0R (node.socket,
candidate19092/reference19093/UI19113/faucet19089). Focused V42 candidate19094
RUN /tmp/qz-1109-v42-EQt5tN, marker /tmp/quetzal-v42-workspace, owns independent
database; uses V30 node. V36–V41 candidate processes were stopped after exact
PID command-line ownership verification. Preserve all evidence/databases.
Fresh cluster required before repeating fixed delegation-fee fixture/full gate.

Never enumerate /nix/store; resolve exact source/package outputs through Nix.
Build/test via project Nix devshell. Worktree /home/sam/work/se7en/quetzal/wallet-migration.
