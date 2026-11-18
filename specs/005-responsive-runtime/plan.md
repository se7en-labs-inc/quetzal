# Implementation Plan: Responsive runtime

**Branch**: `005-responsive-runtime` | **Date**: 2026-11-17 | **Spec**: [spec.md](spec.md)

## Summary

Separate API, node/sync, storage and computation onto dedicated owned threads.
Move summary reads to coherent committed projections, isolate indexed range
reads, and route every live mutation through typed ordered storage commands.
Bound admission, payload memory and execution concurrency. Preserve automatic
SQLite migration through the shared feature 004 engine before normal runtime
startup. See [research.md](research.md) for source findings and choices.

## Technical Context

- **Language/version**: Rust 2021; pinned Fenix toolchain from project flake.
- **Dependencies**: Existing Tokio 1, axum 0.8, sled 0.34, pallas 0.32,
  tokio-util, tracing, serde; no new dependency assumed.
- **Storage**: Existing modern and Byron sled stores. Versioned derived summary
  records/indexes only where required; canonical data retains compatibility.
- **Testing**: Unit/property, concurrency/fault, native node integration, packaged
  migration, independent API conformance and reproducible latency benchmark.
- **Platform**: Current Linux/Nix qualification; other platform latency is not
  claimed. Existing authenticated transport remains mandatory in secure mode.
- **Goals**: Spec SC-001–007; normal-load p95 100 ms summaries/network, 200 ms
  bounded pages; >=2x improvement for endpoints missing their baseline budget.
- **Constraints**: Financial correctness, bounded resources, current ten-second
  worker shutdown budget, unchanged launcher/API and no lost acknowledged state.
- **Scope**: Existing workspace; no broker, new SQL store or thread per wallet.

## Constitution Check

| Principle | Design gate |
|---|---|
| I Safety | Commit/revalidate before publication; durable local acknowledgments; fault and native tests |
| II Era awareness | Existing pallas multi-era parsing and rollback rules retained in compute jobs |
| III Type safety | Typed command variants, namespace/revision/generation wrappers, owned zeroizing secret payloads, propagated errors |
| IV REST | Existing API/error/ordering contract and mutual TLS; unchanged independent conformance |
| V Nix | All builds and qualification through the project flake/dev shell |
| VI Sled | Sled is sole live storage; no SQL Cargo dependency; SQLite executable only reads copied migration sources |
| VII Tests | Unit/property plus native persistence, rollback, migration and API integration coverage |
| VIII Quality | Formatting, warning-free compilation, strict all-target Clippy, workspace and native tests before merge; CI green |

Post-clarification review: the user approved the additive HTTP 503
`service_unavailable` overload response on 2026-11-17. Record this narrow extension
to principle IV's pinned contract; preserve ordinary responses and independent
conformance expectations. The migration
source reader follows the existing feature 002 interpretation of principle VI.
If implementation demands a new exception, resolve it explicitly before coding.

## Project Structure

```text
crates/quetzal-api/src/
  main.rs                  bootstrap, readiness, supervision
  runtime.rs               new: dedicated runtime ownership and join handles
  state.rs                 async service clients and published observations
  workers.rs               sync task placement, cancellation, node limits
  handlers/                await service clients; no synchronous storage/CPU
crates/quetzal-wallet/src/
  storage/mod.rs           new: typed command/query clients, admission budgets
  storage/worker.rs        new: fair writer lanes, commits, durability barriers
  storage/view.rs          new: revisions, summary/catalog publication
  storage/query.rs         new: bounded consistent indexed read workers
  compute.rs               new: bounded expensive wallet/block jobs
  db/                      internal canonical state, atomic commands/indexes
  wallet.rs                orchestration using storage/compute services
  sync.rs                  ordered preparation and acknowledged block commands
crates/quetzal-migrate/     reuse reviewed feature 004 library and existing CLI
crates/quetzal-api/tests/   new runtime isolation and fault integration coverage
crates/quetzal-api/src/bin/runtime-bench.rs
                           new opt-in Rust benchmark using public fixtures
docs/                      runtime tuning and existing migration documentation
evidence/<date>-responsive-runtime/
                           public identities, measurements and receipts
```

No dependency cycles: API owns runtime construction and references wallet/network;
wallet owns storage and compute services and depends only on lower layers.
Migration's offline import depends on wallet, never on API runtime ownership.

## Delivery Sequence

1. Inventory direct reads/writes, locks and external waits across every API
   family, including pools/settings; establish unchanged-source performance
   baseline and fault harness. Record feature 004 integration prerequisite.
2. Introduce typed storage boundaries, revisions/generations, atomic domain
   operations, fair queues and lifecycle supervision while keeping behavior.
3. Build committed views/indexed queries and dedicated compute/sync services;
   switch all callers including metadata/GC, node refresh and submission retry.
