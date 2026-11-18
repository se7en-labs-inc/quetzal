# Feature Specification: Responsive runtime and automatic state migration

**Feature Branch**: `005-responsive-runtime`

**Created**: 2026-11-17

**Status**: Implementation in progress — inventory, benchmark and compute foundation implemented; production runtime routing pending

**Input**: Dedicated threads for API responses, syncing and sled writes; substantially faster API responses; a dedicated migration tool and automatic cardano-wallet SQLite import when Quetzal has no state.

## Scope and existing work

Conformance establishes behavior, but does not establish latency under restoration,
storage pressure or expensive wallet operations. This feature changes execution
and read paths while retaining the cardano-wallet API, secure transport, wallet
semantics and existing sled compatibility.

The standalone Rust `quetzal-migrate` tool already exists (feature 002). Automatic
startup import was implemented separately in the uncommitted
`004-startup-migration` worktree. Its reviewed complete source patch has now been
integrated here without modifying that worktree or forking the converter. This
feature owns its runtime integration and acceptance coverage. Missing state means
missing Quetzal sled state, even when the configured database directory already
exists and contains SQLite files.

## User Scenarios & Testing

### US1 — Read wallets promptly during restoration (P1)

As a Daedalus user, I can open wallet summaries, balances, addresses and history
while wallets restore or the node reconnects, without waiting behind bulk sync,
cryptographic work or disk writes.

**Why this priority**: Interactive delays directly affect normal wallet use.

**Independent Test**: Compare the same public read workload against the baseline
and candidate during active restoration, using the performance contract.

**Acceptance Scenarios**:

1. **Given** five funded wallets restoring, **when** clients repeatedly request
   wallet summaries and bounded list pages, **then** responses meet SC-001/002 and
   contain a coherent committed wallet state.
2. **Given** the storage writer is stalled for two seconds, **when** clients read
   already published summaries and network information, **then** those reads meet
   SC-001 and no uncommitted checkpoint, balance or reservation is exposed.
3. **Given** a previously observed node disconnects, **when** network information
   is requested, **then** it responds without a synchronous node round trip and
   preserves the established disconnect/progress contract. A cold start without
   an observation retains the existing `not_responding` behavior.
4. **Given** several wallets consume the same historical chain range, **when**
   restoration runs, **then** they share block fetching and decoding and use
   incrementally maintained ownership indexes rather than rebuild them per block.
5. **Given** an output extends a sequential address gap, **when** discovery
   expands, **then** newly derived addresses participate in discovery within that
   block as well as later blocks, regardless of output order.
6. **Given** wallets already follow the live tip, **when** a new wallet begins
   historical discovery, **then** existing wallets keep following live blocks and
   the new wallet catches up without inheriting an unearned checkpoint.

### US2 — Keep mutations and synchronization safe under load (P1)

As a user sending funds while wallets sync, my successful changes survive restart,
read responses reflect completed changes, and overload cannot cause double spends,
lost updates or unbounded memory growth.

**Why this priority**: Faster responses must preserve financial correctness.

**Independent Test**: Interleave submission, address allocation, deletion, block
application and rollback; inject queue saturation, crashes and storage failures.

**Acceptance Scenarios**:

1. **Given** concurrent operations use one wallet, **when** a change is reported
   successful, **then** subsequent reads include it or a later valid state, and
   acknowledged local mutations survive abrupt process restart.
2. **Given** sync and API work exceed capacity, **when** queues fill, **then**
   sync producers pause, API admission has bounded waiting and compatible errors,
   and already accepted mutations receive a definite recorded outcome.
3. **Given** a rollback or deletion races queued work, **when** old work completes,
   **then** it cannot publish an obsolete balance, resurrect a deleted wallet or
   reuse a reserved input or allocated change address.
4. **Given** an EOF or termination request, **when** shutdown begins, **then** new
   work stops, accepted mutations finish or fail explicitly, committed state is
   flushed, threads join within the existing shutdown budget, and a flush failure
   produces an unsuccessful process exit.

### US3 — Launch existing cardano-wallet state without manual conversion (P1)

As an operator replacing cardano-wallet, I use the existing database argument.
If there is no Quetzal state and supported SQLite wallets exist, startup imports
them automatically through the shared migration engine before serving requests.
I can also run the dedicated migration executable to inspect, check or import.

**Why this priority**: This is an explicit requirement and the startup boundary
must remain safe through the runtime redesign.

**Independent Test**: Start `quetzal serve --database DIR` on stopped, funded
reference fixtures; compare public snapshots for all five supported families,
restart, and verify later Quetzal changes are not reimported over.

**Acceptance Scenarios**:

1. **Given** no Quetzal state and supported wallet SQLite files, **when** startup
   runs, **then** it imports credentials, discovery, allocations, balances/assets,
   history, submissions, staking and checkpoint before opening API listeners or
   starting normal wallet workers; no password or recovery phrase is required.
