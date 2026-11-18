# Tasks: Responsive runtime

Implementation is in progress. Document review is not implementation.
Dependencies are sequential phases unless a task states otherwise. Reuse existing
feature 002/004 artifacts and engine; no subagent work is authorized by this list.

## Phase 1 — Baseline and dependency gates

- [x] T001 Inventory every live `WalletDb` read/write, expensive computation,
  node wait, accepted input size, error mapping and lock lifetime in API/wallet
  modules; record typed command coverage, permitted overload errors, bounded
  oversized-job policy and submission rejection/retry semantics in `research.md`
  and `contracts/runtime.md`. Confirm sled atomicity/read-fence feasibility.
- [ ] T002 Build deterministic Rust fixtures and the opt-in
  `crates/quetzal-api/src/bin/runtime-bench.rs` driver using existing dependencies;
  independently check fixture balances/reservations/order. Capture exact host,
  baseline source/build, workload and raw latency/resource evidence as specified
  in `contracts/performance.md` before changing runtime execution.
  Progress: optional driver and all-family deterministic scale fixtures are
  implemented/tested; source/executable identities frozen; two node-unavailable
  smoke runs validate routes/results. Full quiet-host/native baseline pending.
- [x] T003 Review feature 004's completed implementation/evidence and integrate
  its reviewed result into this branch when available. Record exact source
  identity and any intermediate performance control. Do not alter its active
  worktree. Required before T023; US1/US2 design can proceed independently.
  Integrated the complete reviewed ten-file source patch, SHA-256
  `a80dcf81ab8cd3d439d04afb22ed8eff1307ef0fcc3e6886c26ffe506c4e7d96`,
  based on master 84f2f4d. Retained feature 004 specifications. Its original native
  evidence remains in its worktree; combined runtime qualification is still pending.

## Phase 2 — Shared storage and execution foundations

- [ ] T004 Add namespace, generation/revision, operation and typed result wrappers
  in `crates/quetzal-wallet/src/storage/mod.rs`; define command/query inventory
  from T001 and expose clients without raw live mutation handles.
- [ ] T005 Implement item/byte permits, in-flight accounting, admission deadlines,
  producer concurrency, cancellation and oversized-job rules in `storage/mod.rs`
  and `compute.rs`; test both payload and response/backlog accounting.
  Progress: standalone owned compute pool and retained-result budgets are
  implemented; admission, cancellation, panic and drain/join tests pass. Storage
  queues and API ingress integration remain pending.
- [ ] T006 Implement fixed modern/Byron writer threads, separate API/sync lanes,
  causal barriers and weighted fair scheduling in `storage/worker.rs`. Verify
  bounded thread counts and that both lanes advance under sustained demand.
- [ ] T007 Implement atomic typed domain changes in `db/`, durable generations,
  revision revalidation and flush/ack barriers; cover creation/deletion,
  reservations/allocations, submission provenance and chain undo/checkpoints.
- [ ] T008 Implement `crates/quetzal-api/src/runtime.rs` ownership/supervision,
  dedicated API and sync runtimes and bounded compute/read pools; integrate
  bootstrap readiness/error propagation in `main.rs` without detached threads.
- [ ] T009 Add fault/concurrency tests in wallet storage modules and
  `crates/quetzal-api/tests/runtime_lifecycle.rs` for wrong-thread work, queue
  closure, panic, partial startup, ordering and flush failure. Complete before
  moving callers; follow the user's failure-recovery agreement if checks fail.

## Phase 3 — US1: Responsive committed reads

- [ ] T010 [US1] Implement versioned wallet summary/catalog projections in
  `storage/view.rs` and `db/`; publish coherent revisions after commit and rebuild
  on existing-store reopen. Include assets, pending/change and namespace identity.
- [ ] T011 [US1] Implement indexed bounded reads with revision consistency in
  `storage/query.rs` and `db/`; retain existing filters/order and isolate complete
  response scans. Bound retries and preserve existing accepted API semantics.
