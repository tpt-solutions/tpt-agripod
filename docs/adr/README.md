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
| [0006](0006-edge-mqtt-client-minimq.md) | Edge MQTT client is minimq, with std-embedded-nal for Linux TCP | Accepted |
| [0007](0007-fleet-mqtt-client-and-broker.md) | Fleet MQTT client and broker (client half superseded by 0010) | Accepted |
| [0008](0008-fourier-motzkin-first-decision-procedure.md) | Fourier–Motzkin as the first decision procedure; simplex to follow | Accepted |
| [0009](0009-keystone-write-path.md) | Keystone write path: batched INSERT now, COPY after the SDK gains it | Accepted |
| [0010](0010-fleet-mqtt-client-correction.md) | Fleet MQTT client is yosssi/gmq; gmqtt is broker-only (corrects ADR 0007) | Accepted |
| [0011](0011-edge-json-parser.md) | Edge profile parsing uses an in-house strict JSON parser | Accepted |
| [0012](0012-raw-retention-90-days.md) | Raw sensor retention is 90 days | Accepted |
| [0013](0013-edge-mqtt-over-tokio.md) | Edge MQTT runs minimq over tokio, with embassy-time's generic timer queue | Accepted |
| [0014](0014-site-config-no-defaults.md) | Site-specific values come from a site file, with no defaults | Accepted |

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
