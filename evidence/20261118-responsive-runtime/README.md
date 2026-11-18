# Responsive runtime: implementation start

Worktree `005-responsive-runtime`, base
`84f2f4d691df3a32928766733def609164888475`. Source changes are uncommitted.

## Implemented scope

- Source inventory covers reads/writes, network waits, locks, submission outcomes,
  transaction boundaries and oversized-job handling.
- Optional Rust benchmark and deterministic five-family scale fixture builder.
  Each full fixture wallet has 1,000 UTxOs, 10,000 CBOR-backed history records,
  200 used addresses and one pending reservation. Reopen checks independently
  calculate available/total balance and verify counts. The fixture is synthetic,
  not a real node ledger or evidence of native funding.
- Standalone compute pool owns fixed worker threads; limits queued/in-flight/
  retained-result item and byte budgets; bounds admission; reports preparation
  panic; drains accepted pure work and joins. Tests cover cancellation and failure
  of the last worker without abandoning callers. Production callers are not yet
  routed through it; no API latency improvement is claimed.
- Approved HTTP 503 `service_unavailable` mapping with the existing JSON envelope.
  The user approved this narrow extension to the pinned API contract.
- Reviewed feature 004's complete source patch is integrated, with its specs and
  standalone migration executable preserved. Patch SHA-256:
  `a80dcf81ab8cd3d439d04afb22ed8eff1307ef0fcc3e6886c26ffe506c4e7d96`.
  Original worktree and native evidence remain untouched. New runtime/migration
  acceptance will be rerun after runtime integration, not inferred from old receipts.

## Prototype harness smoke runs

`smoke-1.json` and `smoke-2.json` each contain 1,000 measured requests at 50/sec
for 20 seconds, following two seconds of warmup and route preflight. Both have
`complete=true`, `all_responses_valid=true`, zero invalid/missing measured responses,
and explicit false performance/native qualification fields. Run 2 adds once-per-
second target-process resources. The prototype driver evolved between runs;
these are harness checks, not repeated qualification trials.

Both target the unchanged release, built from master via `nix build`:

```text
/nix/store/w6cxjb3l90wllaniabadsj6g21d3w1l8-quetzal-0.1.0/bin/quetzal
SHA-256 bf740efcb4ec7daf9c0463a5e454d4375b395f19040ec2186d3406c8efe94b51
flake.lock SHA-256 acba8cf07723def3cb60f0822bcce3fbbbf3c9ea6a5d68bd4c8b3c9b240d3a49
```

Host: AMD Ryzen AI 9 HX 370, 12 physical cores/24 logical CPUs, approximately
62 GiB RAM, Linux 6.18.60, ZFS user-data filesystem. Fixtures are on that filesystem,
not `/tmp` (which is tmpfs). Process RSS, CPU ticks and I/O samples are in run 2.

The node socket was deliberately absent; native sync was not running. Loopback
plaintext mode was used. Thus these runs do not measure a connected/native node,
restoration interference, mutual TLS latency, storage stalls or overload recovery.
Run 1 summary p95 was approximately 144–148 ms across families in its mixed
workload; its small sample counts are insufficient for acceptance conclusions.

Owned runtime root:
`/home/sam/work/se7en/quetzal/.runtime-bench-20261117-L96pgZ` (private).
Driver commands used generated `fixture/manifest.json`, loopback ephemeral ports,
20/2/50 duration/warmup/rate, scenario `node-unavailable-smoke` and source identity
`84f2f4d691df3a32928766733def609164888475`. Run 2 set
`QUETZAL_BENCH_PID` to its owned process. Both servers exited successfully on
Ctrl-C; run 2's private log confirms `Quetzal shutdown complete`. Private sled
state and full process logs are not published.

## Local validation

Combined source including the reviewed startup migration passes:

- `nix develop -c cargo test --workspace --all-features`: 137 passed, zero failed,
  three explicitly ignored external-fixture tests. See `workspace-tests.log`.
- Formatting and strict workspace/all-target/all-feature Clippy.
- `nix build --no-link --print-out-paths`, including its package test phase:
  `/nix/store/pxsf9hw8vm3k7fsj0bxfhspfs834j4pa-quetzal-0.1.0`.
  Executable hashes are in `package-binaries.sha256`.
- The packaged `serve` rejects invalid SQLite with exit 1, leaves source bytes
  unchanged and creates neither direct nor managed sled state. The opt-in
  benchmark is excluded from the default package. See `failure-startup.json`.

An intermediate compute-foundation build before migration integration succeeded
at `/nix/store/6p1yrb6h93w1qc0r4pf73pyxk856n91m-quetzal-0.1.0`.
Neither new build is a fully integrated responsive-runtime candidate.

## Qualification still required

The user approved 1,000-second quiet-host trials at unchanged 50/sec to obtain
at least 1,000 samples per endpoint/family, plus candidate-only writer fault
injection. Normal/native replay comparisons use the unchanged baseline and
candidate. API/sync/storage integration, read views, full native correctness,
independent conformance and repeated performance acceptance are pending.

See the feature tasks and `docs/runtime-benchmark.md`. Smoke success and unit
tests do not constitute completion of the runtime feature.
