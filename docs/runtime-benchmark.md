# Runtime benchmark

`quetzal-runtime-bench` is an opt-in Rust executable. Normal builds and wallet
packages exclude it. It validates responses as well as recording timings; a fast
error cannot pass as a successful read.

Build and create disposable synthetic scale state:

```sh
nix develop -c cargo build --release -p quetzal-api \
  --features runtime-bench --bin quetzal-runtime-bench

target/release/quetzal-runtime-bench fixtures /absolute/private/root/fixture
```

The parent must exist and the fixture destination must not exist. Use a private
directory on the filesystem being measured. The default fixture contains five
families, each with 1,000 UTxOs, 10,000 CBOR-backed historical records, 200 used
addresses and one pending reservation. `--small` creates a test-sized fixture.
Fixtures use deterministic public test entropy and a public test password;
they must never receive real funds. The fixture builder reopens state and checks
available/total balances, history, addresses and UTxOs before publishing it.
Watch-only credentials stay absent. Random Byron has no invented internal-role
allocation. These records are synthetic; their checkpoint is not a native node
checkpoint. Do not attach this synthetic store to a real node and call it a
funded native replay experiment.

`fixture/database` is the sled store, `fixture/manifest.json` describes public
expectations, and `fixture/byron-genesis.json` selects the fixture's test network.
Launch the selected wallet executable yourself against a disposable copy of
that store. For an offline smoke test, use an absent node socket, the generated
genesis, `--random-port`, `--shutdown-handler`, and a loopback listener. Capture
its reported address and process ID, and stop/join that owned process afterward.

Run the driver against that address:

```sh
QUETZAL_BENCH_PID=12345 target/release/quetzal-runtime-bench run \
  http://127.0.0.1:8090 /absolute/private/root/fixture/manifest.json \
  /absolute/private/root/receipt.json \
  1000 60 50 node-unavailable SOURCE_AND_BINARY_IDENTITY
```

Arguments after output are measurement seconds, warmup seconds, requests/second,
scenario and source identity. Defaults are 1,000/60/50 and `unqualified` labels.
Output must not exist. The driver sends PUT name updates to the first Shelley
fixture and derives its public account key; never aim it at personal wallet state.
The steady-state mix is 20% summary, 10% wallet list, 20% network information,
20% history limited to 100, 10% complete address lists, 10% complete UTxO lists,
5% rename and 5% expensive account derivation. The shared API has no UTxO route,
so that class uses the Shelley fixture rather than counting routing failures.

For mutual TLS, set `QUETZAL_BENCH_CA` to a CA PEM file and
`QUETZAL_BENCH_IDENTITY` to the client certificate plus private-key PEM file.
The driver does not disable TLS verification or follow redirects.

Every scheduled request is recorded from its scheduled start through complete
body receipt/validation, including client queue delay. There are at most 64
retained jobs and 16 active connections. Hitting the client bound is recorded as
a failure, not omitted. Response bodies are limited to 32 MiB in this driver;
this does not introduce a new wallet API response limit. Per-family/endpoint
histograms retain failed samples and expose separate failure counts. Preflight
checks every route and expected wallet identity/balance/count/key before load.
Linux resource sampling records RSS, thread count, CPU ticks and kernel I/O bytes
once per second; inaccessible observations remain null. No response bodies,
passwords or private keys are written to receipts.

A zero exit code and `all_responses_valid` establish only that this workload
completed correctly. The tool deliberately writes `performance_qualified: false`
and `native_sync_qualified: false`: qualification also requires the repeated,
quiet-host and native/fault scenarios in the feature's
[performance contract](../specs/005-responsive-runtime/contracts/performance.md).
Short smoke tests establish harness behavior, not latency acceptance or a
baseline/candidate improvement. Do not compile or run other tests during timed
qualification trials.

For a low-load diagnostic against a real Preview instance, use the read-only
probe instead of the synthetic fixture driver:

```sh
target/release/quetzal-runtime-bench probe http://127.0.0.1:8090 \
  /absolute/private/root/preview-probe 12 1000 10000
```

The optional numbers are rounds, interval between rounds in milliseconds and
each request's deadline in milliseconds. The output directory must not exist.
At most four GET requests run concurrently. Each round checks network
information, parameters, the latest header, settings and all three wallet lists;
it samples up to five wallets per list for summary, addresses, history and
supported UTxO reads. Use a stable, disposable wallet set. The probe records
request starts and completions immediately in `events.jsonl`, so a stalled
endpoint cannot hide completed control requests. `summary.json` records latency
distributions and failures. HTTP errors, invalid responses, response bodies over
4 MiB and deadlines cause a nonzero exit after receipts are saved.

Receipts omit wallet IDs and response bodies and retain only selected network
and wallet progress fields. Confirm the node's native tip separately with
`cardano-cli query tip --testnet-magic 2 --socket-path SOCKET`; API network magic
alone does not establish a live connection. The same optional TLS environment
variables apply. This diagnostic needs no Daedalus integration. Empty disposable
wallets exercise restoration and reads, but do not establish funded-history
coverage or performance qualification.
