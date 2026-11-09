# Quetzal Shelley checkpoint — V21

Work in `/home/sam/work/se7en/quetzal/wallet-migration`, branch
`001-wallet-rust-backend`. User said "keep going", authorizing the previously
pending Clippy refactor. All dirty changes are preserved; nothing staged,
committed or pushed. Goal remains **all 643 obligations, Shelley before Byron**.
C005–C008 remain open per the user's full-gate instruction.

## Current evidence

Frozen V21, target `http://127.0.0.1:19092/v2/`, environment
`quetzal-shelley-v21`. `release.sha256` identifies candidate and adapter.

- Complete nine-suite HTTP replay: **364/537 pass**.
- Complete sixteen-profile internal replay: **989/1241 pass**.
- Funded Shelley replay: **365/365 pass**. Flags: payments, history, discovery,
  concurrency, validity, balance, collateral, delegation-fees.
- Combined `coverage.json`: **331/643**, exhaustive false; 312 obligations remain.
- Workspace tests, strict all-target Clippy (`-D warnings`), rustfmt, release build
  and explicit live-node protocol/era-history integration pass. Logs are here.
- Five real create/delete/get cycles leave socket descriptor count unchanged
  (12 before/after), recorded in `worker-resources.json`.

The complete twelve-profile live catalogue has not run. No completion claim.
The 252 internal failures remain seeded Haskell selection (157), wallet
credentials (61), and wallet-selection/mint/burn (34).

## Implemented this continuation

- Clippy refactoring groups payment validity/metadata and shared-wallet creation
  parameters; mechanical lints/unused types are fixed without lint suppressions.
- Authenticated policy-key creation persists and flushes the public key; read and
  policy-ID operations require no private key. Derivation itself remains pure;
  only `write_policy_xpub` persists. Policy IDs hash the resolved native script,
  and creation/ID responses use 202. All five extended policy HTTP cases pass.
- Wallet workers stop by signal and join after node shutdown. Cancelled/stalled
  handshakes close their multiplexer/socket; negotiation times out at ten seconds.
  Socket EOF and worker lifecycle tests pass.
- `balancing.rs` implements unsigned owned-input partial balancing: preserve
  outputs/validity, retain preselected inputs, add owned inputs only as needed,
  conserve native assets, converge fees and change, enforce signed-size limits.
  It rejects collateral fields in reference order, existing witnesses,
  unavailable/duplicate inputs, and unsupported scripts/external inputs,
  withdrawals, mint/burn and certificate accounting. It does not reserve inputs,
  submit transactions or insert history. Funded balancing confirms on the node.
- Delegation fees use the actual balanced empty-payment prototype and live
  protocol deposit. Independent signed-size estimates and state invariants pass.
- Explicitly relative validity bounds both round up from exact wall time; omitted
  lower bound remains zero. Invalid bounds return the reference 403 response.
  Captured Conway validity rejection CBOR is decoded by verified constructor tags
  and rendered exactly; expired/future intervals and later admission pass live.
- Unsupported construction withdrawal/mint/burn requests now reject explicitly
  rather than silently dropping the requested action.

## Required user answer before the next encoding approach

An asynchronous question asks whether to add a scoped encoding step and preserve
raw body bytes through signing/submission. **No answer received; do not assume
approval.** The user's AGENTS.md requires asking before changing approach after
a failed test. Concrete proposal:
`../20261107-balancing/encoding-proposal.md`.

The size-boundary replay passed 23 inputs but rejected Pallas's definite array at
95 inputs. Pinned Haskell requires indefinite arrays above 23. Pallas `Set<T>`
always encodes a definite array and has no framing selector. The proposed change
keeps Pallas typed transactions, changes only input-array framing, and uses raw
body identity/witness-only replacement consistently in signing, submission,
decoding and pending IDs. **No implementation of that change has been applied.**
Do not rerun the known failing input-limit profile expecting success beforehand.

## Remaining major work

Shared activation/address derivation/merged signing; real staking and voting
transactions (synthetic responses still exist and are not real ledger behavior);
withdrawal/certificate/script/external-input balancing; native mint/burn; seeded
selection; credential transitions/fault atomicity; durable resubmission/restart;
remaining HTTP precedence and pool lifecycle/state queries; later Byron work.
Keep all conformance and provenance checks intact.

The required trusted Haskell key-oracle executable digest remains unreproduced;
the complete rollback/full-conformance controls remain blocked on it. Exact
digests, Nix pins and prior investigation are in the reboot checkpoint.

## Private services and reproducibility

Current marker: `/tmp/quetzal-final-workspace`, run directory
`/tmp/qz-1107-final-t2xArg`. Candidate port 19092, reference 19093 (UI 19113),
faucet 19079, node socket `RUN/node.socket`. Frozen executables `quetzal-v21` and
`adapter-v21` are inside RUN; databases/logs remain outside the repository.
Owned PID files: `cluster.pid`, `candidate.pid`, `reference.pid`.
Tool sessions: cluster 92783, candidate 79489, reference 45997.

Earlier resume/quality/clock service generations were stopped after verifying
ownership; logs/databases remain retained. Canonical fee mnemonic has now been
funded on this cluster. A future independent delegation-fee replay requires a
fresh cluster, not just a fresh wallet DB, because that profile requires its
fixed address to start empty. Never overlap HTTP list-wallet suites and funded
workflows. Freeze binaries and use one target/environment for a combined gate.

Source backup: `/home/sam/.codex/tmp/quetzal-verified-v21/crates`.
The earlier recovered source-edit incident is documented in the resume
checkpoint; no work was lost. Two tool-invocation failures are retained:
`../20261107-balancing/failed-adapter-path-attempt` (wrong working directory)
and `failed-cli-path-attempt` here (mistyped CLI path, before funding). They are
excluded from the fresh gate. V18/V19/V20 functional failures remain preserved.
