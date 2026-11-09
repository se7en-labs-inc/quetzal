# Quetzal Wallet Backend Constitution

## Core Principles

### I. Safety-First Financial Software
Quetzal is production software managing real ADA funds. Every change must justify its risk against the existing stability baseline. Correctness and security take absolute priority over performance or convenience.

### II. Era-Aware Design
All domain types and logic must account for Cardano's multi-era protocol evolution (Byron → Shelley → Allegra → Mary → Alonzo → Babbage → Conway and beyond). Pallas provides era-indexed primitives; use them. Era-specific handling must be explicit — never silently ignore an era.

### III. Type Safety as Security
Handling cryptographic keys and financial transactions demands defense in depth through Rust's type system. Use newtype wrappers for all domain identifiers (wallet IDs, addresses, key hashes). Prefer type-level guarantees and `#[must_use]` over runtime checks. Avoid `unwrap()` and `expect()` in library code — propagate errors via `thiserror`/`anyhow`. Use `zeroize` for secrets in memory.

### IV. REST API Contract
The wallet exposes a REST API compatible with the existing cardano-wallet OpenAPI spec (`specifications/api/swagger.yaml` in the reference implementation). API surface parity with the Haskell backend is a hard requirement for Daedalus compatibility. Breaking changes require explicit versioning.

### V. Reproducible Builds
Nix flakes (via `flake.nix`) are the single source of truth for the build environment. Crane builds the Rust workspace. Fenix provides the pinned Rust toolchain. All `nix` commands use the project flake. The devshell (`nix develop`) is the authoritative development environment.

### VI. Storage via Sled
Persistent state uses sled (an embedded key-value store) exclusively. No SQL dependencies. Data is organized into named sled Trees (one per logical collection). Composite keys encode ordering where range scans replace SQL `ORDER BY`. Secondary index trees maintain consistency within a single sled batch/transaction.

### VII. Comprehensive Testing
- **Unit tests** (`#[test]`) cover all domain logic in isolation
- **Property tests** (`proptest` or `quickcheck`) cover cryptographic and serialization roundtrips
- **Integration tests** run against a local cardano-node via the node socket protocol (pallas-network)
- **API tests** validate REST endpoint behavior against the OpenAPI spec
- **Roundtrip tests** for all CBOR/JSON serialization

Tests run locally before pushing. CI is not a substitute for local verification.

### VIII. Code Quality Gates
All code must pass before merge:
1. **rustfmt** formatting (project-standard config)
2. **clippy** with `--deny warnings` (all targets)
3. **Compilation** with no warnings
4. **Unit and property tests** green
5. **Integration tests** green against target era
6. CI must be green — never merge with failing checks

## Architecture

### Layered Workspace Design
The Cargo workspace follows strict layering — dependencies flow downward only:

```
quetzal-api          ← REST API server (axum), Daedalus-compatible endpoints
quetzal-wallet       ← Business logic: balance, coin selection, tx construction
quetzal-address      ← Address derivation (Byron/Icarus/Shelley) and discovery
quetzal-network      ← Node communication (pallas-network miniprotocols)
quetzal-crypto       ← Cryptographic primitives (key derivation, signing)
quetzal-primitives   ← Core domain types: Wallet, Tx, UTxO, Address, Coin
```

Each crate has its own test suite. Cross-crate circular dependencies are forbidden.

### Pallas as Cardano Primitives
`pallas` (version 0.32) is the canonical source for all Cardano on-chain types, CBOR encoding/decoding, transaction traversal, and node miniprotocols. Do not reimplement what pallas provides.

### Sled Storage Layout
Each logical collection is a named sled Tree:
- `wallets` — wallet metadata keyed by wallet ID
- `utxos:{wallet_id}` — UTxO set keyed by `TxHash ++ OutputIndex`
- `txs:{wallet_id}` — tx history keyed by `SlotNo ++ TxHash` (slot-ordered range scans)
- `addresses:{wallet_id}` — discovered addresses keyed by address bytes
- `checkpoints:{wallet_id}` — chain sync checkpoints keyed by slot

Secondary index trees use the prefix `idx:` convention.

### REST API
The axum server implements the cardano-wallet REST API surface. The OpenAPI spec is the contract. Handlers delegate to `quetzal-wallet` — no business logic in the API layer. Error responses follow the cardano-wallet error format for Daedalus compatibility.

## Development Workflow

### Commit Discipline
Conventional Commits format. Each commit addresses a single concern. `feat:` for new capabilities, `fix:` for correctness, `chore:` for tooling.

### Dependency Management
All version constraints live in `[workspace.dependencies]` in the root `Cargo.toml`. Individual crates use `dep.workspace = true`. Do not pin versions in crate-level `Cargo.toml` files.

## Governance

This constitution defines the quality and design principles for all spec-driven work on Quetzal. All specs, plans, and implementations must demonstrate compliance with these principles.

**Version**: 1.0.0 | **Ratified**: 2026-08-22
