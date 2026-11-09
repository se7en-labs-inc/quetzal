# Proposed input-set encoding correction

Approved by the user's "yes" and subsequent instruction to continue through all
obligations without repeated routine approval. Implementation and validation are
underway; fresh evidence is in `../20261109-input-encoding/`.

The funded size-boundary replay passes the 23-input probe but rejects the returned
95-input transaction's framing. The pinned reference writes definite arrays for
up to 23 inputs and indefinite arrays above that threshold. Pallas 0.32.1 `Set<T>`
stores a `Vec<T>` and always writes a definite array; it has no framing selector.
`MaybeIndefArray` exists separately and cannot be installed in a transaction body's
typed `Set` field without changing the dependency's transaction type.

The proposed correction keeps Pallas transaction/input types and uses its CBOR
encoder to replace only body field 0's array framing after normal serialization.
Other body fields retain their encoded bytes. The signed-size calculation runs
on the corrected bytes so fee and maximum-size decisions include the actual
encoding, including the 255/256-element boundary.

Signing must hash and preserve the exact body bytes. Currently API signing,
submission checks, decoding and pending IDs recompute the hash from the normal
decoded body, and signing re-encodes that body. Introduce shared wallet helpers
for raw body identity and witness-only replacement using the existing
`txsize::Parts`/writer machinery. Use them consistently in the API and pending
store so unsigned/signed/submitted/confirmed transaction identities agree.

Expected production paths: `wallet/src/balancing.rs`, `wallet/src/writer.rs`,
`wallet/src/ledger.rs` or `txsize.rs`, `wallet/src/wallet.rs`, and API transaction
handlers. This does not change the harness, oracle digests or selection algorithm.

Validation: 23/24/255/256-input framing and exact size; native-asset conservation;
body-byte preservation through first/repeated signing; raw ledger ID agreement
in pending history and node confirmation; full writer vectors; workspace,
Clippy, formatting and release checks; fresh funded input-limit replay against
the frozen candidate. Retain all earlier failed reports.
