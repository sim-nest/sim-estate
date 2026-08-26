# sim-lib-estate-book

In one line: CAS-backed durable evidence for SIM estate operations.

## What it gives you

Audit an estate operation from its content-bound plan through approval, dispatch, verification, cleanup, reconciliation, and final outcome. Compare-and-swap storage makes approval consumption and state transitions atomic across competing processes. Events remain append-only, while readable projections are rebuilt from canonical records instead of trusted as mutable summaries. The Table boundary keeps storage portable across memory, files, and other durable implementations without moving policy into the storage adapter.

## Why you will be glad

- One approval can be consumed exactly once even under process contention.
- Rebuilt projections expose corruption or drift instead of concealing it.
- Stable event identities make tests and operational inspection deterministic.

## Where it fits

The book is the durable evidence layer beneath the guarded organ and read-only estate projection. It owns atomic records and reconstruction, not provider execution, private artifacts, UI policy, or the human approval decision.
