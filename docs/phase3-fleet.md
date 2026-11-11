# Phase 3 fleet and edge software: what is built and what is not

Written for: engineers wiring the fleet services to MQTT, the edge node, and Keystone, and for
the maintainer deciding what Phase 3 gates on.

## Built and tested

### Shared wire layer (`fleet/wire`)

- The envelope and every message shape in `docs/mqtt-topics.md`, with the same validation rules
  the document states. Decoding tolerates unknown fields, so a newer edge can add fields.
- Topic builders and a parser that rejects wildcards.
- The documented examples decode. Each rule has a case that fails.

### Telemetry (`fleet/telemetry`)

- **Point validation.** UTC timestamps (local time is refused), node and metric names, finite values.
- **Batcher.** Size and interval flushes. The buffer is bounded and `Submit` blocks when it is
  full (back-pressure). Transient sink errors are retried with backoff. A batch that still fails
  stops the batcher, and the error is returned from `Close`. Nothing is dropped silently.
- **Ingester.** Decodes telemetry messages, drops duplicate `msg_id`s (bounded memory), counts
  sequence gaps and reorders per node, and skips samples whose quality is not `good`. Device
  samples carry the device in the node key, so two devices reporting the same channel cannot
  collide on the primary key.
- **MQTT source.** Subscribes to `agripod/+/+/telemetry` at QoS 1. Tested end to end over a broker:
  a duplicate delivery and a stale sample are both handled correctly.
- **Sinks.** A file sink (JSON Lines, fsynced per batch) and a Keystone sink (batched INSERT in
  transactions, idempotent on resend, ADR 0009). The Keystone sink is tested against a fake that
  enforces the primary key, not against a live server.
- **Command.** `-in FILE` (or stdin) for offline ingest. `-broker HOST:PORT` for live ingest
  until interrupted, which flushes before exiting.

### Orchestrator (`fleet/orchestrator`)

- **Grow-cycle state machine** (`cycle`). Stage time is active time, so holds pause the clock.
  Carry-over between stages is kept. Overrides need a reason. A backwards clock is refused. Every
  transition is journalled. State is saved atomically.
- **Profile loader** (`profile`). Unknown fields are rejected, as the schema requires. Checks the
  schema's rules, and the pump duty limit against the edge contract. A test keeps the Go limit
  equal to `edge/contracts/pump.contract`.
- **Profile push** (`fleet/push.go`). Validates locally and refuses to send an invalid profile. Sends
  `profile/push`, waits for the ack with the same `msg_id`, and retries on timeout. A rejection
  always surfaces as a rejection. Each node's applied profile is recorded. Tested over a broker with
  a fake edge: a valid push is applied, an invalid one is rejected and not recorded, a lost push is
  retried with the same `msg_id`, and a silent node fails after its attempts.
- **Node registry** (`fleet/registry.go`). Tracks `status`, including retained last-will
  messages. Tested over a broker: a node that crashes (no DISCONNECT, keep-alive expires) is marked
  offline by its will.
- **Command.** `validate`, `push`, `init`, `tick`, `hold`, `resume`, `override`.

### Edge (`edge/crates/tpt-agripod-edge`)

- **Profile validation** (`profile.rs`). Applies the schema's rules, rejects unknown fields and
  control names, and reads the pump duty limit from the shipped contract, checked by the solver.
  The apply path is atomic: a refused profile keeps the previous one active.
- **JSON parser** (`json.rs`). Strict RFC 8259, written in-house (ADR 0011). Tested with malformed
  input and a seeded fuzz run.
- **Runtime apply.** The pump duty setpoint from an accepted profile is what irrigation uses. The
  integration test shows it reaching the hardware, and a refused profile leaving the setpoint
  unchanged.

## Not built, and why

| Item | Status | Blocker |
|---|---|---|
| MQTT on the edge | Not started | `minimq` chosen (ADR 0006), not added. Needs `std-embedded-nal` for Linux TCP |
| Edge TLS and auth | Not started | Phase 3 security item. Needs a `no_std` TLS crate audited against the policy |
| Telemetry buffer on the edge (offline resync) | Not started | Depends on the MQTT work above |
| LED dimming loop | Not started | The value is stored; no LED actuator exists to drive |
| Profile targets feeding controllers | Not started | No controller consumes VPD or DLI targets yet |
| Full JSON Schema validation | Not done | Both sides check the schema's rules by hand. A shared test-vector file would keep them in step |
| Keystone SDK adapter and COPY | Not built | SDK module path, no COPY, no SQLSTATE (ADR 0009) |
| Live Keystone and live edge tests | Not run | No Keystone or edge hardware here |
| Retention and rollup | Retention is a placeholder; no rollup | Retention policy decision |
| Lettuce and potato profiles | Not written | Agronomic inputs, not invented here |
| Reservoir thermal model, predictive scheduling, nutrient mixture model | Not started | Phase 3 physics items |

## Placeholders

- The cycle template in `fleet/orchestrator/templates/example-120d.json` is a placeholder.
- The retention in `SchemaStatements` is a placeholder. Batch and buffer sizes, the ack timeout,
  attempt counts and backoff are assumptions.
- The MQTT keep-alive values are assumptions. Last-will detection takes about 1.5 keep-alive
  periods.
