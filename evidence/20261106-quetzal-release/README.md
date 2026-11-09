# Quetzal release compatibility audit — 2026-11-06

**The wallet is incomplete and does not conform.** The strict gate covers
83/643 obligations, with 560 missing and 1,442 failed case observations.
See `coverage.json`. These are fresh candidate reports, not reference
qualification or harness self-tests.

| Check | Result |
| --- | --- |
| Workspace tests through the project Nix shell | 39 passed |
| Release workspace build through the project Nix shell | Passed |
| Nix package build and packaged adapter smoke test | Passed |
| Formatting | Passed |
| Clippy with warnings denied | Failed; see `clippy.log` |
| Nine differential HTTP suites | 157/537 passed |
| Sixteen internal profiles | 186/1,241 passed |
| Basic real-node workflow attempt | 4/11 passed; stopped before funding |
| Strict active catalogue gate | 83/643 covered; incomplete |

The basic lifecycle suite passes 11/11 and the independent metadata-signature
suite passes 4/4. These successes do not establish funded spending, restoration,
rollback, complete key/encryption compatibility, or API parity.

## Runtime and provenance

- Harness checkout: adjacent `cardano-wallet-conformance/master`, commit
  `b733b0140386ab104a16185d7e7024f4678f7946`.
- Contract source: Haskell wallet commit
  `2fc5c383c94bb907deaa0cd27b1b407f86224b6c`.
- Environment: `quetzal-node11.1.3-release-v1`.
- Candidate: `target/release/quetzal`; internal adapter:
  `target/release/quetzal-conformance-adapter`.
- Native node 11.1.3, cardano-cli 11.2.3.0 and the pinned Haskell wallet and
  local-cluster executables were used, with network magic 42.
- Exact executable digests are in `binary-identities.sha256`; suite digests
  and assertion receipts are embedded in each report.
- The Nix package was built from a temporary source snapshot containing the
  current manifests, lockfiles, crates and flake modules. This avoids Git flake
  filtering of unstaged crate renames without staging the user's changes.
  `nix-package.out` records its output path. Both binaries are installed, and
  the packaged adapter passed the independently frozen Shelley root vector.
  Full conformance reports used the Cargo release binaries identified above;
  the package smoke test is recorded separately.
- The reference source was resolved with `nix flake metadata` at the contract
  revision. Its exact cluster configuration directory supplied the native
  fixture. There was no scan of `/nix/store`.
- Candidate and reference used separate fresh databases, on ports 18792 and
  18793; the owned faucet used port 18779. All three owned foreground services
  were stopped after testing. Private runtime files remain outside this repo.

The HTTP runner replayed every `http` entry in `fixtures/execution-profiles.json`
against the candidate and the Haskell reference on the same node. The vector
runner replayed every internal profile with its exact corpus and operation
filters against the candidate's real-function adapter. Unsupported adapter
operations returned explicit errors; expected answers were never implemented
inside the adapter. No oracle-only reports were included in the gate.

The separate `live` invocation independently observed block advancement, but
failed transport cases and the `getBlocksLatestHeader` response contract
(`/slot_no`). It stopped before the funded transaction workflow. The remaining
owned restart, rollback, era and other live profiles were not run. The complete
37-profile `conform` entrypoint has not passed or been represented as passing.

## Remaining implementation work

1. Replace placeholder transaction construction/signing/submission with actual
   era-aware CBOR transactions, ledger fees and node acceptance. The current
   submission handler stores pending bytes without submitting them to the node.
2. Complete chain accounting, restoration from genesis/checkpoints, durable
   pending/retry state, atomic block updates and rollback of spent outputs.
3. Implement Byron/random, hardware/watch-only and shared wallet behavior, plus
   the missing address, script, encryption and credential adapter operations.
4. Complete native assets, delegation/rewards/governance and real node queries;
   several caches and network-parameter responses remain placeholders.
5. Finish the status/media/error-precedence matrix and the remaining response
   schemas; resolve Clippy failures without hiding missing functionality.
6. Run all 37 fresh profiles through `conform` and require a complete gate.

Earlier attempts are kept in sibling evidence directories. Do not combine their
different candidate versions/environments into a new conformance claim.
