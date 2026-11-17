# Local node recovery correction

Observed on unchanged master719daea with current independent suite30a6aad...:
network.local-node-recovery / sync:disconnect fails with reference sync progress
differs. Four other acceptance profiles and transport pass.

Pinned source2fc5c383: NetworkInformation.getNetworkInformation reads currentNodeTip
from cached network state; Network.Implementation._syncProgress uses its retained
interpreter and current wall time. NotResponding applies when no interpreter has
been observed, rather than every failed new connection. Quetzal's handler queries
a fresh snapshot on each request and forces not_responding after failure, while
falling back to historical replay state. That loses independently observed state.

Plan: retain an in-memory last successful native NetworkSnapshot, shared across
AppState clones and Byron namespace. Populate during startup native query and
successful network-information requests. On query failure retain that exact
native tip/era and compute progress from its native clock and current time. Never
promote historical replay or invent native observations on cold start. New
successful observations replace cached state, including rollbacks/reconnection.
No persisted format changes or conformance source/expectation changes.

Validation tasks:
- [x] Focused regression: cold failed node never inherits replay readiness.
- [x] Focused regression: observed native state survives disconnect and replaces
      conflicting replay state; progress recomputes ready/syncing with time.
- [x] Focused regression: subsequent native observation replaces prior point/era.
- [x] Formatting, targeted API tests and strict workspace Clippy.
- [x] Fresh package and independent local recovery/full cohort against fixed
      candidate. Keep original run and failures intact; no cohort stitching.