- [ ] T012 [US1] Route summary/list/history/address/UTxO reads and public API
  projections through storage clients in `wallet.rs` and `handlers/`; remove
  synchronous DB/CPU work from HTTP executor paths across all families.
- [ ] T013 [US1] Move native observation/parameter refresh to sync services in
  `runtime.rs`, `workers.rs` and `state.rs`; make network information use shared
  last-native observations and current-time progress without a request query.
  Preserve cold start/disconnect/rollback behavior and parameter freshness.
  Progress: native tip/clock observation now runs independently of historical
  replay; network information consumes the last observation. Parameter/header
  request paths and the complete owned sync-service boundary remain pending.
- [ ] T014 [US1] Test delayed writer, active restore, network outage and expensive
  concurrent jobs in `crates/quetzal-api/tests/runtime_reads.rs`; verify latency,
  response correctness, read-after-write and snapshot revision consistency.

## Phase 4 — US2: Safe mutations and sync under pressure

- [ ] T015 [US2] Move derivation, passwords, transaction preparation and block
  discovery into bounded `compute.rs` jobs; zeroize owned secrets and return
  revision-qualified plans. Add crypto/serialization property and parity tests.
- [ ] T016 [US2] Route API wallet/shared/Byron/key/address/pending/migration-API
  mutations through storage commands in wallet modules and `handlers/`; remove
  live direct writes and revalidate prepared reservations/allocations at commit.
  The existing wallet-migration API remains distinct from SQLite startup import.
- [ ] T017 [US2] Route chain intersection/forward/rollback through acknowledged
  ordered commands in `sync.rs` and `workers.rs`; preserve per-wallet restoration
  cursors, imported rollback floor, discovery and local submission provenance.
  Depends on T034–T036 for the shared replay/discovery model; do not merely move
  the existing independent per-wallet block loops to another executor.
  Progress: independent block loops have been replaced by the shared service,
  including pool history. Chain commits still use the existing atomic DB methods;
  typed storage commands and writer-lane admission remain pending.
- [ ] T018 [US2] Route pools/settings/rewards/metadata/GC and retry mutations
  through storage clients; move background tasks onto owned sync/compute domains
  in `main.rs`, `workers.rs`, wallet modules and associated handlers. Audit T001
  inventory for zero bypasses and no lock held across external waits.
- [ ] T019 [US2] Implement supervised shutdown in `runtime.rs`, `main.rs` and
  `shutdown.rs`: stop admission/producers, drain accepted changes, flush, join
  all domains and propagate errors within the existing budget under valid load.
- [ ] T020 [US2] Add adversarial concurrent mutation/property tests in wallet
  storage modules and API tests: duplicate spends/change allocations, stale plans,
  deletion/recreation, cross-lane rollbacks and starvation. Independently compare
  canonical state and public views after every operation sequence.
- [ ] T021 [US2] Add subprocess crash/restart and EOF tests in
  `crates/quetzal-api/tests/runtime_recovery.rs`; kill between admission, commit,
  flush, publication and reply. Verify all acknowledged state survives, uncertain
  outcomes are handled safely, and no pending work or owned threads leak.
- [ ] T022 [US2] Exercise native replay/intersection, deep imported rollback,
  pending confirmation, disconnect/reconnect and concurrent submission against
  owned nodes; retain exact source identities and public evidence.

## Phase 5 — US3: Dedicated and automatic migration integration

- [ ] T023 [US3] After T003 and T010/T019, wire reviewed feature 004 layout/import
  selection before live database ownership, view rebuild and listener binding in
  `main.rs`/`runtime.rs`; preserve temporary/fresh/direct/managed startup modes.
  Progress: existing startup integration is present and runs before listeners
  and wallet workers. Wiring to the new owned runtime/views waits for T010/T019.
- [ ] T024 [US3] Verify standalone `inspect/check/import` and shared startup
  engine remain packaged via existing Nix definitions and `quetzal-migrate`;
  document lifecycle order and automatic behavior in `docs/sqlite-migration.md`.
