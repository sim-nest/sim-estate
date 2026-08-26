# sim-estate-project

In one line: Pure closed exposure compiler for SIM estate operations.

## What it gives you

Compile a reviewed operation description into a closed, typed exposure without handing callers a shell-shaped escape hatch. Strict schemas and deterministic encoders bind provider, target, risk, input, and output identities while rejecting unknown fields and unsafe combinations. The result can be inspected, hashed, stored, and compared before any effect-capable site sees it. Extensible identities preserve room for new providers and risk classes while the executable shape stays deliberately narrow.

## Why you will be glad

- Callers express intent through typed data instead of command strings.
- Deterministic compilation makes review evidence stable and recomputable.
- Rejection happens before dispatch, keeping malformed authority away from hosts.

## Where it fits

This crate sits between authored estate configuration and the guarded estate organ. It owns pure validation and compilation. Provider discovery, approval, durable history, execution, and reconciliation remain in their dedicated libraries and sites.
