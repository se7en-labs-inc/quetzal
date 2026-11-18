# Shared wallet synchronization

The API registers wallet consumers with one owned `wallet-chain-sync` thread
(current-thread Tokio runtime, explicit 16 MiB stack). Live following and
historical catch-up each have one active stream. Catch-up cohorts contain at most
64 consumers; registration is limited to 256 consumers and a 64-command channel.
A 20 ms initial registration window groups wallets opened together. Completed
catch-up quanta rotate fairly; late registrations cannot rewind an active cohort.
Wallets reaching the tip merge into the live stream. Namespace-separated stores
retain independent workers/cursors, even when wallet IDs coincide.

Each block is decoded once for its cohort. Output address bytes and input
outpoints are normalized once, without formatting unrelated addresses as text.
Wallet discovery pools, address ownership and UTxO maps are loaded from canonical
state and retained across blocks. External discovery mutations invalidate these
caches. Matched outputs extend the role-specific gap, and the staged block is
rescanned until discovery stabilizes before applying transactions in ledger
order. Only relevant transactions require detailed history materialization.
Rollback, reconnection failure and deletion invalidate speculative state.

Relevant wallet effects retain atomic per-block commits. Empty wallet blocks
save a recoverable endpoint at most every 256 blocks, at cohort suspension and
at the live tip. A crash may replay empty progress; it cannot discard committed
receipts/spends. Pool history is a shared consumer, with batches of at most 256
blocks and per-block reversible records. It resumes from persisted native points
instead of replaying from genesis on every startup. Native clock/tip observation
runs independently of historical processing. Network information reads the last
native observation and computes progress against current time.

The service explicitly handles the node's initial rollback-to-intersection
message separately from a later chain rollback. Ahead consumers are validated
before skipping history. Removed consumers cannot write after their removal is
acknowledged. Shutdown cancels streams and joins the owned thread before flushing
stores. Critical replay/storage task failures signal main supervision.

This is the shared-following portion of feature 005. The complete typed storage
boundary, all API read views, full memory budgets, historical discovery-range
backfill and final fault/performance qualification remain tracked in its tasks.
The ordinary Preview restoration budget is a proposed acceptance target; retain
actual trial receipts rather than infer success from this architecture.
