# sim-site-estate-command

In one line: Sealed ProcessPort adapter for compiled SIM estate operations.

## What it gives you

Run declared estate operations through the canonical bounded process port without granting callers shell, path, program, or environment authority. The adapter accepts only compiled operation bindings and maps them to fixed program references, project roots, argument atoms, budgets, cancellation, and private artifact references. Process receipts retain bounded output and exact dispatch evidence, including the unknown-after-dispatch case. This keeps command execution reusable while preserving the estate organ's stronger review and reconciliation rules.

## Why you will be glad

- Product callers cannot smuggle arbitrary command syntax through the adapter.
- Shared process budgets and receipts make execution behavior consistent with other SIM sites.
- Dispatch uncertainty remains explicit for later reconciliation.

## Where it fits

This crate bridges compiled estate operations to `sim-lib-exec`. It owns the translation boundary only. Provider semantics, approval, durable history, process implementation, and user-facing projections remain in their dedicated components.
