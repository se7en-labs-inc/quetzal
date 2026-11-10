# Mutual TLS correction — complete conformance

The unchanged independent runner completed a fresh owned cohort successfully:
**646/646 obligations, 3,808 checks, zero failures, all 38 profiles.** All
attempts passed, binary identities remained unchanged, setup/execution error is
null, and the elapsed time was 1,544 seconds.

- [Overall result](result.json), [strict coverage](coverage.json), [all reports](reports/)
- [Attempts](attempts.json), [run binding](run.json), [binary identities](identities.json)
- [Configuration](execution-config.json), [validated contract](validated-specification.json)
- [Standard cleanup](standard-cleanup.json), [committee cleanup](committee-cleanup.json), [final process audit](cleanup-audit.json)

The suite fingerprint is
`b7370b72b8e87e3b941b20cfd4ae98eefa280f97e4c53668ce04a3a0f831a58c`.
Environment: `quetzal-mtls-20261109`. The cohort's HTTP target is
`http://127.0.0.1:19099/v2/`; its separately owned secure listener is
`https://127.0.0.1:19101/v2/`, recorded explicitly in the attempts receipt.

| Group | Passing checks |
| --- | ---: |
| Mutual TLS / server trust / plaintext rejection | 9 |
| Differential HTTP | 537 |
| Independent internal vectors | 1,241 |
| Funded operations and restart | 939 |
| Node disconnect/reconnect | 82 |
| Registered DRep lookup | 6 |
| Pool lifecycle | 120 |
| Parameter enactment | 149 |
| Pending/deletion | 56 |
| Expiry/mixed history | 79 |
| Both Byron pending styles | 180 |
| Automatic retry | 108 |
| Checkpoint recovery | 29 |
| Native rollback/reapplication | 224 |
| Babbage-to-Conway transition | 49 |
| **Total** | **3,808** |

The transport report contains seven positive/negative probes plus service and
cleanup observations. It requires client authentication, rejects absent,
foreign-CA and wrong-purpose client credentials, verifies server trust/name and
rejects plaintext. Every negative is bracketed by authenticated API success.
The same candidate executable is used for HTTP and TLS launches.

## Local and package checks

All 85 executed workspace tests pass; the existing standalone private-node test
remains ignored by default. Real-node qualification is covered separately by the
owned conformance profiles above. Formatting, denied-warning all-target Clippy,
release build and the Nix package build pass.

The initial regression using Reqwest intermittently surfaced HTTP dispatch
cancellation instead of the certificate alert. Preserve [that failure](workspace-tests-alert-race.log).
With the user's approval, the regression now reads rejection alerts directly
through Rustls, without accepting timeouts, EOF or generic connection errors.
It repeats 30 client-certificate rejection probes with authenticated brackets.
The independent runner and production transport were unchanged by this test fix.

See [workspace tests](workspace-tests.log), [Clippy](clippy.log),
[formatting](format.log), [Nix output](nix-package.log) and
[Nix build/test log](nix-package-build.log). The first Nix expression's
[string-context error](nix-package-invocation-failed.log) is retained; correcting
the expression allowed the same filtered working-tree source to build, including
the new untracked transport module. The resulting package is
`/nix/store/br0qy0hwymivzfcsjbks4j7hddxafpwz-quetzal-0.1.0`.
Conformance used the independently measured local release binaries, not this
separately built package. [Hashes](binaries.sha256) identify each executable.

## Provenance and limits

The runner was compiled from the current external conformance checkout with
`--locked` and outputs under Quetzal's ignored target directory. No conformance
repository files or assertions were modified. No earlier reports were imported.
The original V58 evidence remains unchanged and qualifies only its historical
643/37 contract.

Native tools retain the independently pinned identities in the manifest. Cluster
configs came from wallet commit `2fc5c383c94bb907deaa0cd27b1b407f86224b6c`.
The actual Haskell key oracle was rebuilt through the pinned Nix compiler recipe
and measured as `337e2b28428a1a241ff6d27bae025069c619fb5a5836c55d25b4ae4ed2122993`.
[Reference source hashes](reference-sources.sha256) and [current source hashes](source.sha256)
are retained. Only test probe code changed after the local release binary was
frozen; production code remained unchanged throughout the cohort.

All owned processes were cleaned up. The private runtime, generated PKI and
wallet databases remain outside this evidence directory. Only public receipts
are retained here. The directory name follows the task date; UTC execution times
are recorded in run.json.

This qualifies the finite corrected contract. Actual pinned Daedalus startup,
certificate provisioning, other platforms and defined performance benchmarks
remain separate acceptance work. API semantic fixtures run over HTTP separately
from secure-listener checks; this does not claim every operation was replayed
over TLS or qualify another binary/package by association.
