# Reproduce the owned individual profiles

Keep `cardano-wallet-conformance` read-only. Build its locked runner and library
with an external `--target-dir` in this Quetzal checkout. All public commands below
use the exact frozen runner/adapter and binaries recorded in `binaries.sha256`.

The small Rust driver in `individual-profiles.rs` calls the unchanged runner's
public CLI. It reproduces the standard/committee/owned command arguments from
`src/executable.rs`; each exact command and exit is retained under
`individual/commands/`. It does not implement candidate behavior or alter tests.
It uses the conformance library only for exclusively owned fixture/process
lifecycle. Compile with the project Nix shell:

```sh
nix --quiet develop --command rustc --edition=2021 \
  -L dependency=target/conformance/debug/deps \
  --extern wallet_conformance=target/conformance/debug/deps/libwallet_conformance-6288d856a69cc1fd.rlib \
  --extern anyhow=target/conformance/debug/deps/libanyhow-57cee97fa7b587ba.rlib \
  --extern serde_json=target/conformance/debug/deps/libserde_json-46f6fdb03231ad8b.rlib \
  evidence/20261110-replacement/individual-profiles.rs \
  -o target/replacement-individual-profiles

target/replacement-individual-profiles standard
target/replacement-individual-profiles committee
target/replacement-individual-profiles owned
target/replacement-individual-profiles disconnect
```

Use a fresh private root (update the driver's ROOT constant) for repetition. The
standard profile's closed base cluster supplies the fork configuration for owned
rollback/era cases. The initial standard root had mode0755, so disconnect setup
correctly refused it before candidate execution. The corrected driver sets its
owned root to0700; the failed setup receipt remains separate from the repeat.
All profiles use unchanged public options and deadlines. The driver records and
continues after a reported failing profile; its process exit alone does not mean
all invoked profiles passed. Inspect individual exits and reports.

`transport-profile.rs` invokes the public `transport::check` from that same frozen
library, with newly generated private PKI and the same candidate. `native-eof.rs`
uses independent node/mempool observations and a verified Reqwest mTLS client;
its fixture helper dependencies are recorded in `helper-dependencies.sha256`.
`sync-options.rs` is an inconclusive supplemental bracket probe, not a qualified
lagging/ahead test. Its failed attempt must not be counted as passing.

Public receipts only are copied into this evidence directory. Runtime workspaces
under `/tmp/qz-repl-*` contain sensitive keys, wallet data and mnemonics and must
not be published. No report from an individual run is imported into the aborted
full catalogue cohort.
