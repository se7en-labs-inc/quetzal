# Rollback oracle provenance — resolved by suite-owner update

2026-11-09: the suite owner externally replaced the compiler-dependent fixed
digest requirement with recorded executable identity and stability checks.
We did not modify the suite. The actual source-built Haskell helper below now
passes the unchanged native rollback control. V58 completes the full suite:
643/643 obligations, 3,807 checks, zero failures across 37 fresh profiles.
Canonical receipts: ../20261109-v58-gate/VALIDATION.md and result.json.
The original blocker and build investigation below are historical evidence.

The unchanged rollback helper requires the original source-built Haskell
`wallet-key-reference` executable with SHA-256
`9326e291df8ccb064cd77740a3a7063cf84cc382d5dc4481be28a47680d44114`.
The user confirms that they do not have this artifact. The original recorded
location `/tmp/wc-keys-build/wallet-key-reference` is absent.

The fixed suite's `reference/KeyAdapter.hs` has the documented source digest
`5ebf8ed97a16cf3a15757e70e919d2d56ce8b9a43ddb34823fca716c4340d09c`.
Rebuilding through its documented GHC9.12.3 environment and flags yields
`337e2b28428a1a241ff6d27bae025069c619fb5a5836c55d25b4ae4ed2122993`,
including a repeat rebuild. Building through the Quetzal Nix development shell
yields `ed71f69d8fadce341f540468621cb87747fd21d505ee220a36b4f6b4c5948b4a`.
Neither is the required executable. Optimization, stripping and scratch-path
experiments also did not reproduce the original digest.

Rebuild logs and measured checksums are retained alongside this file. Exact
compiler environment:
`/nix/store/3f6zix5gy2f9gq7ihzsjp4qa578bp5yf-ghc-shell-for-packages-ghc-9.12.3-env`.
Build instructions remain in the unchanged conformance checkout's
`docs/reference-keys.md`, using the source above and the declared package set.
Historical qualification receipts record the original executable identity but
contain neither the executable nor a complete reproducible link environment.

Resolution requires recovery of the original artifact or enough original build
and link inputs to reproduce its exact identity. Any change to the suite's
trusted executable identity belongs to its owner and is outside this task.
Quetzal has not substituted either rebuild, modified the suite or bypassed its
guard. Rollback conformance cannot be claimed until that guard is satisfied.

The era-boundary helper does not depend on this executable; it uses frozen
public-key vectors and remains independently executable. Earlier continuation
notes incorrectly grouped era verification with the rollback blocker.

Fresh upstream verification (V56): fetching the exact immutable revision from
`git+https://github.com/cardano-foundation/cardano-wallet?rev=2fc5c383c94bb907deaa0cd27b1b407f86224b6c`
with the unchanged toolchain recipe resolves the same GHC environment above.
The raw rebuild repeats337e2b28…; all133 independent key/root/signature vectors
pass against that actual Haskell program. This is diagnostic evidence, not a
substitute for the missing rollback oracle. Building inside the conformance
Nix shell produceseaa674fb33d2fa1696a828ea4d54c73648d915fe66e795cf27fe66e373fb7cce,
with either relative or absolute source paths. The pinned wallet's own development
shell build is also being checked; logs remain adjacent to this file.

Final verification: `../20261109-v56-gate/rollback-blocked.json` records the
unchanged helper rejecting the fresh upstream rebuild before any rollback test
starts. The aggregate covers642/643 obligations; onlywallet.rollback is missing.
The optional full upstream developer-shell probe was stopped by the agent after
Nix identified552 builds, including unrelated Ruby/Mithril/HLS tools. Its resolved
shell derivation and dependency plan are saved; no hash is claimed from that
unfinished probe. The documented minimal compiler build completed successfully.
The suite and its trusted pin remain unchanged.
