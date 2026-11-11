# Phase 3 fleet layer: what is built and what is not

Written for: engineers wiring the fleet services to MQTT, the edge node, and Keystone, and for
the maintainer deciding what Phase 3 gates on.

## Built and tested (host only)

### Telemetry (`fleet/telemetry`)

- **Point validation.** Node and metric names, UTC timestamps (local time is refused, per the
  units doc), and finite values.
- **Batcher.** Batches by size and by interval. The buffer is bounded and `Submit` blocks when it
  is full, which is the back-pressure. Transient sink errors are retried with backoff. A batch that
  still fails after the attempts stops the batcher, and the failure is returned from `Close`.
  Nothing is dropped silently.
- **File sink.** JSON Lines, fsynced per batch.
- **Keystone sink.** Batched `INSERT` in transactions with idempotent resends (ADR 0009). It is
  tested against an in-memory fake that enforces the primary key. It is not tested against a live
  server.
- **Command.** `tpt-agripod-telemetry -in points.jsonl -out points.jsonl` reads JSON Lines,
  rejects bad lines with their line numbers, and writes through the batcher.

### Orchestrator (`fleet/orchestrator`)

- **Grow cycle state machine** (`cycle`). Stage time is active time: holds pause the clock.
  Carry-over between stages is kept. Overrides need a reason. A backwards clock is refused. Every
  transition is journalled.
- **Atomic persistence.** Write to a temp file, fsync, rename, fsync the directory.
- **Profile loader** (`profile`). Unknown fields are rejected, as the schema requires. It checks
  the required fields, enums, patterns, numeric ranges, control names, and the pump duty limit
  against the edge contract. A test keeps that limit equal to `edge/contracts/pump.contract`.
- **Command.** `validate`, `init`, `tick`, `hold`, `resume`, `override`. Each takes `-now` for
  reproducible runs.

## Not built, and why

| Item | Status | Blocker |
|---|---|---|
| MQTT client in orchestrator and telemetry | Not started | Adding `gmqtt` to `go.mod` (ADR 0007). Needs network to the Go proxy and a version pin |
| Profile push to edge, with ack, retry and version tracking | Not started | MQTT |
| Node registry, health, last-will | Not started | MQTT |
| Full JSON Schema validation | Not done | The loader checks the schema's rules by hand. A full validator needs a dependency that is MIT or Apache-2.0 dual |
| Keystone SDK adapter | Not built | Module path (ADR 0009) |
| `COPY` bulk path | Not built | Needs COPY in the Keystone Go SDK (upstream) |
| Live Keystone test | Not run | No Keystone instance here |
| Retention and rollup | Retention is a placeholder in the DDL; rollup not written | Retention policy decision |
| Lettuce and potato profiles | Not written | Agronomic inputs. Not invented here |
| Stage durations in profiles | Not in the schema | The template file is a placeholder (`templates/example-120d.json`) |

## Placeholders

- The cycle template in `fleet/orchestrator/templates/example-120d.json` is a placeholder.
- The retention in `SchemaStatements` is a placeholder. The `DefaultConfig` batch and buffer sizes
  are assumptions. `MaxAttempts` and the backoff are assumptions.
