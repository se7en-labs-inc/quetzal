# Implementation and validation quickstart

Implementation worktree:
`/home/sam/work/se7en/quetzal/005-responsive-runtime`.
Feature 004 remains a separate dirty worktree. Its reviewed complete source patch
is integrated here; preserve the original worktree and its evidence. The compute
foundation and optional benchmark now exist; production callers
are not yet routed through the new pool. See `docs/runtime-benchmark.md` for the
current executable and exact fixture/receipt behavior.

Resolve feature paths before implementation:

```sh
.specify/scripts/bash/check-prerequisites.sh --json --require-tasks --include-tasks
```

Use the project environment for development and checks:

```sh
nix develop -c cargo fmt --all -- --check
nix develop -c cargo clippy --workspace --all-targets -- --deny warnings
nix develop -c cargo test --workspace
nix build
```

After the planned benchmark target exists, run it through `nix develop` using
its documented baseline/candidate launch arguments. Capture the same-host
baseline before refactoring. [The performance protocol](contracts/performance.md)
is the authority for fixture size, scenarios, repeats and thresholds; a cargo
unit test or successful HTTP ping is not latency qualification.

Run focused runtime tests for writer stalls, queue saturation, generation/revision
races, flush failure, abrupt crash and EOF before the full native run. Use owned
temporary fixtures and nodes; never migrate a personal source database as a test.
Run native sync/rollback/submission tests and the existing independent conformance
runner without editing its source, expectations or user-owned files. Retain exact
runner/source identities and all failures in public evidence.

For migration, exercise both the packaged `quetzal-migrate inspect/check/import`
commands documented in `docs/sqlite-migration.md` and ordinary `quetzal serve
--database DIR` on stopped native SQLite fixtures. Verify fresh/direct/managed
restart, all five families, retained source hashes, invalid/corrupt input,
interrupted/concurrent publication and original credentials/post-import spending.
Normal listeners/workers must never start on a partial import.

Record public results under `evidence/<date>-responsive-runtime/README.md` with
exact commands, identities and scope. If an approach or check fails, explain it
and ask before implementing a different approach. Do not push or create external
issues without an explicit request.