- [ ] T025 [US3] Run startup integration tests with the new execution model:
  missing state, SQLite directory, pool-only, corrupt/unsupported source,
  conflicts/symlinks, interrupted/concurrent publication, restart without source,
  and no API listener on failure. Verify source hashes and completed destination.
- [ ] T026 [US3] Qualify packaged `serve --database DIR` on funded native SQLite
  fixtures for all five families: exact public snapshots, original credentials,
  pending confirmation, same-state restart and later spending checked at node.
  Confirm committed views initialize from imported state without a rescan.

## Phase 6 — US4: Diagnosis and final qualification

- [ ] T027 [US4] Instrument request/queue/byte/compute/commit/flush/view/sync and
  lifecycle timings in API/storage/compute workers; add fault observations that
  identify each delayed domain and verify no secrets or unbounded label sets.
- [ ] T028 [US4] Document thread ownership, queue budgets, overload/uncertain
  results, startup/shutdown and measurement interpretation in
  `docs/runtime-architecture.md`; include scope and tuning evidence.
- [ ] T029 Run project Nix formatting, warning-free build, strict all-target
  Clippy, unit/property and native integration gates from `quickstart.md`; retain
  outcomes. Do not substitute unit tests for native correctness checks.
- [ ] T030 Run unchanged independent API conformance with source/suite identities;
  verify mutual TLS, EOF, error bodies and network progress behavior without
  editing the external suite or user-owned files.
- [ ] T031 Run all repeated same-host performance/fault/overload scenarios in
  `contracts/performance.md`; verify SC-001–004 per endpoint and trial alongside
  SC-005/006 correctness. Record misses explicitly and ask before another approach.
- [ ] T032 Review FR-001–016 and SC-001–007 against evidence, update documentation
  and task status honestly, confirm CI before merge, and retain public receipts
  under `evidence/<date>-responsive-runtime/`. No automatic commit, push or merge.

## Preview diagnostic integration

- [x] T033 [US1] Exercise direct HTTP responsiveness against a real Preview node
  bootstrapped from a verified Mithril snapshot, without Daedalus. Extend the
  optional Rust driver with a read-only probe using independent per-request
  deadlines and incremental receipts. Cover network information/parameters,
  latest header, settings, wallet lists and per-wallet summary/address/UTxO/history
  while restoration progresses. Record native node and wallet tips separately,
  HTTP failures/timeouts and candidate/config/snapshot identities. Use isolated
  node/wallet state; do not reuse synthetic scale checkpoints on Preview. Report
  the actual synchronization and wallet-history scope without claiming full
  performance qualification. User requested and authorized running an isolated
  node, then specified Mithril bootstrap on 2026-11-18.
  Evidence: `evidence/20261118-preview-api/`; 3,120 reads across five empty
  families during early restoration, zero failures/deadlines, max 16.316 ms.
  Startup/reconnect probe separately retains six HTTP 500s. No funded history,
  complete restoration, overload or latency qualification is established.

## Shared following and bounded restoration

- [ ] T034 [US1] After T004/T008, implement fixed live/catch-up scheduling and
  namespace/generation-qualified replay cohorts in `workers.rs` and wallet sync
  services. Share fetching/decoding for cohort ranges; bound cohort/cache bytes
  and joining-wallet admission. Preserve individual cursors, live following,
  fair catch-up, cancellation and rollback ordering.
  Progress: owned sync thread, one live and one catch-up stream, fair quanta,
  independent cursors, bounded consumer/cohort counts and shared block decode are
  implemented. This focused Preview repair precedes the complete T004/T008
  boundary; byte accounting, durable generation wrappers and overload integration
  remain pending. See `docs/shared-sync.md` and native diagnostic evidence.
- [ ] T035 [US1] After T004/T007, implement resident discovery state
  and address-byte/outpoint ownership indexes in wallet `discovery.rs`/`sync.rs`.
  Load canonical state at registration; update incrementally for gap expansion,
  receipts/spends and allocations. Preserve all families, overlapping owners,
  namespace isolation, imported state and generation/revision invalidation.
  Track historical range coverage and bounded backfill where discovery semantics
  require it; readiness cannot outrun unsearched newly eligible history.
  Progress: retained canonical address-byte ownership, discovery pools and UTxO
  maps now update incrementally. External mutations invalidate their cache epoch.
  Historical range coverage/backfill and durable revision integration are pending.
