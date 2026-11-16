# Migration verification, 2026-11-10

Latest update: `../20261116-funded-discovery/README.md` records native attempt 10
passing exact public API parity for all five families, same-database restart,
original pending confirmation and new spending. Funded discovery and historical
validity/expiry/timestamp corrections resolve the earlier failures below. Broader
schema and qualification tasks remain open in the feature checklist.

Update 2026-11-16: the migration fixture now uses the standalone Quetzal Pallas
faucet implemented in sibling `003-development-faucet`. Its service has no
cardano-wallet or cardano-cli dependency. Extended Ed25519 keys, matching Byron
funding sources and bootstrap witnesses are supported. Native attempt 6 accepted
four 30 ADA funding transactions and verified funded balances for all five wallet
families. It then reached migration and failed with
`invalid imported wallet: unowned or duplicate UTxO`. Funding is now verified;
full migration qualification remains incomplete. See native-attempt6.log and
the faucet feature's `evidence/20261116-development-faucet` for funding receipts,
key/source tests and intermediate failures. The comparison harness still uses
the pinned reference wallet and external conformance tools for test provenance.
The older pending CLI-wrapper proposal below is superseded by this implementation.

Feature worktree: `/home/sam/work/se7en/quetzal/002-sqlite-state-migration`.
No changes were made to cardano-wallet-conformance. These records describe the
checks performed before the migration checkpoint commit; no push was performed.

Implemented CLI: `inspect`, `check` (complete temporary conversion and sled
reopening), and `import` (verified fresh-directory publication).

Completed checks:

- Workspace tests: 108 passed, 0 failed, 2 explicitly ignored external-fixture
  tests. See `workspace-tests.log`.
- Seven migration source/publication/credential/history tests pass; one native
  source test is opt-in. Latest mapper changes have the focused rerun in
  `mapping-tests.log`.
- The explicit opt-in test converted all five stopped reference-created SQLite
  wallets, authenticated their original encrypted roots, rejected wrong
  passwords and verified reopened sled metadata, keys, addresses, UTxOs,
  history and pending rows. See `stopped-reference-files.log`.
- Strict workspace/all-target Clippy passes (`clippy.log`). Cargo formatting and
  `git diff --check` pass.
- Release Nix package builds, including its test phase (`package-build.log`).
  Exact final package: `/nix/store/18m7fxh3chkm74z165n88d5z65mng169-quetzal-0.1.0`.
  Its migrator inspected the five reference wallets with `PATH=/nonexistent`,
  proving SQLite is carried by the package closure. The result is
  `packaged-reference-inspection.json`. Binary hashes are in
  `current-binaries.sha256`.

Native end-to-end qualification is **not passed**. Both attempts stop before
invoking migration: the faucet's cardano-cli rejects the reference API's
bech32-encoded Icarus/Byron address. The first diagnosis incorrectly identified
the shared address. Attempt 2 adds the user-approved explicit single-cosigner
shared delegation template, but fails at funding Icarus, the second destination.
The errors are retained in `native-attempt1.log` and `native-attempt2.log`.

On 2026-11-16 the user approved converting only fixture faucet destinations to
equivalent Base58 Byron representations. The driver now performs this conversion
while keeping all five wallet families and full public state/restart/spending
comparisons. It rebuilds successfully against the frozen conformance helpers.
Attempt 3 still failed at the faucet with HTTP 500 (`native-attempt3.log`);
Diagnosis after restart: the pinned upstream local-cluster faucet's
`lib/local-cluster/lib/Cardano/Wallet/Launch/Cluster/Faucet.hs:300` unconditionally
uses `CA.bech32 addr` to render every `--tx-out`. This re-encodes even a Base58
request as bech32 before invoking cardano-cli. Attempt 3's cluster log shows
Shelley funding succeeded, then the Icarus funding command failed on that bech32
address. Request-side conversion cannot fix this boundary. Native qualification
remains incomplete. User approval has been requested for a fixture-local CLI
wrapper that normalizes only Byron output addresses before forwarding to the
unchanged pinned CLI, without modifying conformance or weakening assertions.
Private fixtures and the corrected binary are preserved outside Git under
`/home/sam/work/se7en/quetzal/restart-context/20261116-faucet-fix`.

The owned private fixture roots remain outside Git:
`/tmp/qz-sqlite-migration-20261110` and
`/tmp/qz-sqlite-migration-20261110-2`. Their cleanup guards stopped the reference
wallet and owned nodes after failure. Mnemonics, SQLite/sled stores and keys are
not included in this evidence directory. A new approved attempt needs a fresh
root and rebuilt driver; debug wallet/migrator binaries now contain the mapper
and sled account-key fixes made after attempt 2.

Remaining qualification includes the full native migration/public API comparison,
pending resubmission and spending after restart, an independent native
rollback/intersection regression, and any mapping/API corrections those expose.
Legacy credential coverage here includes synthetic persisted V1 layouts as well
as genuine reference-created V2 files; it does not claim native legacy-source
qualification. Original retained historical CBOR is preserved, but mint/burn API
projection remains an existing gap requiring review before claiming complete
history parity for such transactions.