2. **Given** completed Quetzal state, **when** startup runs again, **then** it
   reuses that state even if source SQLite remains or disappears.
3. **Given** no Quetzal state and no wallet SQLite files, **when** startup runs,
   **then** it starts a fresh wallet; a pool-cache-only directory is also fresh.
4. **Given** corrupt/unsupported source, wrong network, invalid checkpoint,
   conflicting destination or failed import, **when** startup runs, **then** it
   fails with actionable context, preserves the source and serves no empty state.
5. **Given** interruption or two concurrent startups during conversion, **when**
   publication is attempted, **then** no completed destination is overwritten and
   later startup can recover safely. The standalone tool retains its commands.

### US4 — Diagnose latency and stalled workers (P2)

As an operator, I can distinguish request execution, queue delay, storage flush,
sync lag and expensive computation, and identify unhealthy workers without
exposing wallet secrets.

**Independent Test**: Inject one delay in each execution domain and verify its
timing and lifecycle observations identify the affected domain.

## Edge Cases

- Empty wallets; modern and Byron namespaces with identical wallet IDs; shared,
  watch-only and random Byron discovery; pending transactions with local change.
- Deep rollback across the imported floor; restart between durable commit and
  read-view publication; wallet deletion/recreation while old jobs are queued.
- Disk full, flush errors, worker panic, CPU saturation, node outage, full queues,
  client disconnect before/after admission, shutdown with an in-flight write.
- Very large history and unbounded API responses: preserve their contract, isolate
  their cost and account for memory; do not silently cap results or claim they
  satisfy the bounded-page latency target.
- Direct sled, managed child sled, mixed SQLite/sled, symlinks and incomplete
  staging directories follow feature 004's fail-closed layout rules.

## Requirements

- **FR-001**: API I/O, node/sync orchestration, sled mutation execution and CPU-heavy
  wallet computation MUST have explicitly owned, dedicated threads or thread
  pools with bounded concurrency. Ordinary tasks on one shared executor do not
  satisfy this separation. Thread count MUST NOT grow with wallet count.
- **FR-002**: HTTP executor threads MUST NOT perform synchronous sled operations,
  blocking filesystem work, cryptographic derivation/password work, unbounded
  scans or long block processing. They await bounded services and render results.
- **FR-003**: Summary reads MUST use coherent committed views; bounded history,
  address and UTxO queries MUST use isolated read workers and appropriate indexes.
  A successful mutation MUST establish read-after-write visibility before reply.
- **FR-004**: All live mutations, including sync, submissions, allocations, pools,
  settings, credentials and deletion, MUST use the storage execution boundary.
  Preserve atomic domain invariants, per-wallet ordering and rollback ordering.
  State-changing requests MUST NOT report success on enqueue alone.
- **FR-005**: Every queue MUST bound both items and retained bytes; admitted work
  MUST have explicit cancellation, timeout, error and shutdown semantics. Sync
  backpressure MUST preserve chain order; fair scheduling MUST prevent either
  interactive work or sync from starving indefinitely.
- **FR-006**: All acknowledged local mutations MUST be durable before success;
  chain checkpoints MUST never advance beyond recoverable ledger state. Lost
  replies MUST NOT imply rollback of already committed operations.
- **FR-007**: Network observation/refresh MUST run outside requests. Preserve the
  existing native-observation and time-derived progress behavior, including
  rollback, disconnect, cold start and shared namespace observations.
- **FR-008**: Lifecycle supervision MUST cancel and join every owned execution
  domain on signal/EOF/failure, drain accepted mutations safely and propagate
  storage or critical worker failure. Preserve mutual TLS and launcher behavior.
- **FR-009**: Retain the dedicated Rust `quetzal-migrate` executable and shared
  conversion engine; implement startup selection/import exactly as US3 and
  feature 004 specify. SQL remains confined to the copied-source migration reader;
  sled remains the sole live persistence system.
- **FR-010**: Migration MUST preserve all supported state and namespaces, private
  staged conversion, flush/close/reopen verification and atomic no-replace
  publication; normal API/workers MUST start only after successful publication.
- **FR-011**: Existing sled stores MUST reopen without manual conversion; any
  additional derived indexes/views MUST be versioned and rebuilt safely before
  readiness. Never replace canonical persisted state with a cache.
- **FR-012**: Record request latency by endpoint class, queue wait/depth/bytes,
  computation time, commit/flush time, view revision, sync lag, backpressure and
  worker lifecycle. Logs MUST omit keys, phrases, passwords and raw source rows.
- **FR-013**: Preserve the independent conformance suite and its expectations.
  Performance qualification MUST run separately against an unchanged workload,
  record baseline/candidate identities and disclose fixture and platform limits.
- **FR-014**: A shared synchronization service MUST fetch and decode overlapping
  chain ranges once per active replay stream and fan out wallet-specific changes.
  Fixed bounded live/catch-up capacity MUST preserve independent wallet cursors,
  fairness, rollback and prompt live following when another wallet joins.
