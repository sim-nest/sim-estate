# sim-estate

SIM's host-free estate language, exposure compiler, provider contract, and deterministic model site.

The public API deliberately cannot carry commands, paths, environment bindings, inventory patterns, or provider-private output. Provider adapters translate sealed exposure declarations at a later platform boundary; this repository contains no process or host integration.

## Crates

- `sim-estate-core`: portable records, provider contract, and conformance suite.
- `sim-estate-project`: strict exposure declarations and compiler.
- `sim-lib-estate-book`: immutable content-addressed operation history.
- `sim-lib-estate`: guarded plan/apply/reconcile organ.
- `sim-lib-estate-serve`: one read-only directory and typed, capability-shaped calls shared by every product surface.
- `sim-site-estate-model`: deterministic scripted provider and fixture project.
- `sim-site-estate-command`: pure compilation into sealed process requests.
- `sim-site-estate-ansible`: fixed Ansible discovery and hash-chained callback evidence.

Run `cargo run -p xtask -- check` for the full repository contract.
