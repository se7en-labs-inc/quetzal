# Local conformance against Quetzal master — 2026-11-17

The corrected candidate passes the current approved local gate: **651/651
obligations, 43/43 profiles, 3,926 checks, zero failures**, in 1,663 seconds.
`attempt2/result.json` records complete=true, all_attempts_passed=true,
identities_unchanged=true and no setup/execution error. No reports were imported.

Candidate source is master `719daea861a6655710ec36d988a14316b7392ed2` plus the
uncommitted four-file correction in `network-fix.patch`. The independent runner
is conformance `e644f74cc3efd8d94f66c6b5670691e58c43a33d`, suite fingerprint
`30a6aadbf988727b7acf8347e66c72cbbe2da1ef5035f7cefb93f4f0f4554df8`.
The fixed release is `/nix/store/w6cxjb3l90wllaniabadsj6g21d3w1l8-quetzal-0.1.0`.
Source patch and executable hashes were verified unchanged after the full run.

Unchanged master initially covered 650/651 obligations: 43 profiles, 3,926
checks, one failure, in 1,651 seconds. Its sole failure was
`network.local-node-recovery / sync:disconnect`, “reference sync progress
differs.” Complete original receipts remain in `attempt1/`.

Quetzal previously forced `not_responding` whenever a fresh node query failed.
The reference retains its observed native tip and interpreter and calculates
progress using current time. The correction retains the last successful native
snapshot across disconnects and namespaces, recalculates progress, and accepts
subsequent native observations including rollbacks. A cold start without a
native observation still reports `not_responding`; historical replay cannot
promote readiness. No persisted format changes were needed.

Validation includes two focused API regression tests, formatting, strict
workspace/all-target Clippy, and a successful Nix release build/test phase.
The independent isolated recovery receipt in `focused/` passes its actual
fault observations. That isolated command intentionally remains incomplete
(4/5 obligations) because funded pending EOF needs the full owned profile.
The fresh complete cohort in `attempt2/` supplies all 43 profiles independently;
no focused or earlier receipts were stitched into its result.

Both full runs use the passing pinned baseline's data and 180-second component
deadline. The unavailable fixture source path was replaced with Quetzal's local
copy only after all 630 retained pinned checksums passed. Candidate launch paths
and distinct loopback ports were configured for each run. No independent
conformance source, expectations or policy were changed; its initial user-owned
changes were preserved exactly. The current local contract separates extended
mainnet, genuinely lagging/ahead native sync and large-history qualification.
These results do not relabel yesterday's 652-obligation/44-profile receipts.

Cleanup receipts confirm owned nodes and wallets stopped. Final process audits
found no remaining owned processes for either full cohort or the focused run.
Only public receipts are saved here. Private workspaces `.c17`, `.c17p`, `.c17b`
and restart context remain under the Quetzal family with restricted permissions;
their keys, mnemonics, PKI and databases must not be published.