- [ ] T036 [US2] After T006/T007/T015/T034/T035, implement bounded staged block
  matching and ordered replay commit batches in `sync.rs` and storage workers.
  Reach same-block gap-discovery closure before ledger-order effects, avoid
  reconstructing full wallet state per block, and preserve per-block rollback,
  durable local mutations, checkpoint recovery and explicit rollback retention.
  Progress: fixed-point same-block matching precedes ledger-order changes;
  relevant effects commit per block, empty progress and pool history use bounded
  256-block batches. Retained matcher state avoids full DB reload per block.
  Full storage admission/byte budgets and writer isolation remain pending.
- [ ] T037 [US2] Test shared decode/fetch counts, late wallet joins, overlapping
  ownership, cross-namespace IDs, same-block reversed gap triggers, same-block
  receipt/spend, multi-step gap extension, rollback/restart, deletion/recreation
  and allocation races. Compare optimized results with an independent reference
  replay. Bound memory and prove live followers progress during catch-up.
  Progress: native-protocol initial intersection/rollback test proves one decode
  serves two wallets. Wallet tests cover reversed gap triggers, same-block spend,
  rollback/replay, cache invalidation and checkpoint recovery. Shared stalled
  handshake cancellation/join passes. Native restart and late-join diagnostics
  are recorded separately; the complete adversarial/reference matrix is pending.
  Native late join: fresh watch-only genesis-to-ready within 379.3 seconds,
  with existing funded wallet live throughout. First 4,080 concurrent diagnostic
  reads all passed, max 28.842 ms. This is not the full T038 qualification.
- [ ] T038 [US1] After T017/T020/T021/T037, qualify SC-007 on the recorded Preview
  snapshot and quiet host with five fresh families, independently funded receipt,
  read workload, late join and native rollback/restart. Record full restoration
  wall time, index build, fetch/decode/match/commit rates and bytes written.
  Report the proposed 15-minute target honestly; early empty-wallet smoke results
  and the existing background funding monitor cannot qualify it.

## Dependencies and coverage

Phase 1 inventory/baseline precede implementation; foundations precede callers.
US1 and US2 use the same storage ordering/publication contract and converge before
qualification. T023 requires the integrated feature 004, not a copied partial
library. Final qualification requires every story's correctness tests.
T034–T038 extend the existing feature after the Preview investigation. Their
focused shared-following implementation is running against a copy of the private
Preview store; the original store is preserved. Partial progress does not satisfy
the complete storage-boundary, backfill or qualification requirements.

| Requirement | Primary tasks |
|---|---|
| FR-001/002 execution isolation | T004–009, T012, T015–018 |
| FR-003 consistent fast reads | T010–014, T020–021 |
| FR-004 mutation boundary | T001, T006–007, T016–018 |
| FR-005 bounded fair queues | T005–006, T009, T020, T031 |
| FR-006 durability | T007, T017, T020–022 |
| FR-007 native observation | T013–014, T022, T030 |
| FR-008 lifecycle | T008–009, T019, T021 |
| FR-009/010 migration | T003, T023–026 |
| FR-011 existing-state compatibility | T010–011, T021, T023–026 |
| FR-012 observation | T027–028 |
| FR-013 independent qualification | T002, T029–032 |
| FR-014 shared following | T034, T037–038 |
| FR-015 incremental discovery | T035–037 |
| FR-016 bounded replay | T036–038 |
| SC-001/002 latency | T002, T014, T031 |
| SC-003 overload | T005, T020, T027, T031 |
| SC-004 sync progress | T006, T017, T022, T031 |
| SC-005 correctness | T009, T014, T020–022, T029–030 |
| SC-006 migration | T023–026, T029 |
| SC-007 restoration time | T034–038 |
