# Specification quality and acceptance status

Reviewed 2026-11-10. Requirement quality is distinct from passing implementation
tests. This supersedes the original seven-story/17-requirement checklist.

- [x] All 78 paths / 96 operations and wallet families remain in scope.
- [x] Duplicate creation, create-and-send and migration side effects match the reviewed API contract.
- [x] Identity uses 20-byte reference digests and distinguishes families and public-key restoration.
- [x] Storage encryption does not replace mnemonic/root derivation.
- [x] Restoration requires historical funds; restart requires checkpoint reuse.
- [x] Rollback restores spent inputs and preserves local pending/retry provenance; expiry is separate.
- [x] Secure acceptance requires mutual TLS, positive/negative certificate controls, fail-closed startup and loopback default.
- [x] Historical 643/37 receipts do not qualify the corrected 646/38 contract.
- [x] Linux-only qualification, unsupported eras/scripts and database restoration boundaries are explicit.
- [x] Fresh full 646-obligation/38-profile qualification: 3,808 checks, zero failures, unchanged identities and owned cleanup; evidence/20261109-mtls.
- [x] Valueless shutdown-handler semantics, real HTTP/TLS lifecycle controls, admitted-request drain and same-database recovery; 88 executed workspace tests and Nix package tests pass.
- [x] Funded Shelley/Icarus confirmed and pending state survives actual EOF with independent node observations over mutual TLS; evidence/20261110-shutdown/native-eof.json. The external T110 executable gate is still pending; 646/38 counts exclude EOF qualification.
- [ ] Fresh all-pass catalogue cohort for the shutdown candidate. The recorded cohort covers643/646 with36/38 profiles passing; native expiry/retry fixtures stalled with NoLedgerView. Unchanged separate repeats pass79/79 and108/108. Preserve the failed cohort and keep repeats separate; evidence/20261110-shutdown.
- [ ] Actual pinned Daedalus launch, certificate provisioning and wallet workflow acceptance.
- [ ] Defined and executed reproducible performance and memory benchmarks.

Current replacement iteration (652 obligations /44 profiles):

- [x] Full pinned public serve parser and option effects implemented; CLI5/5 passes.
- [x] Native era/genesis queries remove the sibling-filename requirement; actual-node network reads are independent of global replay.
- [x] Configured registry behavior passes9/9 independent reference/native cases with exact ledger quantities.
- [x]99 local tests plus formatting, strict Clippy, release and final Nix package checks pass; actual cardano-wallet executable/alias supplied.
- [x] Full-launch mTLS latest-acknowledged and funded pending EOF recovery verified independently, with exact paused-tip recovery and original-ID confirmation.
- [ ] Current full catalogue qualification. The full runner aborts on its precreated-report collision; user approved unchanged public profiles individually. Keep these receipts separate.
- [ ] User-deferred mainnet, lagging/ahead and representative100000block/1GiB historically funded dataset qualification, including global follower performance.

No all-green production or drop-in compatibility claim follows from this checklist.
