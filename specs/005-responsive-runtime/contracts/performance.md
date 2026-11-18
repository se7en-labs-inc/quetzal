# Performance qualification protocol

This is an acceptance protocol, not a recorded benchmark result.

## Fixed comparison

Before implementation, record baseline source `84f2f4d`, executable and flake-lock
hashes, build profile, CPU/core count, RAM, disk/filesystem, kernel, runtime thread
settings, node version/network/era and fixture hashes. If feature 004 is integrated
before comparison, also measure that exact source as an intermediate control;
do not silently change the baseline. Use the same otherwise idle host, pinned
Nix release build, node, fixtures, transport and workload for the candidate.
Reference host has >=4 physical CPU cores, >=8 GiB RAM and local SSD; report exact
resources. Current default thread counts are a starting configuration, not a
performance result.

Use five namespace-qualified wallets including all supported families. Each has
1,000 UTxOs, 10,000 historical transactions and 200 discovered addresses. Generate
deterministic fixtures using a Rust fixture builder, real ledger encodings and
wallet import/test boundaries. Synthetic fixtures measure scale only. Independently
verify public/canonical fixture invariants and record pending reservation cases.
Native-funded fixtures separately qualify correctness and actual chain behavior.
Keep signed payloads, private state, keys and phrases out of public evidence.

## Workload

Use a Rust load driver in `quetzal-api` with existing reqwest/Tokio dependencies.
Send open-loop 50 requests/second across 16 connections, with a maximum 64
in-flight requests. Record offered, started, completed, rejected and overdue
requests; measure latency from scheduled send time to final response body to
include client queue delay. Do not hide coordinated omission. Keep workload seed,
endpoint schedule and timeouts unchanged between runs.

Mix: 20% single-wallet summary, 10% wallet list, 20% network information,
20% history pages, 10% address pages, 10% UTxO pages and 10% local mutations or
expensive wallet operations. Use the pinned API's actual query semantics; where
an endpoint has no paging contract, select a bounded fixture/response and report
it separately rather than adding pagination. Creation/deletion cycles use a
fixed number of transient wallets so data size does not grow between trials.
Include watch-only reads and encrypted derivation/password work. Verify mutation
results and expected error responses independently of latency statistics.

Warm up for 60 seconds, measure 1,000 seconds, repeat three times from identical
state. Retain per-endpoint histograms and sample counts (>=1,000 per measured
endpoint per scenario), p50/p95/p99/max, CPU, RSS, queue budgets, sync throughput
and sync lag. Every repetition must meet the acceptance thresholds; do not pass
on an average that conceals a failing trial.

Scenarios:

1. Caught up, idle node background work.
2. All wallets restoring against the same block stream; measure applied
   blocks/second and every wallet's progress alongside API latency.
3. Candidate-only controlled two-second writer stalls, injected ten times at recorded offsets;
   summaries/network must stay responsive. Mutations may wait for execution and
   are evaluated for correctness, not the summary latency budget.
   The unchanged baseline has no dedicated writer and is not modified to invent
   one for this fault test. Compare baseline/candidate on normal load and native
   replay; do not present candidate-only fault measurements as a paired speedup.
4. Concurrent expensive derivation/password/transaction preparation jobs; fixed
   operation rate from the workload above and bounded worker occupancy.
5. Node disconnect/reconnect and rollback with cached observations.
6. Twice offered load (100 requests/second) for ten minutes, then normal load;
   verify <=250 ms admission, item/byte limits, explicit overload and recovery
   within five seconds. Plateau test compares retained-job bytes and RSS across
   the final five one-minute windows after warmup; investigate monotonic growth
   beyond allocator noise rather than claiming bounded queues prove bounded RSS.

Apply SC-001/002 budgets to scenarios 1–5, SC-003 to scenario 6 and SC-004 to
healthy scenario 2. Report each endpoint, status class and phase separately;
an overload error is never a fast successful response. Accepted mutation execution
latency and durability are reported separately; only admission has a 250 ms bound.
Under a deliberate storage stall, an accepted mutation can legitimately take
longer than this to execute.

The 2x improvement requirement applies per endpoint whose unchanged baseline
misses its absolute p95 budget. Absolute budgets still apply when baseline is
already fast. If baseline and candidate both miss, record failure and seek
direction before adopting a different architectural approach.

## Correctness gates and evidence

Protocol clarification approved by the user on 2026-11-18: extend measurement
from 300 to 1,000 seconds at unchanged offered load so even the 2% per-family
request classes have at least 1,000 samples per trial. Writer stalls are a
candidate-only fault gate. Latency budgets and independent conformance remain
unchanged. Short smoke runs validate the harness and never qualify performance.

Latency cannot compensate for stale/mixed balances, altered response fields or
weakened TLS. Run unchanged independent conformance and dedicated crash,
rollback, concurrent mutation, storage-error and migration tests. Native startup
qualification verifies all five families, original credentials and pending
confirmation, restart and later spending against the node.

Publish a public receipt directory with source/build/fixture identities, exact
commands, repetitions, raw timing distributions or bounded histogram data,
resource samples, gate outcomes and scope exclusions. Extended mainnet replay,
large histories beyond this fixture and other hosts remain unqualified unless
separately measured. Never relabel old conformance receipts as new evidence.

## Native restoration budget — SC-007

Additional profile following the 2026-11-18 live Preview diagnosis. Proposed
engineering target: complete restoration within 900 seconds per fresh wallet on
the recorded Preview snapshot/host, with five supported families restoring and
the ordinary read workload running. Measure from accepted creation through
discovery initialization and reaching the moving native tip; do not stop at a
percentage threshold or at detecting one UTxO. Retain all family completion times.
Apply the recorded family-specific gap/discovery defaults. This profile is
separate from the five-wallet synthetic scale profile and 1,000-second trials.

Freeze the node database snapshot at
`51c321956b4321423f12111781e89ce14fd40483c64f7e62d3b3199e4928517e`
(Preview epoch 1485/file 29710) and record the later native tip/chain span used in
each paired replay. Reset only disposable wallet state between trials. Funding
must occur at a recorded native transaction/point; independently compare node
UTxOs and wallet history/balance. No deterministic test wallet receives real ADA.
Also run one late join after existing followers reach the tip: their cursors must
remain live while the new wallet performs its own discovery. Retain original
baseline and candidate source identities; performance misses remain failures.

Record blocks fetched/decoded per stream, logical wallet-block applications,
index initialization/extension work, matching time, batch sizes/commit time,
process/kernel write counters, database growth, retained memory and API latency.
Decoded-block counts must demonstrate sharing for cohort ranges, rather than
hide per-wallet duplicate work inside a pool. Verify same-block gap extension,
spends, rollback/restart and all-family equivalence before timing. Finite caches
may reread old ranges for late joins; disclose those counts separately.
