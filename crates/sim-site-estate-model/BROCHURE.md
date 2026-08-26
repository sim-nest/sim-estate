# sim-site-estate-model

In one line: Deterministic host-free model site for SIM estate providers.

## What it gives you

Script stale plans, disappearing targets, unsupported previews, ambiguous dispatch, malformed events, cancellation, verification failure, controller loss, and reconciliation without host variability. The model implements the same provider contract as a physical site, so the guarded organ can be tested against exact state transitions and failure evidence. Deterministic scenarios make difficult races repeatable and let conformance tests prove that uncertainty enters quarantine instead of being silently accepted.

## Why you will be glad

- Rare provider failures become cheap, repeatable test cases.
- The same contract checks modeled and physical behavior.
- Deterministic evidence makes regressions easy to reproduce and review.

## Where it fits

This crate is the host-free provider implementation used by tests, examples, and design validation. It owns scenario control and modeled responses. Physical execution, private artifacts, durable storage, approval policy, and user interfaces remain outside it.