- **FR-015**: Discovery MUST initialize namespace/generation-qualified address
  ownership and known-UTxO outpoint indexes from canonical wallet state once,
  then update them incrementally on discovery, spend, allocation, rollback,
  deletion and restart. Sequential gap expansion MUST discover newly eligible
  outputs within the same block, including outputs preceding the gap trigger.
  Preserve random Byron, shared and watch-only discovery semantics.
- **FR-016**: Historical replay MUST use bounded block preparation/commit batches
  and avoid per-block full wallet scans and needless wallet-state reconstruction.
  Recoverable checkpoints, bounded rollback history and atomic wallet changes
  MUST remain correct across batch boundaries, cancellation and crashes.

### Clarification — 2026-11-17

The user approved explicit HTTP 503 `service_unavailable` responses when bounded
queue admission is exhausted. Retain the existing `{code,message}` envelope.
This extends the pinned API's overload behavior; do not call the new response
part of the unmodified upstream schema or alter the independent suite to accept it.

### Clarification — 2026-11-18: shared following and incremental discovery

The user requested a common engine following for all wallets, with preprocessed
ownership lookups extended when address discovery advances the gap. New-wallet
discovery may take time, but hour-scale restoration is unacceptable for the
ordinary Preview test profile. A 15-minute Preview restoration budget below is
an initial engineering target, not a measured result or a user-approved universal
mainnet guarantee. A node's Mithril snapshot does not supply wallet ownership
history; correctness requires discovery of historical receipts and spends.

### Key Entities

- **Execution domain**: Owned threads, capacity, lifecycle and health.
- **Wallet state revision**: Namespace, wallet ID, generation and committed
  revision used to reject stale work and publish coherent reads.
- **Storage command**: Typed operation, admission identity, ordering scope,
  payload budget, completion result and durability requirement.
- **Read view**: Immutable public projection of a committed revision, rebuilt
  from canonical sled state and published after commit.
- **Migration outcome**: Selected existing/new state path or verified published
  import and receipt; never an implicit empty fallback after failure.

## Success Criteria

- **SC-001**: On the recorded qualification host, wallet summary/list and network
  information requests achieve p95 <=100 ms and p99 <=250 ms during active sync,
  a two-second writer stall and concurrent expensive wallet operations; no read
  request in the designated workload fails. Measure each endpoint separately.
- **SC-002**: Bounded history/address/UTxO pages (up to 100 items) achieve p95
  <=200 ms and p99 <=500 ms during active sync. For any endpoint whose baseline
  p95 exceeds its target, candidate p95 MUST also improve by at least 2x under
  the same workload. Publish results even if a target is missed.
- **SC-003**: At twice the offered qualification load for ten minutes, configured
  queue item/byte limits remain intact, API admission wait is <=250 ms, overload
  is explicit, and recovery occurs within five seconds after load subsides.
  Benchmark verifies retained-job memory plateaus rather than grows with time.
- **SC-004**: Under normal qualification load, sync throughput is >=80% of the
  same-host baseline and each restoring wallet advances during every 30-second
  observation window while the node supplies blocks and storage is healthy.
- **SC-005**: Crash/restart, rollback, duplicate allocation, concurrent submission,
  deletion, overload and shutdown tests lose no acknowledged mutations and
  expose no mixed revision or stale-generation state. Existing API conformance
  passes without changed expectations.
- **SC-006**: Native startup import retains exact supported public state for all
  five families, original credential usability, pending confirmation, restart
  and post-import spending; failures preserve source/completed destinations.
  Packaged standalone and automatic paths both work with bundled SQLite.
- **SC-007**: On the recorded Preview snapshot/host, five fresh wallets covering
  all supported families with the ordinary address gap reach a moving native tip
  within 15 minutes while the SC-001/002 read workload runs. Include a known
  funding transaction; independently verify its receipt, wallet balance/history
  and subsequent spends. An additional joining wallet MUST NOT reset existing
  cursors. Record startup/index-build time, fetch/decode counts, applied blocks,
  bytes written and all misses. This separate native restoration profile does
  not replace the synthetic scale or repeated 1,000-second latency trials.

## Assumptions and boundaries

- These are proposed acceptance budgets, not measured present performance. The
  performance contract fixes the host/workload before candidate measurements.
- The qualification workload is five wallets, each with 1,000 UTxOs, 10,000
  historical transactions and 200 discovered addresses, using deterministic
  public test fixtures. Large mainnet history remains a separately reported
  qualification scope, not implied by these results.
- Source cardano-wallet is stopped; import is one-way and uses feature 002's
  schema/family/network support. Automatic source discovery is the configured
  database directory, not an arbitrary filesystem search.
- No new public API, broker, SQL runtime store, conversion rewrite, daemon mode
  or thread per wallet. Runtime tuning is internal unless CLI changes are
  separately documented and tested for compatibility.
- Checklist completion describes document review; task status and retained
  evidence track actual implementation. No full runtime qualification is implied.
