# Quetzal resumed — 2026-11-07

Worktree: `/home/sam/work/se7en/quetzal/wallet-migration`, existing branch
`001-wallet-rust-backend`. Nothing staged, committed or pushed. Preserve all
pre-existing dirty files. The goal remains all 643 obligations, Shelley first.

## Implemented and verified

- Writer replay now passes 28/28, confirming the pre-reboot ordering/error fixes.
- API signing uses `writer::merge_vkey`, preserving other witnesses and scripts
  and producing identical transaction bytes on repeated signing. A handler-level
  regression verifies signatures and body identity.
- History accepts `max_count=0` (confirmed against the running pinned reference,
  despite the Swagger query minimum) and filters `minWithdrawal` against each
  individual withdrawal. Regression covers zero limits, missing wallets,
  thresholds and filtering before truncation.
- Mechanical Clippy fixes use integer `is_multiple_of` in slotting and fixed-size
  AES chunks in metadata encryption. Release metadata vectors pass 6/6; slotting
  vectors pass 87/87.
- Final workspace tests pass (one node-dependent integration test ignored),
  formatting passes and the release build passes. Logs are beside this file.
  Clippy remains failing; see `clippy.log`.

## Evidence generations

V14: `release.sha256`, complete nine HTTP and sixteen internal profiles, plus
`live-payments.json` with payments/history/discovery/concurrency enabled.
HTTP: 358/537 pass. Internal: 989/1241 pass. Funded: 197/199 pass. Remaining
funded failure was history rejecting zero count, which stopped later queries.
`coverage-v14.json` covers **316/643**, exhaustive false; 327 obligations remain
missing and the complete live catalogue has not run. Preserve failed reports.

V15: `history-release.sha256`, final source with history corrections. Focused
`live-history-v15.json` passes **148/148** against port 18794. Do not combine this
different generation/target/environment with V14 to claim a complete new gate.
`workspace-tests.log`, `format.log`, and `history-release-build.log` validate the
final source. The latest release executables are V15.

The 252 internal failures remain in seeded Haskell selection (157), credentials
(61) and wallet-selection/mint/burn (34). Other remaining domains and trusted
oracle provenance are documented in `../20261107-reboot/CONTINUATION.md`.

## Pending user choice

Strict Clippy fails on existing sorting, test layout, cloned slices, sync type
complexity, duplicate writer error branches, and payment/shared-wallet function
argument counts. An asynchronous question asks whether to refactor those
signatures into parameter structs with mechanical lint cleanup, or defer that
cleanup and continue Shelley behavior. No answer received at this checkpoint;
the dependent signature refactor has not been performed.

## Local services

Fresh owned cluster: `/tmp/qz-1107-resume-NTjqcc`, marker
`/tmp/quetzal-node-workspace`. Node socket: `RUN/node.socket`; faucet port 18779.
V14 candidate uses port 18792 and `candidate.sled`; reference uses port 18793
and `reference`; V15 candidate uses port 18794 and `candidate-history.sled`.
Raw logs and databases remain outside the repository. Foreground tool sessions:
cluster 15412, V14 candidate 57234, reference 19334, V15 candidate 57467.
`RUN/candidate-history.pid` records the V15 process. Do not assume these survive
another reboot. Commands and immutable binary paths are in the reboot checkpoint.

## File-edit incident and recovery

A shell move command invoked missing `awk`, then continued and emptied
`handlers/transactions.rs`. The file was recovered from the previous native
Codex session's complete `cat` output, replaying every subsequent recorded patch
at its original hunk coordinates and formatting step. Today's patches were then
replayed and the tests moved to the end of the file. A first context-only replay
misplaced an ambiguous field; that attempt was discarded and coordinate-based
replay corrected it. The final comparison against the recovered pre-session
source contains only intended query/signing changes and the new tests. Final
workspace tests and release build pass; funded history passes 148/148.

Pre-session recovery source is retained privately at
`/home/sam/.codex/tmp/quetzal-recovery-20261107/transactions-before-resume.rs`.
The previous native transcript is
`/home/sam/.codex/sessions/2026/11/06/rollout-2026-11-06T15-55-39-01a1b2a5-4c04-7070-be1a-cf6e0f1449a7.jsonl`.
No recovery source or raw transcript was added to the repository.
