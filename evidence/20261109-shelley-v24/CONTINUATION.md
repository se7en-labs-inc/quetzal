# Quetzal continuation — 2026-11-09, V24

Goal remains all 643 obligations, Shelley before Byron. User explicitly approved
autonomous continuation without repeated routine approval. The conformance
suite must remain unchanged. Worktree `wallet-migration`, branch
`001-wallet-rust-backend`; all existing changes preserved, no commits/staging.

Frozen V24 complete nine HTTP and sixteen internal profiles plus expanded funded
Shelley replay cover **371/643**, exhaustive false. HTTP **391/537**; internal
**989/1241**; funded **528/537**. Do not claim completion. Full controls catalogue
has not passed. Workspace tests, Clippy, formatting and release build pass V24.

V22 fixed balancing input framing above23 and raw body identity in signing,
submission, decode and pending IDs. Its funded replay passes419/419 including
23/95/113/117 input acceptance and118 rejection. V22 tests cover23/24/255/256
encoding and exact signed size. V22 HTTP replay followed funded execution and
its canonical key fixture therefore encountered pre-existing funds; its gate is
329/643, not a comparable clean baseline. All reports retained.

V23 shared activation/signing funded profile passes121/121. V24 additionally
fixes stake-script address indices (verified against actual pinned reference),
own-account-key validation before storage, key encoding and POST key derivation,
shared transaction history and native-script missing witness counts. Cosigner
activation is atomic across template/address/index trees, serialized across
patches. Shared payment construction includes native scripts and signing uses
CIP1854. V24 expanded live profile includes shared, voting, precedence, metadata,
assets, pool-join-guard and external signer as well as earlier successful flags.

V24 failures: empty pool cache times out pool join guard; voting remains synthetic;
precedence exact envelope; shared metadata decode expected500 but returns202;
mint/burn construct501; watch-only decode400. Internal failures remain seeded
selection157, credentials61, wallet-selection/mint-burn34. Preserve trusted
key-oracle digest checks; missing oracle provenance remains unresolved.

Current source has advanced after V24: NoContent endpoints ignore unsupported
Accept headers; duplicate errors include wallet ID; separate Byron service/store
under `database/byron` and worker map prevents same-ID Shelley/Icarus collisions.
This namespace change is compiling/testing in `../20261109-namespaces/` and is
not yet frozen or replayed. AppState::new now returns Result. Router binds Byron
subtree to its family state, startup restores both worker registries. Existing
prototype Byron data is retained in its old store; migration still needs work.
Continue by testing same-ID isolation/restart, replaying HTTP families, and
investigating remaining failures. Do not combine post-V24 source with V24 evidence.

Private V24 run marker `/tmp/quetzal-v24-workspace`:
`/tmp/qz-1109-v24-1QQW0o`, candidate19092/reference19093 UI19113/faucet19079.
Frozen binaries `quetzal-v24`, `adapter-v24`. Owned PID files in run directory.
Cluster session63139, candidate85095, reference5596. V24 fixed fee mnemonic is
now funded, so repeating that profile requires another fresh cluster.
Earlier V22/V23 run `/tmp/qz-1109-encoding-FB1yFY` still retained/running:
cluster89038, candidate77434, reference42835, shared candidate9091.
Old V21 services were stopped after verifying process ownership.

Nix binary pins/source inventory and historical recovery remain documented in
the V21 and reboot checkpoints. Never scan `/nix/store`; use exact pinned paths.
