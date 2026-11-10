# Stdin EOF shutdown

The literal valueless `--shutdown-handler` flag is implemented. An open stdin
pipe and ordinary bytes leave the service running; EOF requests clean shutdown,
including immediate EOF. Without the flag, closed stdin is ignored. SIGTERM and
Ctrl-C share cleanup, but signal exits do not count as EOF qualification.

The service stops accepting connections, drains admitted HTTP/TLS requests,
cancels handshakes, joins global and wallet workers, and flushes Shelley/shared
and Byron stores. Cleanup failures return failure rather than a clean-exit claim.

## Local and package checks

- `nix --quiet develop --command cargo check --workspace`: pass.
- `nix --quiet develop --command cargo test --locked --workspace`: **88 passed**,
  zero failures. One existing standalone private-node test is ignored; the native
  EOF check below supplies separate real-node evidence.
- Workspace formatting and `git diff --check`: pass.
- `nix --quiet develop --command cargo clippy --locked --workspace --all-targets -- -D warnings`: pass.
- `nix --quiet develop --command cargo build --locked --release --workspace`: pass.
- The filtered working-tree Nix package builds and runs the same 88 tests in
  release mode. See [build output](nix-build.log) and [test log](nix-package-tests.log).
  Output: `/nix/store/n6j9qjhcrpz7fkkk4qd6q6w6w1i7pvc7-quetzal-0.1.0`.
  The existing crane/nixpkgs version warning is retained in the build output.

The subprocess matrix owns the real executable, pipe and database. For both
HTTP and mutual TLS it checks open pipe, ordinary bytes, EOF, immediate EOF,
flag absent, bounded successful exit, completion log, listener release and
same-database metadata/addresses/history/UTxOs. It also verifies SIGTERM with
stdin kept open, and TLS shutdown with a stalled ClientHello. A transport test
holds an admitted request open and requires shutdown to await its completion.
Kill/reap exists only as failure cleanup and cannot earn an EOF pass.

## Funded native EOF recovery

The [Rust driver](native-eof.rs) and [public receipt](native-eof.json) qualify
funded Shelley and Icarus Byron wallets over strict mutual TLS. The native
factory, node and CLI are the independently pinned tools from
[the configuration](execution-config.json); fixture ownership and query helpers
come from the unchanged external conformance library. Their
[source hashes](native-reference-sources.sha256) and [linked artifact hashes](native-dependencies.sha256)
are recorded. This is a
candidate-specific check, not an implementation of external T110.

The driver independently observes 300 ADA of funding, bridges 100 ADA into the
Byron wallet, and confirms outgoing payments from both families by original ID
and exact destination/value at the node. It pauses exclusively owned producers,
submits another payment from each family, and proves the IDs are in the node's
mempool with no corresponding ledger outputs.

Actual stdin EOF then exits successfully and releases the port. Restart uses the
same database and TLS settings while the ledger remains paused. Identity, names,
balances/assets, passphrase timestamps, complete address lists, UTxOs and stable
confirmed/pending history match exactly; the Shelley account public key matches.
The independent node still holds both pending IDs and its tip is unchanged.
After producers resume, both original IDs confirm independently, without any
submission API call after restart and without duplicate history entries.

Both EOF shutdowns finished in **10 ms / 20 ms**. Owned cluster cleanup passed.
Generated keys, mnemonics and runtime databases remain private under `/tmp`.
[Executable hashes](binaries.sha256) distinguish the tested local release,
adapter, runner, native driver and separately built package; the package does not
inherit real-node qualification by association. [Source hashes](source.sha256)
identify the implementation and regression tests.

To compile the one-off driver against the already-built independent library:

```sh
nix --quiet develop --command rustc --edition=2021 \
  -L dependency=target/conformance/debug/deps \
  --extern wallet_conformance=target/conformance/debug/deps/libwallet_conformance-6288d856a69cc1fd.rlib \
  --extern reqwest=target/conformance/debug/deps/libreqwest-8ef75755dddf7036.rlib \
  --extern anyhow=target/conformance/debug/deps/libanyhow-57cee97fa7b587ba.rlib \
  --extern bip39=target/conformance/debug/deps/libbip39-a6f8b08553245302.rlib \
  --extern serde_json=target/conformance/debug/deps/libserde_json-46f6fdb03231ad8b.rlib \
  evidence/20261110-shutdown/native-eof.rs -o target/native-eof-check
target/native-eof-check
```

Dependency artifact names depend on the pinned build. The config is copied to
`target/shutdown-conformance.json` for this invocation. The driver requires fresh
private runtime `/tmp/quetzal-shutdown-native-20261110` and unused ports 19102 and
19193. It leaves earlier receipts intact on any failure.

## Existing catalogue regression

A fresh owned run of the existing **646-obligation / 38-profile** executable
catalogue finished against the same frozen local release. Its launch omits
`--shutdown-handler`: that runner deliberately closes stdin and has not yet
implemented T110. These counts cannot qualify EOF lifecycle behavior.

The [result](regression/result.json) is **incomplete: 643/646 obligations,
3,700 passing checks and two failures, 36/38 profiles passing**. Execution took
2,136 seconds with unchanged binary identities and no setup/execution error.
The [coverage gate](regression/coverage.json) names the missing resubmission,
history-filtering and Byron 500-status evidence. All [38 reports](regression/reports/),
[attempts](regression/attempts.json), [run binding](regression/run.json),
[identities](regression/identities.json) and specification exports are preserved.
No earlier reports or focused repeats were imported into this cohort.

The main funded profile passed 950 checks, disconnect 82, DRep 6, pool lifecycle
120, parameter enactment 149, standard pending/deletion 56, both Byron pending
styles 180, checkpoint recovery 29, rollback/reapplication 224 and era transition
49. The two failed profiles are described below; their incomplete workflows also
prevented some later assertions from executing.

The first cohort's expiry profile failed with `node did not advance past expiry`.
An independent CLI query still observed relay tip slot 306, while the producer
log reported [`NoLedgerView`](expiry-producer-no-ledger-view.log) beginning at
slot 367. The pending transaction's signed upper slot was 351. This is evidence
of a stalled native fixture; it does not establish a wallet expiry failure.
The unchanged expiry profile [passed all 79 checks](expiry-repeat.json) on a
fresh owned cluster, on distinct ports; [output](expiry-repeat.log) is retained.
The first failed cohort remains recorded. The focused repeat has its own target
and environment and is not imported to manufacture an all-pass full cohort.

The cohort's automatic-retry profile also timed out waiting for transaction
confirmation. Its producer reports the same [`NoLedgerView`](retry-producer-no-ledger-view.log)
condition. The unchanged retry repeat [passed all 108 checks](retry-repeat.json)
on a separate fresh owned cluster; [output](retry-repeat.log) is retained. Like
the expiry repeat, it is separate evidence and does not repair the first cohort.

All owned fixture processes were stopped and reaped. The standard and committee
cleanup receipts are retained under `regression/`; the native and focused-repeat
receipts include their own cleanup assertions. The final [process audit](cleanup-audit.json)
found no matching owned services. Final [identity checks](identity-check.log)
confirm the binaries and implementation/test sources stayed unchanged.

No external conformance repository files were edited. During this run the
external specification also gained broader T111 launcher/sync/metadata/durability/
performance acceptance work. This change implements the requested T110 EOF
behavior; neither this receipt nor the existing catalogue claims T111 completion.
Actual pinned Daedalus launch-and-use and performance qualification remain
separate acceptance work.
