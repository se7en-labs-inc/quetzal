# Tasks

- [x] F006 Required extended Ed25519 funding keys: validated Pallas loader/signing,
  positive and malformed-layout/signature tests, real fixture source verification.

- [x] F001 Implement configuration, key/address validation and bounded HTTP contract.
- [x] F002 Implement Pallas UTxO queries, balanced/signed Conway payments and submission.
- [x] F003 Persist reservations and verify concurrency, task cancellation and restart behavior.
- [ ] F004 Add documentation and workspace/Nix packaging; run fmt, Clippy and tests.
- [x] F005 Integrate migration fixture and verify native Shelley/Byron funding; report migration qualification independently.

2026-11-16: Ten faucet tests, explicit verification of all 20 real extended-key
funding sources, formatting, strict workspace Clippy and release Nix packaging
pass. Workspace testing
stops at the existing API TLS shutdown test, which times out on both initial and
isolated runs. Extended keys are now required and implemented, including Byron
source-root validation and bootstrap witnesses. Native attempt 5 exposed and
corrected an era-tag offset. Attempt 6 accepted four 30 ADA transactions from
real extended-key Byron sources and verified funded balances for all five wallet
families. Migration then failed on `unowned or duplicate UTxO`; complete migration
qualification remains open. F004 stays open for the unrelated workspace TLS gate.
