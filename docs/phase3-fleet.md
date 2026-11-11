# Phase 3 fleet and edge software: what is built and what is not

Written for: engineers wiring the fleet services to MQTT, the edge node, and Keystone, and for
the maintainer deciding what Phase 3 gates on.

## Built and tested

### Shared wire layer (`fleet/wire`)

- The envelope and every message shape in `docs/mqtt-topics.md`, with the validation rules the
  document states. Decoding tolerates unknown fields. Topic builders reject wildcards.

### Edge MQTT (`edge/crates/tpt-agripod-edge`: `link.rs`, `node.rs`)

- **Node logic** (`node::Node`) returns the messages to publish and never touches the network:
  telemetry for good readings, status on each heartbeat and on safe state, acks for profile pushes
  and commands.
- **Transport** (`node::run`) connects with `minimq` over tokio, subscribes to `profile/push` and
  `command`, publishes what `Node` returns, and reconnects after failures. The last-will is a
  retained `offline` status.
- **Profile pushes** are validated by the edge (the orchestrator's check is not trusted). A repeated
  push gets the same ack back and is not applied twice. Unsafe profiles are rejected with reasons,
  and the previous profile stays active.
- **Commands** are acked as rejected, because command execution is not implemented.
- **Offline operation** (ADR 0015): the control loop runs on its own schedule, whether or not the  broker is reachable. Telemetry is held in a durable outbox and drained in order on reconnect. An  outage drill (`tests/outage_drill.rs`) kills and restarts the broker process and checks that the  outage telemetry is delivered with no gap.
- **Tested against a real broker** (`tests/node_link.rs`, five cases): profile applied and acked,
  repeat push answered identically, unsafe profile rejected with reasons, telemetry each cycle with
  increasing `seq`, safe state on sensor loss, and last-will after the task is aborted. The broker
  is `fleet/testbroker` (mochi, MIT). Run with `AGRIPOD_TEST_BROKER`. CI runs them in the
  `edge-broker` job.

### Telemetry (`fleet/telemetry`)

- **Point validation**: UTC timestamps (local time is refused), node and metric names, finite values.
- **Batcher**: size and interval flushes. The buffer is bounded and `Submit` blocks when it is full
  (back-pressure). Transient sink errors are retried with backoff. A batch that still fails stops
  the batcher, and the error is returned from `Close`. Nothing is dropped silently.
- **Ingester**: decodes telemetry, drops duplicate `msg_id`s (bounded memory), counts sequence gaps
  and reorders per node, and skips samples whose quality is not `good`. Device samples carry the
  device in the node key, so two devices reporting the same channel cannot collide on the key.
- **MQTT source**: subscribes to `agripod/+/+/telemetry` at QoS 1. Tested end to end over a broker.
- **Keystone sink**: the vendored Keystone SDK (`fleet/third_party/keystone-sdk-go`, pinned to commit
  `53e2017`) is wired in through `KeystoneSession`, which reconnects after a broken connection.
  Writes are batched INSERTs in transactions, idempotent on resend (ADR 0009). Schema creation is
  `telemetry -init-schema -keystone HOST:PORT`, with retention 90 days (ADR 0012).
- **Commands**: `-in FILE` (or stdin) for offline ingest; `-broker HOST:PORT` for live ingest until
  interrupted, which flushes before exiting; `-keystone HOST:PORT` to write to Keystone instead of
  a file.

### Orchestrator (`fleet/orchestrator`)

- **Grow-cycle state machine** (`cycle`): stage time is active time, so holds pause the clock.
  Overrides need a reason. A backwards clock is refused. Every transition is journalled. State is
  saved atomically.
- **Profile loader** (`profile`): unknown fields are rejected. The schema's rules are checked, and
  the pump duty limit is checked against the edge contract (a test keeps them equal).
- **Profile push** (`fleet/push.go`): validates locally and refuses to send an invalid profile.
  Waits for the ack with the same `msg_id`, retries on timeout, and records each node's applied
  profile. Tested over a broker with a fake edge.
- **Node registry** (`fleet/registry.go`): tracks `status`, including retained last-will messages.
- **Stage templates** must be completed: the example is refused (ADR 0014).

### Site configuration and the edge contracts (ADR 0014)

- `SiteConfig` (edge) holds the installation values: valve open-time limit, LED headroom, irrigation
  thresholds and timing, drain limit. It has no defaults. Every field must be present and valid, and
  `"status": "production"` is required. The example file is refused.
- The solenoid and LED contracts are templates rendered from the site file.
- The simulator's tariff is a required input. The CLI refuses to simulate without it.

## Not built, and why

| Item | Status | Blocker |
|---|---|---|
| Edge TLS and auth | Not started | Needs a `no_std` TLS crate audited against the policy |
| Hardware-side outage behaviour (power loss, sensor loss, actuator loss) | Not tested | The broker outage drill passes (`tests/outage_drill.rs`, ADR 0015). Power-loss and hardware drills need the hardware |
| The `agripod-edged` daemon | Not built | Needs the sensor and actuator drivers (device register maps from datasheets). Without them a daemon would only ever report safe state |
| LED dimming loop | Not started | No LED actuator exists to drive |
| Profile targets feeding controllers | Not started | No controller consumes VPD or DLI targets yet |
| Full JSON Schema validation | Not done | Both sides check the schema's rules by hand. Shared vectors (`profiles/testvectors`, 18 cases) keep them in step; the vectors cover the rules, not every JSON Schema keyword |
| Live Keystone test | Written, not run | `keystone_live_test.go` needs a running Keystone (`KEYSTONE_ADDR`). This environment has none |
| Keystone `COPY` bulk path | Not built | The SDK has no COPY sub-protocol (ADR 0009) |
| Reservoir thermal model, predictive scheduling, nutrient mixture model | Not started | Phase 3 physics items |
| Lettuce and potato profiles | Not written | Agronomic inputs, not invented here |

## Values the operator must supply

Nothing runs on a made-up value any more. These must be supplied before a node or a simulation
will start:

- **Site file** (`edge/config/site.example.json`, copied and completed): valve open-time limit (flood
  study), LED headroom (supply rating), irrigation thresholds, pulse, cooldown, cycle time and pump
  duty (calibration), drain limit, broker address, site and node ids.
- **Electricity tariff** for the simulator: `--tariff-peak`, `--tariff-offpeak`, `--offpeak-start`,
  `--offpeak-end`.
- **Stage durations** for a grow cycle (`fleet/orchestrator/templates/example-120d.json` is the
  example, refused as is).

Assumptions that remain in code are engineering settings, not site values: the MQTT keep-alive
(30 s), the control cycle (10 s), the heartbeat (30 s), the reconnect delay (2 s), telemetry batch
and buffer sizes, the ack timeout and attempt counts, and the backoff. Each is documented where it
is set.
