# 0007: Fleet MQTT client is gmqtt; fleet broker is mochi-mqtt

Status: Accepted
Date: 2026-11-11

## Context

The Go fleet services (`tpt-agripod-orchestrator`, `tpt-agripod-telemetry`) need an MQTT client. A broker is needed for development and for the fleet side. Mosquitto is the common choice, but it is EPL-2.0 / EDL-1.0, which the policy excludes. `eclipse/paho.mqtt.golang` is EPL-2.0 / EDL-1.0 as well.

Candidates checked with the GitHub API (licence as detected by GitHub, last push):

| Repository | Use | Licence | Last push |
|---|---|---|---|
| `DrmagicE/gmqtt` | Client and broker library, MQTT 3.x and 5 | MIT | 2026-04 |
| `yosssi/gmq` | Client | MIT | 2018 (stale) |
| `mochi-mqtt/server` | Embeddable MQTT 5 broker | MIT | 2026-09 |
| `eclipse/paho.mqtt.golang` | Client | EPL-2.0 / EDL-1.0 | Excluded by policy |

## Decision

- Client: `github.com/DrmagicE/gmqtt`, for the orchestrator and telemetry services.
- Broker: `github.com/mochi-mqtt/server`, run as the fleet broker for development and production.

## Consequences

- Both are MIT, so the Go licence check passes without needing EDL or EPL exceptions. Go's `--allowed_licenses` check still needs a real test once a module is added (open item in `todo.md`).
- `gmqtt` is a library with both client and broker code. Only the client packages are imported in fleet services, so the broker code is not shipped.
- `yosssi/gmq` was rejected because it has had no commits since 2018.
- Follow-up: pin both by version when they are first added to `go.mod`.
