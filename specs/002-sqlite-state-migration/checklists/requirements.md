# Requirements quality

- [x] SQLite source and sled destination are explicit; seed restoration is not substituted.
- [x] Original source preservation, WAL handling and no-replace atomic publication are required.
- [x] Credentials, namespaces, discovery and local pending/provenance are in scope.
- [x] Unknown schemas and unsupported semantic state fail explicitly.
- [x] Reference-created SQLite and genuine native verification are required.
- [x] Complete transfer for immediate startup confirmed by the user; rescan only for deep rollback recovery.
- [ ] Exact supported schema/family/version matrix qualified.
