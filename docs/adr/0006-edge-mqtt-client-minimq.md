# 0006: Edge MQTT client is minimq, with std-embedded-nal for Linux TCP

Status: Accepted
Date: 2026-11-11

## Context

The edge node speaks MQTT to the fleet (ADR 0002). The licence policy in `todo.md` allows only MIT, BSD-2/3, ISC, Zlib, Unlicense and MIT/Apache-2.0 dual licences. Candidates checked against crates.io metadata:

| Crate | Licence | Result |
|---|---|---|
| `rumqttc` | Apache-2.0 only | Denied |
| `paho-mqtt` | EPL-2.0 | Denied |
| `mqttrs` | Apache-2.0 only | Denied |
| `rumqttd` | Apache-2.0 only | Denied |
| `minimq` 0.13.5 | MIT | Allowed. MQTT 5 client, `no_std` |
| `mqtt-protocol` | MIT/Apache-2.0 | Allowed. Codec only, so we would write the client |

## Decision

Use `minimq` 0.13.5 for the edge client. It is `no_std`, which matches the ADR 0004 direction for the leaf tier and keeps the protocol code portable. On the Linux edge node, TCP comes from `std-embedded-nal` 0.4 (MIT OR Apache-2.0), which implements the `embedded-nal` traits over `std::net`.

Protocol: MQTT 5. The topic map in `docs/mqtt-topics.md` does not depend on the protocol version, so this is an implementation choice only.

## Consequences

- The client is a transport adapter plus `minimq`. TLS and mutual auth (Phase 5) need a `no_std` TLS crate, which must be audited against the policy before it is added.
- The edge client is not wired into `tpt-agripod-edge` yet. Phase 3 does that.
- `minimq` is an embedded-first crate. Reconnect and offline buffering are the edge daemon's job, not the client's.
- Follow-up: check `minimq`'s transitive dependencies with `cargo deny` when it is added to the workspace.
