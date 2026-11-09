# Quetzal V58 — all 643 conformance obligations met

The unchanged suite-owned `conform` entrypoint completed successfully with exit 0:
**643/643 obligations, 3,807 checks, zero failures, 37 fresh profiles.** Every
attempted component passed, executable identities stayed unchanged, and there
was no setup or execution error.

- [Overall result](result.json), [strict catalogue gate](coverage.json)
- [All 37 reports](reports/), [execution attempts](attempts.json), [log](conform.log)
- [Binary/config identities](identities.json), [execution configuration](execution-config-v58.json)
- [Validated specification export](validated-specification.json)
- [Standard cleanup](standard-cleanup.json), [committee cleanup](committee-cleanup.json)

| Profile group | Profiles | Passing checks |
| --- | ---: | ---: |
| Internal vectors |16|1241|
| Differential HTTP against Haskell |9|537|
| Funded wallet operations/restart |1|947|
| Node disconnect/reconnect |1|82|
| Actual registered-DRep lookup |1|6|
| Pool lifecycle |1|120|
| Actual parameter enactment |1|149|
| Normal pending/deletion |1|56|
| Expiry/mixed history |1|79|
| Both Byron pending/deletion styles |1|180|
| Automatic retry across three wallet styles |1|108|
| Checkpoint recovery |1|29|
| Native fork rollback/reapplication |1|224|
| Babbage-to-Conway transition |1|49|
| **Total** |**37**|**3807**|

V57 fixed stale latest-header responses after a native branch switch by querying
point and block number within one acquired node snapshot. V58 also matches the
reference's 300-second public sync tolerance and elapsed-time progress. Internal
worker readiness, checkpoints and balances retain their actual ledger state.
Boundary tests cover tolerance edges and mainnet Byron/fork timing. Workspace
tests, strict all-target Clippy, formatting and release build pass; logs are in
../20261109-rollback-retry/ (tests-v58-fixed.log, clippy-v58-fixed.log).

Candidate and adapter are frozen in /tmp/qz-1109-v58-wYGG6p; hashes are in
binaries-v58.sha256 and source hashes in sources-v58.sha256. Target is
http://127.0.0.1:19099/v2/, environment is quetzal-v58, and suite fingerprint is
96877229916f14ab6ad549a2a1b4374c061d6216b4d124f617b9cac40beb9df6.
All evidence in this gate comes from this single full invocation.

The suite owner externally repaired compiler-dependent reference identity
handling before this session's retry. We did not modify the conformance suite;
suite-source-audit.txt verifies its complete diff still matches the owner's
supplied patch. The actual Haskell key helper uses upstream wallet revision
2fc5c383c94bb907deaa0cd27b1b407f86224b6c and records its stable executable identity.

Earlier incomplete attempts remain in ../20261109-rollback-retry/: V56 exposed
stale headers; V57 exposed the public sync-tolerance gap. One V57 retry fixture
logged native NoLedgerView and stopped advancing; the identical candidate's next
full run passed retry 108/108. V58's complete run passes all controls and cleanup.
No previous-candidate reports were imported. All owned services are stopped;
the final process check found no Quetzal, wallet, node, factory or runner process.
