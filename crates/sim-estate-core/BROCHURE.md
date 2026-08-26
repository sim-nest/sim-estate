# sim-estate-core

In one line: Pure portable estate records and provider contract for SIM.

## What it gives you

Represent plans, approvals, runs, verification, reconciliation, leases, quarantine, and outcomes with one deterministic vocabulary. Provider implementations share the same records and typed failure boundary, so a modeled site and a physical site can be compared without translating between private control formats. Content identities bind evidence to the exact operation it describes. Bounded fields and closed states keep malformed or ambiguous input from becoming authority. The crate carries portable facts only: host access, command execution, and product policy remain with higher layers.

## Why you will be glad

- Deterministic records make provider behavior easy to compare and test.
- Typed states keep uncertainty and quarantine visible instead of smoothing them into success.
- A shared contract prevents each provider from inventing a competing audit vocabulary.

## Where it fits

This is the portable substrate below estate books, organs, command adapters, models, and public projections. It owns records and provider interfaces, not storage, process authority, host discovery, or user decisions.
