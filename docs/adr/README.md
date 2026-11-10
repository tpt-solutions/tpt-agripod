# Architecture decision records

Each ADR records one decision: the context, the choice, and its consequences. Files are numbered in order and never renumbered. A superseded decision gets a new ADR that links back to it.

Status values: `Proposed`, `Accepted`, `Superseded by NNNN`.

## Index

| ADR | Title | Status |
|---|---|---|
| [0001](0001-monorepo.md) | Single monorepo with per-language workspaces | Accepted |
| [0002](0002-mqtt-json-edge-fleet.md) | MQTT with JSON payloads between edge and fleet | Accepted |
| [0003](0003-own-qf-lra-solver.md) | Own QF_LRA solver, not an external SMT library | Accepted |
| [0004](0004-esp32-leaf-tier.md) | ESP32 as a leaf tier behind the Rust edge node | Accepted |
| [0005](0005-keystone-telemetry-sink.md) | TPT Keystone as the telemetry sink | Accepted |

## Template

```markdown
# NNNN: Title

Status: Proposed
Date: YYYY-MM-DD

## Context

What forces are at play? What must be true?

## Decision

What we will do.

## Consequences

What becomes easier, harder, or riskier. Include follow-up work.
```
