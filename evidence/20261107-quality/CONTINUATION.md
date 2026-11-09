# Continued Shelley work — 2026-11-07

User said "keep going", approving continuation through the pending Clippy
refactor. The goal is still all 643 obligations, Shelley before Byron. Reuse
the existing branch/spec, preserve all dirty changes; no commits or pushes made.

## V16 frozen quality generation

All quality checks pass: strict all-target Clippy with warnings denied,
workspace tests, rustfmt, release build, and the explicitly enabled live node
protocol/era-history integration test. No lint suppressions were introduced.
Payment validity/metadata parameters and shared-wallet creation now use grouped
argument structs. Test modules and mechanical lints were cleaned up.

Policy public keys are persisted and flushed after authenticated creation;
reading them no longer requires the private key. Policy IDs hash the resolved
native script, not the bare key. Policy creation/ID responses use 202. All five
extended policy cases pass against the pinned Haskell reference. Watch wallets
return the reference missing-policy-key error. New policy tests pass.
Unsigned construction rejects unsupported withdrawal/mint/burn actions instead
of silently ignoring them. Pool ranking/metadata is still incomplete.

The nine HTTP suites pass 363/537 cases, sixteen internal profiles pass 989/1241,
and funded payments/history/discovery/concurrency pass 239/239 checks.
`coverage.json`: **324/643 covered**, exhaustive false. Missing live profiles
and all remaining obligations still apply. `release.sha256` identifies frozen
V16 binaries; do not combine later-generation reports into this gate.

## V17 cancellation generation

`../20261107-cancellation` contains newer source checks and frozen binary hashes.
Wallet deletion signals the restoration worker and joins it after the node
connection shuts down. Cancellation during connection negotiation cleans up
Pallas multiplexer tasks; stalled handshakes time out after ten seconds and
close the socket. Tests prove cancellation/timeout socket EOF and idempotent
worker stopping without deleting wallet state.

V17 workspace tests, strict Clippy, formatting and release build pass. Broader
funded Shelley replay passed 296/307 checks. Failures identify partial balancing,
delegation-fee schemas, input-size probing through balancing, relative TTL
rounding, and synthetic voting responses. Voting/delegation remain unfinished;
do not claim their synthetic responses are real transactions.

## V18 balancing generation — verification in progress

Production partial balancing preserves supplied outputs, inputs and validity,
resolves preselected owned inputs, adds owned inputs only as necessary, conserves
assets, and converges fee/change using the signed-size estimator and live node
parameters. It rejects unsupported certificate/withdrawal/script accounting,
existing collateral fields in reference order, unavailable inputs and oversized
transactions. It does not submit, reserve inputs or insert history.
Delegation-fee reads balance the actual empty-payment prototype and return the
live key deposit. Script execution/external input resolution remain unsupported.

Validity bounds now use exact wall time, floor lower bounds, ceil upper bounds,
and the reference 403 invalid-validity error. Tests cover fractional 200ms slots
and the mainnet Shelley offset.

V18 workspace tests, strict Clippy, formatting and release build pass. Funded
replay is running at this checkpoint with payments, history, discovery,
concurrency, validity, balancing, collateral, delegation fees and input limits.
Results will be in `../20261107-balancing/live-shelley.json` (session 2648).
The policy derivation helper remains pure; only `write_policy_xpub` persists it,
so unrelated authentication cannot create policy keys.

## Owned private services

Fresh cluster marker: `/tmp/quetzal-quality-workspace`, currently
`/tmp/qz-1107-quality-NICdnV`. Faucet 18879; socket `RUN/node.socket`.
Frozen V16 candidate 18892, reference 18893 (UI 18913), V17 18894, V18 18896.
Binary/database/log names match their generations inside RUN. PID files:
`cluster.pid`, `candidate.pid`, `reference.pid`, `cancellation.pid`, `balancing.pid`.
Foreground tool sessions: cluster 71995, V16 80584, reference 49948, V17 87218,
V18 29069. An older cluster/services from the resume checkpoint also remain
running under `/tmp/quetzal-node-workspace`; use explicit generation markers.

Never run list-wallet HTTP suites while funded workflows create wallets.
Freeze binaries during replays. Raw node/service logs and credentials stay
outside the repository. Exact immutable binary/config paths and full-gate
oracle-provenance blocker remain in `../20261107-reboot/CONTINUATION.md`.

Private source backup before these edits:
`/home/sam/.codex/tmp/quetzal-before-clippy-20261107/crates`.
The earlier file-edit incident was already recovered and verified; see the
resume checkpoint. Source writes now use checked patch operations.
