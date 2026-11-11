# 0010: Fleet MQTT client is yosssi/gmq; gmqtt is broker-only (corrects ADR 0007)

Status: Accepted. Supersedes the client half of ADR 0007. The broker half (`mochi-mqtt/server`)
stands.
Date: 2026-11-11

## Context

ADR 0007 chose `github.com/DrmagicE/gmqtt` as the fleet client. Checking the module for the
first MQTT code in Phase 3 showed that gmqtt v0.5.0 (the latest tag) is a broker library. It has
no client package. The choice cannot be used as written.

The fleet needs a Go MQTT client that passes the licence policy. Candidates checked against the
GitHub API and the Go module cache:

| Repository | Role | Licence | Last activity |
|---|---|---|---|
| `DrmagicE/gmqtt` v0.5.0 | Broker only | MIT | Tag from 2022 |
| `yosssi/gmq` v0.0.1 | Client, MQTT 3.1.1 | MIT | 2018 |
| `at-wat/mqtt-go` | Client | Apache-2.0 only | 2026 |
| `mdzio/go-mqtt` | Broker and client | Apache-2.0 only | 2026 |
| `ashtonian/mqttv5` | Client, MQTT 5 | Apache-2.0 only | 2026 |
| `eclipse/paho.mqtt.golang` | Client | EPL-2.0 / EDL-1.0 | Excluded by policy |

Only `yosssi/gmq` is both a client and permissively licensed. It is old. Its `Publish` queues a
packet and returns, without waiting for PUBACK.

## Decision

- **Client:** `github.com/yosssi/gmq` (MIT). It is the only MQTT client dependency in the fleet
  modules, and it is imported only in `fleet/telemetry/telemetry/mqtt.go` and
  `fleet/orchestrator/fleet/mqtt.go`. Replacing it touches those files.
- **Broker:** `github.com/mochi-mqtt/server/v2` (MIT, active). It is used in tests, as an
  in-process broker. Production broker choice is still open, and the same MIT criterion applies.
- The application protocol does its own reliability. Profile pushes and their acks carry
  `msg_id` and `ack_of`, and the sender retries with the same `msg_id`. Telemetry is
  deduplicated on `msg_id`. So a fire-and-forget `Publish` is acceptable.

## Consequences

- The client is old. Two mitigations: it is behind a thin file, and every behaviour the fleet
  depends on (subscribe, QoS 1 publish, last-will, retained status) is exercised by tests against
  a real broker. If it breaks, the tests say so.
- The orchestrator and telemetry depend on `gmq`'s keep-alive behaviour. `Terminate` stops the
  client's goroutines without closing the socket, so the broker publishes a last-will only after
  keep-alive expires. The tests use a short keep-alive for this reason, and the edge must do the
  same if it relies on last-will.
- If `gmq` needs replacing, the candidates are a small in-house MQTT 3.1.1 client, or an
  Apache-2.0 client plus a documented licence exception. Either needs an ADR.
