# 0003: Own QF_LRA solver, not an external SMT library

Status: Accepted
Date: 2026-11-11

## Context

Actuator commands are checked against linear contracts before they run (for example `pump_duty_cycle <= 0.15`). The check must be bounded in time, run without `std`, and be auditable. Most SMT libraries are large, C-backed, or under licences the policy excludes. Big-number crates in the GPL family are also excluded.

## Decision

Build a small QF_LRA decision procedure in `tpt-agripod-solver`:

- exact rational arithmetic, with no GPL big-number dependency
- simplex-based decision procedure
- a contract DSL with `requires:` and `ensures:` over linear constraints and boolean connectives
- differential tests against a reference solver that runs offline and is not shipped

## Consequences

- We own the correctness burden. Differential testing and property tests are required before the solver is used on hardware.
- The worst-case runtime must be measured and bounded, and a `no_std` feasibility check is required.
- Scope stays small. Any extension needs its own ADR.
