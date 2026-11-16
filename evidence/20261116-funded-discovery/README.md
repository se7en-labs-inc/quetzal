# Funded discovery import correction, 2026-11-16

Native attempt 6 failed the strict import boundary on a shared-wallet UTxO.
The mapper copied SQLite `seq_state_address` rows as complete addresses, but the
pinned reference persists Shelley/shared pool fingerprints as enterprise payment
addresses and reconstructs delegation credentials separately. A funded UTxO's
complete address consequently did not occur in the imported discovery set.
The reproduction shows one unowned shared UTxO and zero duplicate inputs.

The mapper now derives complete Shelley/shared addresses from persisted paths,
account keys and payment/delegation templates. It verifies network and payment
credential type/hash before lifting a fingerprint, rejects mismatched complete
addresses, and verifies the supported Shelley reward-key derivation. Shared
address construction is reused from the wallet library. Strict UTxO membership
and duplicate checks remain intact, with distinct errors and wallet context.

Verification:

- 53 migration/wallet unit and integration tests pass, including fingerprint
  lifting and negative ownership/network/delegation/duplicate cases.
- An explicit real-source regression imports/reopens all five funded wallets,
  authenticates original keys and verifies typed state: Icarus 2 UTxOs; random
  Byron, shared, Shelley and watch-only 1 each. No unowned or duplicate inputs.
- Isolated-target workspace/all-target Clippy with -D warnings passes. Formatting
  and diff checks pass. See validation.log.
- Release Nix package builds (including its test phase):
  `/nix/store/62w59fl7aw220cwgkpcdbpipylfw1p80-quetzal-0.1.0`.
  See package-build.log and native-binaries.sha256.
- Native attempt 7 uses that exact package, funds all five reference families
  through the Pallas faucet, verifies/imports canonical checkpoints and publishes
  the complete fresh destination. See native-import-receipt.json.

Full qualification remains incomplete. After successful import, attempt 7 reaches
the public API comparison and reports transaction-history differences in the
first Shelley wallet: missing validity interval/expiry fields and fractional
timestamp formatting (.8Z versus .800Z, etc.). Comparisons were not weakened.
See native-attempt7.log and history-parity-differences.tsv. Restart and spending
checks were not reached. Conformance source remains read-only.

The first local Clippy run hit stale sibling-worktree build artifacts in the
shared master/target, reporting a TxRecord without the migration field even
though the field exists in this checkout. The isolated 002/target rerun passes;
validation-shared-target.log retains the failed run. Keep targets isolated.

Private native snapshots, databases, keys and difference receipt are retained
outside Git under restart-context/20261116-funded-discovery. Owned processes were
cleaned up. No commits or pushes were made.

History follow-up (in progress): imported validity bounds/confirmed expiry now
come from hash-checked retained CBOR, with conflicting indexed expiry rejected.
Pending and newly synced records carry the same explicit bounds and required
signer hashes. API returns extra_signatures (including an empty array), omits
absent metadata, and formats UTC slot fractions without trailing zeroes. The
focused API regression, five-family stopped-source rerun and strict Clippy pass.
Initial test fixture syntax/amount encoding errors and one transient sled reopen
lock failure are retained in history-* logs; corrected reruns pass. The first
Nix history build captured the incorrect test fixture and failed; the corrected
intermediate package builds. Native attempt 8 will use the complete package.

Native attempt 8: complete history release package
`/nix/store/yd9ym5mx2aj76adlsbyy08kfls108hw6-quetzal-0.1.0` builds/tests and
imports all five freshly funded source wallets. The first strict public comparison
has one remaining difference: an unresolved input emits assets:[] instead of
omitting the unresolved source fields. See history-parity-attempt8.tsv. API
input projection now omits assets when the source is unresolved and retains []
for resolved ADA inputs; the expanded regression and strict Clippy pass. Attempt
9 will collect every failing family before reporting failure, retaining exact
object comparisons and requiring all five to match before restart/spending.

Native attempt 9 passes exact public snapshot equality for all five wallet
families, same-database restart equality, account key preservation, wrong-password
rejection, and independent node confirmation for both retained pending IDs. It
then submits a new Icarus payment successfully (202), but its immediate node
UTxO assertion runs before inclusion. The driver now polls only for absence of
the new transaction ID before applying the unchanged exact address/amount node
assertion. Existing-state and original-pending assertions remain immediate.
Attempt 10 uses the SAME release package; only this test sequencing is corrected.

## Final native result

Native attempt 10 PASSES with the same history-inputs release package:
`/nix/store/jq9jmxkrxm95pv7cciwhw343sdkj8six-quetzal-0.1.0`.
See native-attempt10.log, native-result-attempt10.json,
native-import-receipt-attempt10.json and history-inputs-native-binaries.sha256.

- Pallas faucet funds Shelley, Icarus, random Byron, shared and watch-only wallets.
- All five complete public snapshots exactly match the stopped reference.
- All five snapshots exactly match after same-database restart.
- Account extended public key survives; wrong spending password is rejected.
- Both original pending IDs independently confirm at the real node without a new
  API submission for those IDs after import.
- A new post-migration Icarus spend independently confirms, with exact node
  transaction identity, recipient address and amount checked.
- Owned candidate, faucet, reference and native-cluster processes are stopped.

Local final checks: 66 migration/wallet/primitives tests, one explicit five-family
stopped-source regression and one focused history API regression pass. Workspace
all-target strict Clippy, formatting/diff checks and Nix release build/test phase
pass. The broader feature tasks remain open for their remaining schema and
qualification scope; these results qualify this concrete five-family native run.
Private fixtures and a current source patch are preserved under Quetzal's
restart-context/20261116-funded-discovery, outside Git. Conformance source is
unchanged; its frozen external comparison/oracle code is used only by the test
driver. The faucet runtime uses Pallas and no wallet or CLI subprocesses.

## Verification after rebase onto the faucet

The migration checkpoint is rebased onto master d0d2203, which contains the faucet.
Workspace manifest and lockfile conflicts retain both crates. Combined-tree
formatting, locked migration/wallet/primitives/faucet tests, focused API regression
and workspace/all-target strict Clippy pass (rebase-validation.log).

Native attempt 11 found one exact-string difference in a random-Byron passphrase
timestamp: .14300441Z became .143004410Z. Metadata timestamp conversion now reuses
the same compact UTC fraction formatter as slot times, preserving full precision.
Nine migration tests, including source timestamp regression cases, and strict
Clippy pass (rebase-timestamp-validation.log). No comparison was relaxed.

Final combined release build/test phase passes (rebase-package-build.log):
`/nix/store/6v5chc4s2a081lw44wp1aj3cfj5pxa0r-quetzal-0.1.0`.
Native attempt 12 uses wallet, migrator AND faucet from that exact package and
passes all five complete snapshots, same-database restart, account public key,
wrong password, both original pending confirmations and new spending, all with
unchanged exact node assertions. See native-attempt12.log,
native-result-attempt12.json and rebase-native-binaries.sha256.

Reproduction uses the committed native-migration.rs with only ROOT changed to
`/tmp/qz-sqlite-migration-20261116-12`, CANDIDATE/MIGRATOR pinned to the package
above, and FAUCET set to its bin/quetzal-faucet. Run from a private working directory
containing evidence/20261110-sqlite-migration so its result file stays outside Git.
The exact adapted source, binary, release GC root and private stopped fixtures
are retained under Quetzal restart-context/20261116-funded-discovery. All owned
processes are stopped. No conformance source changes, merge or push are performed.
