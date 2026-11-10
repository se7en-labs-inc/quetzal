# Read-only runner handoff

The frozen updated runner's `conform` command aborted with `File exists (os error 17)` before HTTP/live orchestration. Its result is retained in [full-aborted/result.json](full-aborted/result.json), with complete=false and identities unchanged. No conformance source or assertions were edited.

In `/home/sam/work/se7en/cardano-wallet-conformance/master/src/executable.rs`, lines 968–978 save six placeholder acceptance reports. `Execution::acceptance` at line 526 then saves an actual report to the same path. The local `save` helper at line 105 delegates to `era_fixture::save`, whose `OpenOptions` uses `create_new(true)` (`src/era_fixture.rs:35–39`). The first replacement therefore fails. An existing `rewrite` helper already provides replacement semantics; the suite owner can use it when replacing these intentionally precreated reports and add an orchestration regression.

The 39 failed checks in the aborted cohort are unexecuted placeholder requirements, not executed candidate failures. The 16 internal profiles passed. The aborted cohort is not complete and must not be patched together with reports from other runs.

The user authorized running unchanged public profiles individually. Those reports are retained separately under `individual/`; the isolated `verify-replacement` result is under `replacement/`. Missing user-deferred mainnet/lagging/ahead/representative-history fixtures remain explicit qualification gaps.
