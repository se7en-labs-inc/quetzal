# Research and decisions

Inspected master `84f2f4d` on 2026-11-17. Findings are from current local source;
no external library behavior or benchmark result is assumed.

## Observed bottlenecks and risks

| Source | Observation | Design consequence |
|---|---|---|
| `crates/quetzal-api/src/main.rs` | `#[tokio::main]` serves HTTP and spawns chain sync, refresh and GC tasks on the same runtime | Give HTTP and background node work distinct owned runtimes |
| `crates/quetzal-api/src/workers.rs` | Per-wallet sync/retry tasks use `tokio::spawn`; registry locks participate in lifecycle operations | Move tasks to the sync runtime; extract handles before awaiting; bound active computation/connections |
| `crates/quetzal-wallet/src/sync.rs` | Async processing performs synchronous address reads, discovery and ledger commits | Separate ordered node reception, bounded computation and storage commit acknowledgments |
| `crates/quetzal-wallet/src/wallet.rs` | `list_wallets` calls `get_wallet`; balances scan UTxOs, addresses and pending CBOR; create derives keys and writes multiple records | Maintain committed summary views and move cryptographic operations to compute workers; turn related writes into atomic domain commands |
| `crates/quetzal-wallet/src/wallet.rs` | Transaction history loads `usize::MAX` records before projection | Inventory query filters/order and implement compatible indexed bounded reads where the request is bounded |
| `crates/quetzal-wallet/src/db/*` | Synchronous sled persistence is called directly by services, including flush barriers | Centralize live mutations; isolate synchronous reads and flushes from HTTP |
| `crates/quetzal-api/src/handlers/network.rs` | Network information performs a node query with a three-second timeout; parameters also query on request | Periodic background observation plus committed observation cache; query-dependent commands await a node service |
| `crates/quetzal-wallet/src/writer.rs` | This module builds transaction witnesses/certificates | Add a distinct `storage` module; do not mistake or repurpose it as a database worker |

The existing conformance receipt in
`evidence/20261117-master-conformance/README.md` records 651 obligations/43 profiles
for its pinned source, including the disconnect correction now in master. That
receipt is behavioral evidence for that source, not performance evidence for
this design. Preserve its last-native-observation semantics.

## Decisions

1. **Owned execution domains**: Bootstrap supervises an API Tokio runtime, a
   separate sync/node Tokio runtime, fixed storage write threads (one per modern
   or Byron store), a bounded storage read pool and a bounded compute pool.
   Thread counts are fixed per process configuration, not per wallet. Start with
   two API threads, two sync threads, two read threads and two compute threads;
   tune only using the recorded benchmark. Sled's internal threads remain sled's
   responsibility and are included in observed process resource usage.
2. **Typed services**: Keep wallet logic in `quetzal-wallet`; runtime placement in
   API bootstrap. Hide live database mutation handles behind a storage service.
   Return owned values over bounded channels. Migration retains an exclusive
   offline import boundary rather than queueing partial imports into live state.
3. **Coherent reads**: Publish immutable, revisioned per-wallet summaries only
   after commit, with a namespace catalog snapshot for list operations. Query
   workers use compatible indexes and a revision fence/retry or serialized read
   barrier to avoid mixed records; plain independent sled reads are not a
   snapshot guarantee. Retry is bounded and cannot silently return mixed data.
   Only summary projections are held in memory; do not duplicate full history.
4. **Ordering and preparation**: Compute expensive plans off-thread from an
   identified committed revision. The writer revalidates generation, revision,
   input reservations and allocation constraints, then atomically commits all
   affected records/indexes. A single FIFO of arbitrary closures is insufficient:
   it permits stale calculations, unbounded payloads and head-of-line blocking.
5. **Durability and fairness**: API and sync have separate bounded lanes per store.
   Schedule weighted round robin (initially four interactive commands then one
   sync command when both are ready), with command batches capped by size and
   execution budget. Never interleave the interior of an atomic domain change.
   Preserve per-wallet chain order and rollback barriers across lanes. Group
   flushes may acknowledge multiple local commands together, but none before
   their flush and view publication. Chain receipt waits for coherent commit;
   checkpoint recovery remains transactionally tied to ledger/undo state.
6. **Network cache**: Refresh native snapshots on the sync runtime at one-second
   intervals with one in-flight refresh per namespace-independent connection
   service and bounded query timeouts. Record observation time and failures.
   Startup attempts one bounded observation before readiness, as existing launch
   behavior requires. GET network information never waits for a new query.
   A failed refresh retains the last native observation and recomputes progress
   using current time; historical replay cannot invent native readiness.
7. **Migration dependency**: Feature 004's current uncommitted design extracts
   a shared library in `quetzal-migrate`, resolves fresh/direct/SQLite/managed
   layouts, imports to `DIR/quetzal.sled`, and completes before API binding.
   Integrate its reviewed result before US3 work. Keep `inspect/check/import` and
   Nix-bundled SQLite. No second executable, converter, schema policy or source
   directory option is needed for this feature.

## Alternatives assessed

- More tasks or more threads on the existing runtime do not bound disk/CPU
  interference or fix repeated scans.
- `spawn_blocking` at every handler moves work but shares an uncontrolled
  resource pool and leaves mutation ordering and memory pressure implicit.
- Routing every read behind the writer leaves summary reads blocked by flush.
- Per-wallet threads scale poorly and still contend on shared resources.
- An external storage daemon introduces deployment and protocol complexity
  without evidence that process isolation is needed.

## Remaining implementation investigations

### Preview follow-up — 2026-11-18

The verified Mithril node is current; a fresh wallet still restores from genesis.
The preceding implementation opened an independent chain-sync connection per
wallet and serially requested/processed blocks. `sync.rs` reloaded addresses and
UTxOs, reconstructs discovery pools/ownership maps and stages transaction details
on every block. `db/chain.rs` commits checkpoint, metadata and undo per block,
even for irrelevant blocks; this path does not explicitly flush per block.
Six diagnostic wallets are concurrently replaying. A 30-second sample of the
new funded wallet advanced 31,812 blocks (1,060.4 blocks/sec), far from completion.
These are source findings and live observations, not a profile attributing a
percentage of time to each cost. See `evidence/20261118-preview-api/` for the
earlier low-load responsiveness run; it did not qualify restoration speed.

User direction is shared following plus preprocessed ownership indexes extended
on gap growth. The updated plan uses bounded cohorts and live/catch-up capacity;
incremental address/outpoint maps; same-block discovery closure; and bounded
ordered commits. The focused shared service and retained matcher are now running
against a copy of the Preview store; see `docs/shared-sync.md` and
`evidence/20261118-shared-sync/`. T034–T038 track the remaining storage integration,
historical backfill and native qualification, including a proposed 15-minute
ordinary Preview restoration target. Existing mainnet history guarantees are not
inferred from this profile.

The detailed [runtime inventory](inventory.md) records all persistence command
families, read/network paths, locks and current submission outcomes. It identifies
the generic-overload API mapping decision: the user approved HTTP 503
`service_unavailable` on 2026-11-17. This is an explicit extension to the pinned
schema using the existing error envelope; ordinary conformance remains unchanged.

Before changing execution paths, complete the command inventory, establish the
baseline and confirm sled transaction coverage for dynamic trees, range-read
fences and deletion/recreation. Use the checkout first; inspect external API
source through the agentic-nix skill only if necessary. Prototype benchmark and
native diagnostic runs have receipts; full qualification is pending. If
investigation requires changing the agreed architecture
or a validation fails, report the finding and ask before a different approach.
