# V57 rollback validation and full replay

The externally updated conformance runner now records the actual Haskell helper
identity and verifies it remains stable during derivation. No suite files were
modified by this implementation session. `suite-owner-changes.patch` preserves
all externally supplied suite changes observed before rebuilding the runner.

The actual upstream Haskell helper is built against wallet revision
2fc5c383c94bb907deaa0cd27b1b407f86224b6c. Its executable SHA256 is
337e2b28428a1a241ff6d27bae025069c619fb5a5836c55d25b4ae4ed2122993.

V56 reached the actual native branch switch and passed123/124 checks, then
failed `wallet chain source did not switch to exact B native header`. Its report
is preserved as `rollback.json`. Per-wallet workers had reached B while the
separate global reader was waiting for its five-second reconnect. The public
latest-header endpoint returned that reader's stale A cache.

V57 reads native chain point and block number within one acquired local-state
snapshot, releases it, and closes its cancellation-safe owned connection. It
therefore reports a coherent current native header independently of background
pool-history replay progress. No timing delay or special test behavior was added.

`rollback-v57.json`: **224/224 passing checks**, including all four required
wallet.rollback cases, distinct confirmed/discovered/pending rollback histories,
native input/balance observations, reapplication, final native verification and
owned service cleanup. `binaries-v57.sha256` records the frozen candidate and
adapter. All workspace tests, strict all-target Clippy, release build and fmt
check pass. Standalone16-profile internal replay:1241/1241, reports in
../20261109-v57-gate/.

Full suite-owned37-profile `conform` replay is running in the private workspace
/tmp/qz-1109-v57-S8XIoW/conformance. `execution-config-v57.json` records its public
configuration and independently trusted executable identities. Log:
`conform-v57.log`. One asynchronous differential failure so far:
family-shared-pending-delegation-patch, `ignored reference field is absent`.
All other HTTP profiles pass; funded controls are still running.

New suite fingerprint:
96877229916f14ab6ad549a2a1b4374c061d6216b4d124f617b9cac40beb9df6.
The prior V56 gate642/643 uses a different suite fingerprint and candidate;
its reports must not be mixed with V57 to claim a complete current gate.

Full attempt1 completed all37 profiles:3723 checks,3721 passing,2 failing;
638/643 catalogue obligations, unchanged binary identities and no setup error.
Complete reports/result/coverage/attempts/identities are in full-attempt-1/.
Rollback224/224 and era49/49 also pass inside that complete attempt.
The other failure is pending:workflow, family transaction confirmation timeout.
Independent native pool1 remained at block72/slot311 and logged repeated
Forge.Loop.NoLedgerView after resuming; public forging evidence is retained in
full-attempt-1/native-retry-forging.log. Its recorded guard had current_slot320,
upper_slot359, stable tip311, exclusive_horizon371. No native state was altered
to force progress. The unchanged full suite is now repeating with fresh fixtures
in /tmp/qz-1109-v57-S8XIoW/conformance-retry (session88299); no reports imported.

The shared activation mismatch recurred in full attempt2. Isolated unfunded
instances captured actual response projections (shared-activation-v57-*.json):
Quetzal returned syncing/progress0 at genesis; Haskell returned ready without
progress at the same genesis checkpoint. Exact upstream source explains this:
Primitive.SyncProgress.syncProgress compares checkpoint relative time with wall
time using CLI syncToleranceOption's300-second default. It does not require
exact tip equality. This is a backend compatibility gap, not a discarded flake.

V58 implements the reference public progress rule without modifying internal
worker readiness, checkpoints or balances. Tests cover the exact300-second
boundary, meaningful nonzero progress, future coverage and mainnet Byron/fork
slot timing. Workspace tests, strict all-target Clippy, fmt and release pass.
One test-only missing u64 annotation was corrected; initial compile diagnostics
are retained. Frozen candidate/adapter identities: binaries-v58.sha256.
Full unchanged37-profile conformance runs in /tmp/qz-1109-v58-wYGG6p/conformance,
session18437, target19099/v2/, reference19100, faucet19192, environmentquetzal-v58.
The V57 repeat remains separately owned on19097/19098/19191 (session88299).
No evidence is mixed across candidates, ports, environments or fingerprints.

V58 full-run HTTP families now102/102 PASS, including the exact previously failing
shared activation patch. T189 is verified. No fixture, expectation, response
comparison or ignore rule was changed.

V57 full attempt2 finished all37 reports:3806 checks,3805 passing,1 failing,
640/643 obligations. Only shared activation fails; every native profile passes,
including retry108/108, rollback224/224 and era49/49. Identities unchanged, no
setup/execution error. Full receipts in full-attempt-2/. Owned services cleaned
up by the runner. This confirms the earlier retry nonadvancement did not recur
with the exact same candidate and unchanged suite. V58 has fixed shared activation
and remains under its own full conformance run; no V57 reports will enter its gate.

FINAL: V58 full conform exit0;37 fresh reports,3807/3807 checks,643/643 obligations,
all attempts passed,identities unchanged,no setup/execution error. Native rollback
224/224 and era49/49 pass within that same invocation. Canonical public evidence
and complete validation are in ../20261109-v58-gate/. All owned services stopped;
final process check found no matching candidate/reference/node/factory/runner.
