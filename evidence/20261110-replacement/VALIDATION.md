# Reference replacement validation — 2026-11-10

The frozen drop-in executable is `target/daedalus/cardano-wallet`, byte-identical
to `target/release/quetzal`. [Binary hashes](binaries.sha256) pin the candidate,
adapter, independent runner and package. The Nix package
`/nix/store/1278kfn3pki16xbc9vhjf4fph99ih01b-quetzal-0.1.0` installs
`bin/cardano-wallet -> quetzal`. The separately built package has its own hash;
its 99 tests pass. No argument-filtering wrapper or conformance edits were used.

## Implemented and locally checked

The parser implements the pinned reference serve inventory, required network and
socket selection, conflicts/defaults, literal host specifications, fixed/random
API/UI ports, temporary/persistent databases, TLS, EOF, logging and configured
pool/token/IPFS metadata. The optional UI is a functional read-only Quetzal view.
Native GetInterpreter and GetGenesisConfig queries supply actual era timing,
security and active-slot coefficient without requiring a sibling genesis filename.
Network information queries the actual connected node independently of global
historical replay. Progress uses configured tolerance and reference rounding.
Registry queries validate required/optional properties, isolate subjects, bound
requests/responses and retain raw ledger quantities.

[Final local checks](qualified-checks.log): **99 passed**, zero failures, one
existing private-node test ignored; workspace formatting, strict all-target
Clippy and release build pass. [Final package tests](nix-package-final-tests.log)
and [build](nix-build-final.log) pass. Tests exercise real HTTP/mTLS subprocesses,
fixed/random API/UI listeners, persistent and temporary databases, ordinary stdin
bytes, held pipes, immediate/normal EOF, flag absence, signals and stalled TLS.

Earlier attempts remain visible: missing required parser fixture inputs corrected
without dropping assertions; Clippy parity spelling corrected; a 3-second wallet
creation client timeout under concurrent builds passed on an unchanged repeat
after load subsided. The first external attempt found help prose inventory and
absent-metadata `null` issues; both were fixed before freezing the final candidate.
Initial receipts/hashes are retained under `attempt1/` and `binaries-attempt1*`.

## Independent replacement profiles

The unchanged updated runner contains **652 obligations / 44 profiles**. Its
[isolated result](replacement/result.json) is **complete=false**, with unchanged
identities and successful owned cleanup:

- CLI inventory/help/version/conflict/option behavior: **5/5 passed**.
- Configured registry list/detail known/optional/missing/malformed/unavailable/
  disabled cases: **9/9 passed**, with reference-prepared and independently
  native-confirmed assets, same-name/different-policy isolation, request traces
  and unchanged quantities.
- EOF profile: **7 executed cases passed**. Its funded pending case is reserved
  for the owned pending profile; the empty-wallet profile gets no such credit.
- Launch profile: five executed cases passed; mainnet and lagging/ahead tolerance
  effect are unexecuted without the required fixtures.
- Partial-node and representative-history profiles remain incomplete. The
  reference disconnect bracket also crossed a wall/node-tip boundary. None of
  these missing cases is counted as passing.

[Supplemental funded EOF](native-eof.json) independently verifies the full public
launch over mutual TLS, configured registry and both listeners. Latest acknowledged
names are written immediately before EOF without a delay/readback. Shelley and
Icarus confirmed/pending state and public keys recover exactly at an unchanged
paused native tip; both pending IDs remain in the native mempool and independently
confirm after resume without another submission API call. Both EOF exits took
10 ms; API/UI ports were released and owned native processes stopped. This
receipt is separate from the external gate.

## Regression orchestration

The full `conform` attempt aborted with `File exists (os error 17)` while replacing
precreated acceptance reports. Its [result](full-aborted/result.json), all reports,
attempts and unchanged identities are retained separately. All 16 internal profiles
passed; its 39 failed placeholder checks are unexecuted requirements. See the
[concrete read-only harness handoff](HARNESS-BLOCKER.md). The aborted cohort is
never relabeled complete or combined with passing reports from another run.

With explicit user approval, `individual-profiles.rs` invokes the same unchanged
public commands against exclusively owned native fixtures. Commands, exits and
public reports are retained under `individual/`. The mutual-TLS helper calls the
runner's unchanged public transport checks; [transport report](transport-mtls.json)
passes all certificate and plaintext probes and owned-service cleanup.

At the user's requested immediate commit, individual results are: all nine HTTP
profiles537/537, funded live964/964, DRep6/6, pools134/134, parameters163/163,
normal pending64/64 and expiry79/79 passed. Byron pending, adversarial retry,
checkpoint, rollback, era transition and the disconnect setup repeat are still
running or queued. These remaining profiles are not yet claimed as passing.

Supplemental `sync-options.rs` could not capture a stable reference/native-tip/
wall-clock bracket in 60 seconds and is **inconclusive**, not a pass. Its earlier
adapter attempt incorrectly retained Content-Type parameters; the adapter was
corrected to match the runner's normalization. Both failed logs remain. No
candidate or assertion was changed in response.

## Explicit qualification limits

The user deferred mainnet/lagging/ahead and representative historically funded
100,000-block / 1-GiB fixture provision. No large-chain first-initialization,
same-database service-follower restart or reconnection performance is claimed.
The global pool-history follower still replays; no durable global intersection
was introduced without the required rollback/performance evidence. Wallet
checkpoint recovery is distinct from that global follower workload.

Actual Daedalus UI launch/use is not recorded here. The supplied executable is
ready for that test, but these finite checks do not establish full application or
production qualification. The current full independent gate remains incomplete.
Fixture databases, mnemonics, TLS keys and full runtime workspaces stay private.
