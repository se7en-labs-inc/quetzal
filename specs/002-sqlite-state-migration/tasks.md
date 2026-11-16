# Tasks: SQLite state migration

- [ ] M001 Resolve transfer-versus-rescan semantics and finish version/family/key/discovery/history/submission mapping from actual pinned schemas and records.
- [x] M002 Implement standalone Rust CLI, source inventory, private WAL-aware stable snapshot, version/schema validation and read-only inspection.
- [x] M003 Add validated credential/public-key/family/discovery/shared import boundary with preserved timestamps and namespaces.
- [ ] M004 Implement selected chain/history/staking/submission transfer or explicitly agreed rebuild policy, with safe rollback handling and no silent loss.
- [x] M005 Stage, flush, verify reopening and atomically publish a fresh destination; emit non-secret verification receipt and preserve source bytes on every path.
- [ ] M006 Verify real pinned-reference SQLite wallets, legacy/V2/watch/shared families, malformed and interrupted imports, exact API/key/state parity, funded native pending/restart/spending and rollback behavior.
- [ ] M007 Integrate Nix tool/package support, document exact commands and limitations, and run formatting, strict Clippy, tests and release/package checks.
- [x] M008 Correct payment-only discovery fingerprint mapping for funded Shelley/shared imports; retain strict ownership/duplicate guards and verify all five funded SQLite families. Native attempt 10 also passes exact API parity, restart, retained pending confirmation and new spending.
- [x] M009 Preserve historical validity intervals, confirmed expiry and canonical fractional timestamps; verify strict funded API/restart/spending parity.
