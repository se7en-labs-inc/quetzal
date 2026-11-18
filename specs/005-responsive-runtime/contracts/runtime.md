# Execution and lifecycle contract

## Ownership

Bootstrap owns every runtime/thread and shutdown handle. HTTP tasks run on API
threads; node sessions, periodic observations, wallet sync and retries run on
sync threads. Fixed compute workers handle derivation, password work, CBOR/block
processing and transaction preparation. Storage readers and the two store writers
have their own threads. Calls across domains use owned bounded messages. No lock
guard spans an external await, queue admission or worker join.

## Admission and resources

Initial tunable limits, validated before use:

| Domain | Item capacity | Retained payload budget | Admission |
|---|---:|---:|---|
| Each store interactive write lane | 128 | 16 MiB | <=250 ms, then compatible failure |
| Each store sync write lane | 32 | 32 MiB | Backpressure ordered producer |
| Shared read pool | 128 | 16 MiB | <=250 ms, then compatible failure |
| Shared compute pool | 32 | 32 MiB | <=250 ms for API; sync backpressures |

Count queued and in-flight owned payloads against budgets. Bound active producers
and HTTP request bodies before expensive decoding/preparation so pressure cannot
move to unlimited waiting tasks. Oversized valid work requires bounded streaming
or an exclusive budgeted job; do not partially apply a block or invent a lower
public payload limit. T001 must identify existing accepted size limits and a
concrete bounded policy for valid oversized jobs before storage implementation.
Account for result payloads and outstanding immutable views as well as inputs.

A writer schedules up to four interactive commands then one sync command when
both are ready. Per-wallet causal order overrides lane priority: rollbacks,
deletion and confirmation are barriers. Revalidate prepared work at commit;
never blindly retry a signed transaction against a changed input selection.
Large atomic commands must have measured execution bounds; never split atomic
ledger/undo/checkpoint invariants for fairness.

Admission overload uses HTTP 503 `service_unavailable` and the existing
`{code,message}` envelope, explicitly approved by the user on 2026-11-17.
Document/test this additive overload response locally; it is not in the pinned
upstream schema. Do not modify the independent conformance expectations.
Admitted state-changing work has no implicit cancellation-on-client-drop.

## Commit and read visibility

1. Validate generation, revision, reservations and domain preconditions.
2. Atomically commit related canonical records and indexes, including undo and
   checkpoint invariants; retries have no external side effects.
3. For local mutations, complete the durability flush, optionally shared across
   a bounded group. A failed flush fails the request and marks storage unhealthy;
   an error does not prove the write never happened.
4. Publish one coherent revision of all affected read views/catalog entries.
5. Complete the caller's result. A later independent GET sees that revision or
   a later valid one. API responses never wait on a flush merely to read the
   previously published committed summary.

Block/checkpoint persistence retains recoverable ordering on crash. Node network
side effects are outside sled transaction closures: persist submission and
reservations before relay, use existing transaction identity for retry, and
preserve existing definitive rejection and uncertain-result behavior. The refactor
must inventory that behavior rather than add a second submission mechanism.

## Startup and migration

```text
parse/validate configuration and TLS
  -> resolve configured persistent state layout
  -> if no sled state + wallet SQLite: shared offline migration
  -> validate/reopen selected state; rebuild derived indexes/views
  -> start owned storage/compute/sync domains and obtain readiness
  -> bind API and begin serving
```

Temporary database mode remains temporary. Reuse direct sled and verified managed
stores. With neither sled nor wallet SQLite, create fresh state. Reject ambiguous
layouts, unsupported source and invalid existing state. Managed migration is
private staging -> convert -> flush/close/reopen -> validate -> atomic no-replace
publish to `DIR/quetzal.sled`. Original SQLite/WAL/SHM remain intact. Feature 004
defines exact discovery/recovery rules; do not duplicate its resolver here.

The standalone `quetzal-migrate inspect/check/import` remains independently
packaged and usable. Automatic startup invokes its shared Rust engine on an
owned blocking bootstrap thread, without nested-runtime blocking on API threads.

## Failure and shutdown

On EOF, signal or critical failure: stop HTTP admission, cancel external producers
and refresh jobs, prevent new commands, drain admitted mutations, flush both
stores, release views/readers, then join compute/storage/sync threads. Fit within
the existing ten-second worker shutdown budget under supported load. Record
timeouts and return failure; never report clean shutdown after an unsuccessful
flush. An executing filesystem call is not made cancellable by aborting an async
handle; supervision must retain ownership and report an unresolved join.

Worker panic, channel closure or storage failure transitions the relevant domain
to unhealthy, rejects new mutations and initiates controlled shutdown for critical
storage loss. Tests inspect actual thread placement, joins and process exits;
task names alone do not establish isolation.

## Observation

Shared sync owns fixed live/catch-up streams and bounded replay cohorts, not a
node connection per wallet. Cohort fan-out preserves independent committed
cursors and wallet generations. Ownership indexes are derived canonical-state
caches; matching updates cannot authorize publication ahead of recoverable
commit. Gap expansion revisits staged outputs until discovery stabilizes and
then applies effects in ledger order, including within-block receipts/spends.
Rollback, late registration, allocation and deletion use the same revision and
generation barriers as storage commands. Batch admission accounts for decoded
blocks, index growth, undo and retained results; it cannot retain a full chain.

Record endpoint class and latency histograms; queue item/byte depth and wait;
compute duration; writer execution and flush; published revision; native refresh
age/outcome; per-wallet sync progress; thread start/stop/panic. Avoid unbounded
metric labels and secret payloads. Provide test-accessible counters/timings and
structured tracing; no new public metrics endpoint is required.