4. Integrate reviewed feature 004 before startup; rebuild derived views from
   imported/existing state before readiness. Retain the standalone migrator.
5. Validate fault semantics, native behavior and independent conformance; run
   the exact baseline workload against the candidate and publish all results.

### Shared chain processing and discovery

Replace wallet-owned node streams with a synchronization coordinator in
`quetzal-api/src/workers.rs` and bounded wallet-layer preparation services.
Keep one live follower plus a fixed bounded catch-up scheduler. Group wallets
covering the same replay range into cohorts; fetch/decode a block once for that
stream and match it against cohort ownership indexes. Wallet cursors remain
independent. A newly joining wallet cannot rewind the live follower or skip its
own history. Bound queued joins and cache/cohort bytes; use fair catch-up turns
when wallets cannot share a cursor. Do not promise one lifetime decode across
arbitrarily timed joins with a finite cache.

Build wallet discovery state once from canonical addresses, account discovery
data and known UTxOs. Index canonical address bytes to all owning wallet keys,
and UTxO outpoints to owners; retain each wallet's role-specific gap frontier.
Decode transaction structure once and materialize detailed history only for
relevant transactions. Remove spent outpoints and append discovered addresses
incrementally. When a gap expands, revisit unmatched outputs of the staged block
until discovery stabilizes, then apply transaction effects in ledger order.
This must handle a newly discoverable receipt followed by a spend in the same
block. Preserve family-specific random Byron and shared-wallet discovery rather
than assuming every family uses the same sequential public-key rule.
Track the historical coverage of each derived address range as well: where the
supported discovery contract requires checking earlier blocks, enqueue bounded
backfill for the new range and do not mark the wallet ready before it completes.
Same-block rescanning alone does not establish complete historical coverage.

Use bounded ordered replay batches through the storage writers, retaining the
required block-level undo/intersection semantics. Cache updates are provisional
until commit and are discarded/rebuilt on stale revision, rollback or failure.
Prune rollback data only against an explicit recoverable floor and preserve
deep-rollback replay behavior. Include address allocations, imported state and
wallet generation changes in index invalidation. Measure fetch/decode, matching,
batch commit, I/O and retained memory separately; threads alone do not establish
the restoration target. T034–T038 add this work to the existing dependency graph.

## Risks and Controls

- A single writer can bottleneck or postpone rollback: use two fixed store
  writers, bounded command sizes and fair lanes; measure sync/API together.
- Priority lanes can reorder dependent work: explicit per-wallet barriers and
  generation/revision checks take precedence over lane preference.
- Cached balances can become financially incorrect: derive views in the same
  ordered commit path; independently compare canonical state under adversarial
  operation sequences, restart and rollback.
- Synchronous range reads can see mixed revisions: read fences/barriers and
  bounded retry; do not assume sled offers general snapshot reads.
- Admitted jobs outlive HTTP cancellation: owned completion tracking and
  shutdown drain; a timeout after commit is not a rollback or safe blind retry.
- Migration feature's reviewed source is integrated here; preserve its separate
  worktree and qualify the combined runtime before completing US3.
- Preserving current performance budgets may require further optimization:
  retain measured misses and seek direction before switching approaches.

## Supporting Artifacts

### Preview diagnostic integration (2026-11-18)

Extend the opt-in Rust driver with a read-only `probe` mode for real API instances.
Discover wallet IDs through bounded list requests, keep them private, and probe
endpoint templates with independent deadlines and at most four concurrent calls.
Write incremental JSONL before/after each request so a stalled round cannot erase
evidence. Record small whitelisted progress fields, never full wallet payloads.
A failed or timed-out endpoint must not prevent other endpoints being sampled.
Use current official Preview configurations and a Mithril-verified database with
an isolated node socket/store. Run Quetzal with isolated fresh state; a disposable
watch-only wallet may exercise restoration without spending. Compare genuine
native node progress with API observations. This is diagnostic integration, not
the synthetic 1,000-second performance acceptance protocol. T033 tracks it.

- [Data model](data-model.md)
- [Execution and lifecycle contract](contracts/runtime.md)
- [Performance protocol](contracts/performance.md)
- [Quickstart and validation](quickstart.md)
- [Implementation tasks](tasks.md)

Implementation started with the runtime inventory and opt-in Rust benchmark.
The user approved the 1,000-second trial and candidate-only writer-stall protocol
correction on 2026-11-18. See tasks and public evidence for actual completion;
no production responsiveness or final qualification claim is implied.

The focused shared-following implementation now uses one owned sync thread,
fixed live/catch-up streams and resident discovery/UTxO indexes. It is a staged
repair to the native replay bottleneck while the complete typed storage/view
boundary remains pending. Its direct atomic DB commits must move through the
planned storage commands before the full isolation contract is complete. Native
diagnostic receipts and limitations are in `evidence/20261118-shared-sync/`;
historical discovery-range backfill and SC-007 qualification remain open.
