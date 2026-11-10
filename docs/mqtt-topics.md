# MQTT topic map and message schemas (v0)

Written for: engineers building the edge daemon, the orchestrator and the telemetry service. Status: draft, Phase 0 contract.

Transport: MQTT 3.1.1 or 5.0. Payload: UTF-8 JSON. Timestamps: RFC 3339, UTC.

## Topic prefix

```
agripod/{site_id}/{node_id}/...
```

- `site_id`: a physical installation, lowercase slug, e.g. `wgtn-bay-01`.
- `node_id`: an edge node, lowercase slug, e.g. `nuc-01`.
- Leaf devices (ESP32) appear as `device_id` inside payloads, not as topics.

## Topics

| Topic | Direction | QoS | Retained | Purpose |
|---|---|---|---|---|
| `agripod/{site}/{node}/profile/push` | orchestrator to edge | 1 | no | Push a Crop Profile |
| `agripod/{site}/{node}/profile/ack` | edge to orchestrator | 1 | no | Acknowledge or reject a profile |
| `agripod/{site}/{node}/telemetry` | edge to telemetry | 1 | no | Batched sensor samples |
| `agripod/{site}/{node}/command` | orchestrator to edge | 1 | no | Operator or state-machine command |
| `agripod/{site}/{node}/command/ack` | edge to orchestrator | 1 | no | Command result |
| `agripod/{site}/{node}/status` | edge to fleet | 1 | yes | Heartbeat and online state |

The last-will message is published on `status` with `retain = true`, so the fleet sees the node go offline after an unclean disconnect.

## Common envelope

Every payload carries:

```json
{
  "msg_id": "0190f3a2-7c1e-7d4b-9a3e-2f5c8e1b6d10",
  "ts": "2026-11-11T02:15:00Z",
  "site_id": "wgtn-bay-01",
  "node_id": "nuc-01"
}
```

`msg_id` is a UUID. The receiver uses it to deduplicate after a reconnect, so retried messages are idempotent.

## Message shapes

### `profile/push`

```json
{
  "msg_id": "...",
  "ts": "...",
  "site_id": "...",
  "node_id": "...",
  "profile": { "...": "Crop Profile v0, see profiles/crop-profile.schema.json" }
}
```

The edge validates the profile before applying it. An unsafe or malformed profile is rejected, and the previous profile stays active.

### `profile/ack`

```json
{
  "msg_id": "...",
  "ts": "...",
  "ack_of": "<msg_id of profile/push>",
  "profile_id": "lettuce-v1",
  "profile_version": "1.0.0",
  "status": "applied | rejected",
  "errors": [ { "path": "/targets/vpd_kpa", "message": "..." } ]
}
```

### `telemetry`

```json
{
  "msg_id": "...",
  "ts": "...",
  "site_id": "...",
  "node_id": "...",
  "seq": 10452,
  "samples": [
    { "channel": "air_temp", "ts": "...", "value": 21.4, "unit": "degC", "quality": "good" },
    { "channel": "rh", "ts": "...", "value": 0.68, "unit": "frac", "quality": "good" },
    { "channel": "soil_moisture", "device_id": "esp32-z1", "ts": "...", "value": 0.31, "unit": "frac", "quality": "stale" }
  ]
}
```

`quality` is one of `good`, `stale`, `out_of_range`, `missing`. `seq` is monotonic per node and lets the telemetry service detect gaps. Batches hold up to 500 samples.

### `command`

```json
{
  "msg_id": "...",
  "ts": "...",
  "ttl_s": 30,
  "actuator_id": "pump-main",
  "action": "set_duty",
  "value": 0.10
}
```

Commands past `ttl_s` are dropped, not executed late. Every command is checked against the contracts before it runs.

### `command/ack`

```json
{
  "msg_id": "...",
  "ts": "...",
  "ack_of": "<msg_id of command>",
  "status": "executed | rejected | expired",
  "reason": "contract violated: pump_duty_cycle <= 0.15"
}
```

### `status`

```json
{
  "msg_id": "...",
  "ts": "...",
  "node_id": "nuc-01",
  "state": "online | safe_state | offline",
  "profile_id": "lettuce-v1",
  "profile_version": "1.0.0",
  "uptime_s": 86400
}
```

`safe_state` means the edge has entered its fail-safe state, with valves closed and pumps off.

## Security

- Phase 3: TLS and username/password per node.
- Phase 5: mutual TLS and role separation (see `todo.md`).
- Safety logic never reads from MQTT. An unreachable broker must not change safety behaviour.
